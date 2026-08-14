//! Channel-strip parameter tests for the WASM mixer binding.
//!
//! Exercises the Phase 1 channel-parameter API (input gain, phase, pan law,
//! dynamics params, metering, master gain, limiter ceiling) through the
//! `MixerWasm` JS-interop layer. Same harness pattern as `known_answer.rs`.
//!
//! Run with: wasm-pack test --node --release

use js_sys::Float32Array;
use wasm_bindgen_test::*;

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: u32 = 128;

/// Generate `n` samples of a sine wave at `freq` Hz and `gain` linear.
fn sine(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

/// Feed a buffer into channel `ch` and process one block, returning the
/// interleaved stereo output as a Rust Vec.
fn run_block(mixer: &mut mixer_wasm::MixerWasm, ch: u32, input: &[f32]) -> Vec<f32> {
    let fa = Float32Array::new_with_length(input.len() as u32);
    fa.copy_from(input);
    mixer.set_channel_input(ch, &fa).expect("set input");
    let out = mixer.process(BLOCK_SIZE).expect("process");
    let mut buf = vec![0.0f32; out.length() as usize];
    out.copy_to(&mut buf);
    buf
}

fn max_abs(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |a, &b| a.max(b.abs()))
}

fn rms(buf: &[f32]) -> f64 {
    let s: f64 = buf.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (s / buf.len() as f64).sqrt()
}

/// Test 1: input gain +6 dB ≈ 2× output.
#[wasm_bindgen_test]
fn test_set_channel_input_gain() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);

    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();

    // 0 dB input gain
    mixer.set_channel_input_gain(0, 0.0).unwrap();
    let out_unity = run_block(&mut mixer, 0, &input);
    let max_unity = max_abs(&out_unity);

    // +6 dB input gain ≈ 2× linear
    mixer.set_channel_input_gain(0, 6.0206).unwrap();
    let out_boost = run_block(&mut mixer, 0, &input);
    let max_boost = max_abs(&out_boost);

    let ratio = max_boost / max_unity.max(1e-10);
    assert!(
        (ratio - 2.0).abs() < 0.05,
        "+6 dB should ~2× output, got ratio {ratio:.4}"
    );
}

/// Test 2: phase inversion negates the output.
#[wasm_bindgen_test]
fn test_set_channel_phase() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);

    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();

    mixer.set_channel_phase(0, false).unwrap();
    let out_normal = run_block(&mut mixer, 0, &input);

    mixer.set_channel_phase(0, true).unwrap();
    let out_inverted = run_block(&mut mixer, 0, &input);

    for i in 0..out_normal.len() {
        let sum = out_normal[i] + out_inverted[i];
        assert!(
            sum.abs() < 1e-4,
            "phase-inverted output should negate normal: sample {i} sum={sum}"
        );
    }
}

/// Test 3: Minus3dB pan law gives equal-power center (√2/2 ≈ 0.7071).
#[wasm_bindgen_test]
fn test_set_channel_pan_law() {
    let input = sine(440.0, 1.0, BLOCK_SIZE as usize);

    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();
    mixer.set_channel_pan_law(0, 1).unwrap(); // Minus3dB

    let out = run_block(&mut mixer, 0, &input);

    let expected = std::f32::consts::FRAC_1_SQRT_2;
    for i in 0..BLOCK_SIZE as usize {
        // left == right at center pan
        let left = out[i * 2];
        assert!(
            (left - input[i] * expected).abs() < 1e-3,
            "Minus3dB center: L[{}] = {}, expected {}",
            i,
            left,
            input[i] * expected
        );
    }
}

/// Test 4: compressor threshold 0 dB (no compression) vs -40 dB (heavy).
///
/// NOTE: the engine's `Compressor` poisons its envelope follower on signal
/// zero-crossings (`linear_to_db(0) = -inf` → envelope → -inf → no
/// compression), so we drive it with a DC signal to exercise the gain-computer
/// logic. `update_config` also reconstructs the Compressor (envelope reset to
/// -120 dB), so we warm up before measuring.
#[wasm_bindgen_test]
fn test_set_comp_param() {
    let input = vec![0.5f32; BLOCK_SIZE as usize]; // DC, no zero-crossings

    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();
    mixer.enable_compressor(0).unwrap();
    // zero out makeup so the comparison is clean threshold-only
    mixer.set_comp_param(0, 4, 0.0).unwrap(); // makeup_gain_db = 0

    // Threshold 0 dB: DC at -6 dB is below → no compression. Warm up + measure.
    mixer.set_comp_param(0, 0, 0.0).unwrap(); // threshold_db = 0
    for _ in 0..30 {
        let _ = run_block(&mut mixer, 0, &input);
    }
    let out_open = run_block(&mut mixer, 0, &input);
    let rms_open = rms(&out_open);

    // Threshold -40 dB: heavy compression. Warm up + measure.
    mixer.set_comp_param(0, 0, -40.0).unwrap(); // threshold_db = -40
    for _ in 0..30 {
        let _ = run_block(&mut mixer, 0, &input);
    }
    let out_comp = run_block(&mut mixer, 0, &input);
    let rms_comp = rms(&out_comp);

    assert!(
        rms_comp < rms_open * 0.5,
        "heavy compression should reduce RMS by >50%: open={rms_open:.5} comp={rms_comp:.5}"
    );
}

/// Test 5: gate at -10 dB blocks a -30 dB signal (near-silence).
#[wasm_bindgen_test]
fn test_set_gate_param() {
    // -30 dB ≈ 0.0316 amplitude
    let quiet = sine(440.0, 0.0316, BLOCK_SIZE as usize);

    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();
    mixer.enable_gate(0).unwrap();
    mixer.set_gate_param(0, 0, -10.0).unwrap(); // threshold_db = -10

    let out = run_block(&mut mixer, 0, &quiet);
    let peak = max_abs(&out);

    assert!(
        peak < 0.005,
        "gate should block -30 dB signal below -10 dB threshold, got peak {peak:.6}"
    );
}

/// Test 6: per-channel metering reads ~-6 dB peak / ~-9 dB RMS for a 0.5 sine.
#[wasm_bindgen_test]
fn test_channel_peak_rms_db() {
    // 0.5 amplitude sine → peak 0.5 (-6.02 dB), rms 0.5/√2 (-9.03 dB)
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);

    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();

    let _ = run_block(&mut mixer, 0, &input);

    let peak_db = mixer.channel_peak_db(0);
    let rms_db = mixer.channel_rms_db(0);

    assert!(
        (peak_db - (-6.02)).abs() < 0.5,
        "peak_db for 0.5 sine ≈ -6 dB, got {peak_db:.3}"
    );
    assert!(
        (rms_db - (-9.03)).abs() < 0.5,
        "rms_db for 0.5 sine ≈ -9 dB, got {rms_db:.3}"
    );
}

/// Test 7: master gain 0.5 halves the output.
#[wasm_bindgen_test]
fn test_set_master_gain() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);

    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();

    mixer.set_master_gain(1.0);
    let out_unity = run_block(&mut mixer, 0, &input);
    let max_unity = max_abs(&out_unity);

    mixer.set_master_gain(0.5);
    let out_half = run_block(&mut mixer, 0, &input);
    let max_half = max_abs(&out_half);

    let ratio = max_half / max_unity.max(1e-10);
    assert!(
        (ratio - 0.5).abs() < 0.02,
        "master gain 0.5 should halve output, got ratio {ratio:.4}"
    );
}

/// Test 8: limiter ceiling -12 dB caps the output.
#[wasm_bindgen_test]
fn test_set_limiter_ceiling() {
    let input = sine(440.0, 1.0, BLOCK_SIZE as usize);

    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_eq_bypass(0, true).unwrap();
    mixer.set_limiter_ceiling(-12.0);
    // -12 dBFS ceiling ≈ 0.251 linear; allow a small overshoot margin.
    let ceiling_linear = 10.0f32.powf(-12.0 / 20.0);

    let mut max_peak = 0.0f32;
    for _ in 0..20 {
        let out = run_block(&mut mixer, 0, &input);
        max_peak = max_peak.max(max_abs(&out));
    }

    assert!(
        max_peak <= ceiling_linear * 1.05,
        "limiter ceiling -12 dB ({ceiling_linear:.4}) exceeded: max_peak {max_peak:.4}"
    );
}

/// Test 9: channel_meters_json emits a valid 2-entry array.
#[wasm_bindgen_test]
fn test_channel_meters_json() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);

    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 4).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();
    mixer.set_eq_bypass(1, true).unwrap();

    let _ = run_block(&mut mixer, 0, &input);
    let _ = run_block(&mut mixer, 1, &input);

    let json = mixer.channel_meters_json();

    assert!(
        json.starts_with('[') && json.ends_with(']'),
        "json must be an array: {json}"
    );
    assert!(
        json.contains("\"ch\":0"),
        "json must include channel 0: {json}"
    );
    assert!(
        json.contains("\"ch\":1"),
        "json must include channel 1: {json}"
    );
    // Exactly two entries (two "ch": occurrences).
    let count = json.matches("\"ch\":").count();
    assert_eq!(count, 2, "expected 2 meter entries, got {count}: {json}");
}
