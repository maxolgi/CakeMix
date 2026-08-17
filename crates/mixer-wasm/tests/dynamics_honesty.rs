//! Dynamics honesty tests for `oximedia_mixer::dynamics`.
//!
//! Per AGENTS.md "honesty rule", every DSP path from oximedia-mixer must pass a
//! known-answer test before being trusted. This module (`dynamics.rs`) is **not
//! on the real-time path** (ENGINE_API.md §3.3 lists it as dead-to-RT), so it
//! has never been exercised by the engine. These tests prove whether the DSP is
//! real or fake.
//!
//! # Verdict: the DSP is REAL, with one load-bearing bug
//!
//! `Compressor`, `Expander`, `Gate`, and `Limiter` all implement genuine
//! textbook dynamics math: real gain computers, real one-pole envelope followers
//! with attack/release ballistic smoothing, a real gate state machine with
//! hysteresis + hold, and a real gain-clamping limiter. The gain-reduction
//! formulas are exact for steady (DC) inputs — verified to 4+ significant
//! figures against closed-form expected values below.
//!
//! **HOWEVER** there is a critical robustness defect in `Compressor`, `Expander`,
//! and `Gate`: they call `linear_to_db(sample.abs())`, which returns
//! `-f32::INFINITY` when `sample.abs() == 0.0`. The one-pole envelope follower
//! then computes `coeff * (-inf) = -inf`, so once a single exact-zero sample is
//! seen the envelope is poisoned to `-inf` **forever** and the processor
//! silently stops working (no compression, never opens). A sine wave starting at
//! phase 0 hits `sin(0) = 0.0` on the very first sample and is therefore not
//! processed at all. The module's own unit tests pass only because they feed
//! constant (DC) inputs that never cross zero. The `Limiter` is immune (it works
//! in the linear domain).
//!
//! Additionally, `LimiterConfig.lookahead_ms` is a **dead field**: it is stored
//! but never read by `process_sample`, and there is no lookahead buffer. The
//! limiter is a zero-lookahead clipper-with-release.
//!
//! Tests below assert the ACTUAL behavior (including the documented bug), so the
//! suite passes while recording the defect. Fixing the bug (e.g. clamping
//! `sample.abs()` to a tiny floor before `linear_to_db`) will require updating
//! the zero-crossing tests to assert correct compression instead.

use oximedia_mixer::dynamics::{
    db_to_linear, linear_to_db, Compressor, CompressorConfig, Expander, ExpanderConfig, Gate,
    GateConfig, GateState, Limiter, LimiterConfig,
};

const SAMPLE_RATE: u32 = 48_000;
/// Number of samples to let a one-pole envelope settle to its steady state.
/// For a DC input this converges to the exact input level in dB.
const SETTLE: usize = 200_000;

// ===========================================================================
// Helpers
// ===========================================================================

fn sine_no_zero_cross(freq: f32, amp: f32, n: usize) -> Vec<f32> {
    // Phase offset chosen so no integer-indexed sample is exactly 0.0.
    // This avoids the linear_to_db(0.0) = -inf envelope-poisoning bug so we
    // can observe the compressor's actual AC behaviour.
    let phase = 0.1234_f32;
    (0..n)
        .map(|i| {
            amp * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32 + phase).sin()
        })
        .collect()
}

fn sine_from_zero(freq: f32, amp: f32, n: usize) -> Vec<f32> {
    // Starts at phase 0, so sample 0 is exactly 0.0 -> triggers the bug.
    (0..n)
        .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

// ===========================================================================
// Compressor: REAL DSP — gain computer verified against closed-form math (DC)
// ===========================================================================

/// Above threshold, hard knee: the steady-state gain reduction must equal the
/// textbook formula `gr = (input_db - threshold) * (1 - 1/ratio)`.
#[test]
fn test_compressor_gain_reduction_exact_above_threshold() {
    let cfg = CompressorConfig {
        threshold_db: -20.0,
        ratio: 4.0,
        attack_ms: 1.0,
        release_ms: 10.0,
        makeup_gain_db: 0.0,
        knee_db: 0.0, // hard knee
    };
    let mut comp = Compressor::new(cfg);

    let input = 0.5_f32; // -6.02 dB, well above the -20 dB threshold
    let input_db = linear_to_db(input);

    // Closed-form expected steady-state gain reduction and output.
    let expected_gr_db = (input_db - (-20.0_f32)) * (1.0 - 1.0 / 4.0);
    let expected_out = input * db_to_linear(-expected_gr_db);

    let mut out = 0.0_f32;
    for _ in 0..SETTLE {
        out = comp.process_sample(input, SAMPLE_RATE);
    }

    assert!(
        (out - expected_out).abs() < 1e-4,
        "GR mismatch: got out={out:.6}, expected {expected_out:.6} (gr={expected_gr_db:.4} dB)"
    );
}

/// Below threshold: no gain reduction (DC passthrough).
#[test]
fn test_compressor_no_reduction_below_threshold() {
    let cfg = CompressorConfig {
        threshold_db: -20.0,
        ratio: 4.0,
        attack_ms: 1.0,
        release_ms: 10.0,
        makeup_gain_db: 0.0,
        knee_db: 0.0,
    };
    let mut comp = Compressor::new(cfg);

    let input = 0.01_f32; // -40 dB, below -20 dB threshold
    let mut out = 0.0_f32;
    for _ in 0..SETTLE {
        out = comp.process_sample(input, SAMPLE_RATE);
    }

    assert!(
        (out - input).abs() < 1e-5,
        "Should pass through below threshold: got {out:.6}, expected {input}"
    );
}

/// The 4:1 ratio is exact: an input exactly 12 dB above threshold is reduced to
/// 3 dB above threshold (12 / 4 = 3), i.e. 9 dB of gain reduction.
#[test]
fn test_compressor_ratio_4to1_exact() {
    let cfg = CompressorConfig {
        threshold_db: -20.0,
        ratio: 4.0,
        attack_ms: 1.0,
        release_ms: 10.0,
        makeup_gain_db: 0.0,
        knee_db: 0.0,
    };
    let mut comp = Compressor::new(cfg);

    // 12 dB above threshold -> input_db = -8 dB -> input = 10^(-8/20)
    let input = db_to_linear(-8.0);
    let mut out = 0.0_f32;
    for _ in 0..SETTLE {
        out = comp.process_sample(input, SAMPLE_RATE);
    }

    let out_db = linear_to_db(out);
    // Output should be 3 dB above threshold: -17 dB. Gain reduction = 9 dB.
    assert!(
        (out_db - (-17.0)).abs() < 0.05,
        "4:1 ratio: input -8 dB (12 dB over), expected output -17 dB (3 dB over), got {out_db:.4} dB"
    );
}

/// Soft knee: within the knee band the gain reduction is the quadratic
/// interpolation, distinct from the hard-knee formula. This proves the soft-knee
/// branch is real and active.
#[test]
fn test_compressor_soft_knee_active() {
    let threshold = -20.0_f32;
    let knee = 6.0_f32;
    let ratio = 4.0_f32;

    let cfg = CompressorConfig {
        threshold_db: threshold,
        ratio,
        attack_ms: 1.0,
        release_ms: 10.0,
        makeup_gain_db: 0.0,
        knee_db: knee, // soft knee
    };
    let mut comp = Compressor::new(cfg);

    // -19 dB is inside the knee band [threshold - knee/2, threshold + knee/2] = [-23, -17].
    let input = db_to_linear(-19.0);
    let input_db = linear_to_db(input);

    let mut out = 0.0_f32;
    for _ in 0..SETTLE {
        out = comp.process_sample(input, SAMPLE_RATE);
    }

    // Closed-form soft-knee GR for a point inside the band:
    //   x = input_db - threshold + knee/2
    //   gr = (1 - 1/ratio) * x^2 / (2 * knee)
    let half_k = knee / 2.0;
    let x = input_db - threshold + half_k;
    let expected_gr = (1.0 - 1.0 / ratio) * x * x / (2.0 * knee);
    let expected_out = input * db_to_linear(-expected_gr);

    assert!(
        (out - expected_out).abs() < 1e-4,
        "Soft-knee mismatch: got {out:.6}, expected {expected_out:.6} (gr={expected_gr:.4} dB)"
    );
}

/// Make-up gain is applied multiplicatively after compression.
#[test]
fn test_compressor_makeup_gain_applied() {
    let cfg_no_makeup = CompressorConfig {
        threshold_db: -20.0,
        ratio: 4.0,
        attack_ms: 1.0,
        release_ms: 10.0,
        makeup_gain_db: 0.0,
        knee_db: 0.0,
    };
    let cfg_makeup = CompressorConfig {
        makeup_gain_db: 6.0, // +6 dB
        ..cfg_no_makeup.clone()
    };

    let input = 0.5_f32;
    let mut c0 = Compressor::new(cfg_no_makeup);
    let mut c6 = Compressor::new(cfg_makeup);
    let (mut out0, mut out6) = (0.0_f32, 0.0_f32);
    for _ in 0..SETTLE {
        out0 = c0.process_sample(input, SAMPLE_RATE);
        out6 = c6.process_sample(input, SAMPLE_RATE);
    }

    // +6 dB makeup ≈ 2x the no-makeup output.
    assert!(
        (out6 / out0 - 2.0).abs() < 0.01,
        "Makeup +6dB should ~2x output (within tolerance): out0={out0:.6}, out6={out6:.6}, ratio={:.4}",
        out6 / out0
    );
}

/// Sine without zero crossings IS compressed (output peak < input peak). This
/// proves the compressor performs real, continuous-time-style dynamics on AC
/// audio, as long as no exact-zero sample poisons the envelope.
#[test]
fn test_compressor_sine_compresses_when_no_zero_crossing() {
    let cfg = CompressorConfig {
        threshold_db: -20.0,
        ratio: 4.0,
        attack_ms: 0.1,
        release_ms: 1.0,
        makeup_gain_db: 0.0,
        knee_db: 0.0,
    };
    let mut comp = Compressor::new(cfg);

    let amp = 0.5_f32; // -6 dB peak, above -20 dB threshold
    let sig = sine_no_zero_cross(220.0, amp, 50_000);

    let mut out_peak = 0.0_f32;
    for &s in &sig {
        let o = comp.process_sample(s, SAMPLE_RATE);
        out_peak = out_peak.max(o.abs());
    }

    assert!(
        out_peak < amp * 0.95,
        "Sine should be compressed: out_peak={out_peak:.6}, input_peak={amp}"
    );
    assert!(
        out_peak > 0.0,
        "Compressor must not zero out the signal: out_peak={out_peak}"
    );
}

/// HONESTY FINDING (BUG): a sine starting at phase 0 hits `sin(0) == 0.0`, which
/// makes `linear_to_db(0.0) == -inf`, permanently poisoning the one-pole
/// envelope to `-inf`. The compressor then applies ZERO gain reduction for the
/// entire signal. This test asserts the defective behavior so the suite stays
/// green while documenting the bug.
#[test]
fn test_compressor_survives_zero_crossing() {
    let cfg = CompressorConfig {
        threshold_db: -20.0,
        ratio: 4.0,
        attack_ms: 0.1,
        release_ms: 1.0,
        makeup_gain_db: 0.0,
        knee_db: 0.0,
    };
    let mut comp = Compressor::new(cfg);

    let amp = 0.5_f32; // -6 dB peak, well above threshold
    let sig = sine_from_zero(220.0, amp, 50_000);

    let mut out_peak = 0.0_f32;
    for &s in &sig {
        let o = comp.process_sample(s, SAMPLE_RATE);
        out_peak = out_peak.max(o.abs());
    }

    // FIXED (fork 9717f878): linear_to_db floors at -200 dB, so zero
    // crossings no longer poison the envelope — a loud sine that starts at
    // sin(0)=0 IS compressed.
    assert!(
        out_peak < amp * 0.75,
        "zero-crossing poisoned the envelope again: out_peak={out_peak:.6} \
         (expected compression of the {amp} sine)"
    );
}

// ===========================================================================
// Expander: REAL DSP — downward expansion verified against closed-form math
// ===========================================================================

/// Below threshold, the expander attenuates by `(ratio - 1) * headroom` dB.
#[test]
fn test_expander_attenuates_below_threshold_exact() {
    let cfg = ExpanderConfig {
        threshold_db: -40.0,
        ratio: 2.0, // 1:2 downward
        attack_ms: 1.0,
        release_ms: 10.0,
    };
    let mut exp = Expander::new(cfg);

    // Input at -60 dB (20 dB below the -40 dB threshold).
    let input = db_to_linear(-60.0);
    let input_db = linear_to_db(input);

    let mut out = 0.0_f32;
    for _ in 0..SETTLE {
        out = exp.process_sample(input, SAMPLE_RATE);
    }

    // Expected: gain_db = (input_db - threshold) * (ratio - 1) = -20 dB.
    let expected_gain_db = (input_db - (-40.0_f32)) * (2.0 - 1.0);
    let expected_out = input * db_to_linear(expected_gain_db);

    assert!(
        (out - expected_out).abs() < 1e-5,
        "Expander mismatch: got {out:.8}, expected {expected_out:.8} (gain {expected_gain_db} dB)"
    );
}

/// Above threshold, the expander passes the signal through unchanged.
#[test]
fn test_expander_passthrough_above_threshold() {
    let cfg = ExpanderConfig {
        threshold_db: -40.0,
        ratio: 2.0,
        attack_ms: 1.0,
        release_ms: 10.0,
    };
    let mut exp = Expander::new(cfg);

    let input = db_to_linear(-6.0); // above -40 dB threshold
    let mut out = 0.0_f32;
    for _ in 0..SETTLE {
        out = exp.process_sample(input, SAMPLE_RATE);
    }

    assert!(
        (out - input).abs() < 1e-5,
        "Expander should pass through above threshold: got {out:.6}, expected {input}"
    );
}

// ===========================================================================
// Gate: REAL DSP — state machine, hysteresis, hold, smoothing (DC)
// ===========================================================================

/// A loud DC signal opens the gate and passes through.
#[test]
fn test_gate_opens_for_loud_signal() {
    let cfg = GateConfig {
        threshold_db: -40.0,
        hysteresis_db: 6.0,
        attack_ms: 0.1,
        release_ms: 10.0,
        hold_ms: 0.0,
    };
    let mut gate = Gate::new(cfg);

    let input = 0.5_f32; // -6 dB, above -40 dB threshold
    let mut out = 0.0_f32;
    for _ in 0..10_000 {
        out = gate.process_sample(input, SAMPLE_RATE);
    }

    assert_eq!(gate.state(), GateState::Open, "gate should be open");
    assert!(
        (out - input).abs() < 0.01,
        "open gate should pass signal: got {out:.6}, expected {input}"
    );
}

/// A quiet DC signal keeps the gate closed and blocks the signal.
#[test]
fn test_gate_stays_closed_for_quiet_signal() {
    let cfg = GateConfig {
        threshold_db: -40.0,
        hysteresis_db: 6.0,
        attack_ms: 0.1,
        release_ms: 10.0,
        hold_ms: 0.0,
    };
    let mut gate = Gate::new(cfg);

    let input = 0.000_1_f32; // -80 dB, below -40 dB threshold
    let mut out = 0.0_f32;
    for _ in 0..10_000 {
        out = gate.process_sample(input, SAMPLE_RATE);
    }

    assert_eq!(gate.state(), GateState::Closed, "gate should be closed");
    assert!(
        out.abs() < 0.01,
        "closed gate should attenuate: got {out:.8}"
    );
}

/// HONESTY FINDING (BUG): same zero-crossing poisoning as the compressor. A sine
/// from phase 0 yields `sin(0) == 0.0 -> -inf`, the gate envelope dies, and the
/// gate never opens despite a loud signal.
#[test]
fn test_gate_opens_despite_zero_crossing() {
    let cfg = GateConfig {
        threshold_db: -40.0,
        hysteresis_db: 6.0,
        attack_ms: 0.1,
        release_ms: 10.0,
        hold_ms: 0.0,
    };
    let mut gate = Gate::new(cfg);

    let amp = 0.5_f32; // -6 dB peak, above threshold
    let sig = sine_from_zero(220.0, amp, 50_000);

    for &s in &sig {
        let _ = gate.process_sample(s, SAMPLE_RATE);
    }

    // FIXED (fork 9717f878): the -200 dB floor keeps the envelope finite,
    // so the gate opens for the loud sine despite starting at sin(0)=0.
    assert_eq!(
        gate.state(),
        GateState::Open,
        "gate should open for a loud sine; envelope poisoned by zero-crossing again"
    );
}

// ===========================================================================
// Limiter: REAL (zero-lookahead clipper), but lookahead_ms is a dead field
// ===========================================================================

/// The limiter clamps output to the configured ceiling (instant attack).
#[test]
fn test_limiter_clamps_to_ceiling() {
    let cfg = LimiterConfig {
        ceiling_db: -6.0,
        lookahead_ms: 0.0,
        release_ms: 10.0,
    };
    let mut lim = Limiter::new(cfg);

    let ceiling = db_to_linear(-6.0);
    let input = 0.9_f32; // above ceiling
    let out = lim.process_sample(input, SAMPLE_RATE);

    assert!(
        out.abs() <= ceiling + 1e-5,
        "output {out:.6} must not exceed ceiling {ceiling:.6}"
    );
}

/// A quiet signal passes through the limiter unchanged.
#[test]
fn test_limiter_passes_quiet_signal() {
    let mut lim = Limiter::new(LimiterConfig::default()); // ceiling 0 dBFS
    let input = 0.01_f32;
    let out = lim.process_sample(input, SAMPLE_RATE);

    assert!(
        (out - input).abs() < 1e-5,
        "quiet signal should pass: got {out:.6}, expected {input}"
    );
}

/// HONESTY FINDING: `lookahead_ms` is stored but never used. Changing it has no
/// effect on the output — there is no lookahead buffer.
#[test]
fn test_limiter_lookahead_is_dead_field() {
    let input_seq = [0.5_f32, 0.9, -0.8, 0.3, -0.95, 0.2, 1.5, -1.1];

    let cfg_no_la = LimiterConfig {
        ceiling_db: -6.0,
        lookahead_ms: 0.0,
        release_ms: 50.0,
    };
    let cfg_big_la = LimiterConfig {
        ceiling_db: -6.0,
        lookahead_ms: 100.0, // would matter if lookahead were implemented
        release_ms: 50.0,
    };

    let mut lim_no = Limiter::new(cfg_no_la);
    let mut lim_big = Limiter::new(cfg_big_la);

    for &s in &input_seq {
        let a = lim_no.process_sample(s, SAMPLE_RATE);
        let b = lim_big.process_sample(s, SAMPLE_RATE);
        assert!(
            (a - b).abs() < 1e-6,
            "lookahead_ms has no effect but outputs differ: la=0 -> {a:.6}, la=100 -> {b:.6}"
        );
    }
}

// ===========================================================================
// Utility functions
// ===========================================================================

#[test]
fn test_db_conversions() {
    assert!((db_to_linear(0.0) - 1.0).abs() < 1e-6);
    assert!((db_to_linear(-6.0) - 0.501_187).abs() < 1e-4);
    assert!((linear_to_db(1.0)).abs() < 1e-5);
    // Round-trip.
    let orig = -12.345_f32;
    assert!((linear_to_db(db_to_linear(orig)) - orig).abs() < 1e-4);
    // linear_to_db floors at -200 dB (fork 9717f878): -inf would permanently
    // poison the one-pole envelopes on digital silence.
    assert_eq!(linear_to_db(0.0), -200.0);
    assert!(linear_to_db(1e-11) <= -200.0);
}
