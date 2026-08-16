//! Channel direct-out tap honesty tests.
//!
//! Verifies the `set_channel_tap` / `take_channel_tap` contract end-to-end
//! through MixerWasm: mono per channel, captured post input gain, post
//! gate/comp/EQ, post fader, PRE pan; muted/solo-gated channels silent;
//! missing inputs zero-filled; master output unchanged.
//!
//! Run with: wasm-pack test --node --release
//! (MixerWasm uses js-sys interop, so — like known_answer.rs — these
//! execute under the wasm runtime, not native cargo test.)

use js_sys::Float32Array;
use wasm_bindgen_test::*;

use mixer_wasm::MixerWasm;

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: u32 = 128;

/// Build a mixer with limiter off (exact known answers) and all four
/// channel EQs bypassed (flat biquads still ring — bypass for DC tests).
fn make_mixer(max_channels: u32) -> MixerWasm {
    let mut mixer =
        MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, max_channels).expect("constructor should succeed");
    mixer.set_limiter_enabled(false);
    for ch in 0..max_channels {
        mixer.set_eq_bypass(ch, true).expect("bypass eq");
    }
    mixer
}

/// Feed a mono buffer to a channel via Float32Array interop.
fn feed(mixer: &mut MixerWasm, ch: u32, samples: &[f32]) {
    let fa = Float32Array::new_with_length(samples.len() as u32);
    fa.copy_from(samples);
    mixer
        .set_channel_input(ch, &fa)
        .unwrap_or_else(|e| panic!("set ch{ch} input: {e:?}"));
}

/// Drain the tap into a plain Vec (take_channel_tap → Rust).
fn drain_tap(mixer: &mut MixerWasm) -> Vec<f32> {
    let tap = mixer.take_channel_tap();
    let mut buf = vec![0.0f32; tap.length() as usize];
    tap.copy_to(&mut buf);
    buf
}

fn dc(level: f32, n: usize) -> Vec<f32> {
    vec![level; n]
}

fn sine(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// 1. Unity known-answer: 4 channels, distinct DC levels, all gains 1.0 →
/// one interleaved [ch0, ch1, ch2, ch3] group per frame, bs frames.
#[wasm_bindgen_test]
fn test_tap_unity_known_answer() {
    let mut mixer = make_mixer(4);
    mixer.set_channel_tap(4);

    let levels = [0.1f32, 0.2, 0.3, 0.4];
    for (ch, &level) in levels.iter().enumerate() {
        feed(&mut mixer, ch as u32, &dc(level, BLOCK_SIZE as usize));
    }

    mixer.process(BLOCK_SIZE).expect("process");
    let tap = drain_tap(&mut mixer);

    assert_eq!(
        tap.len(),
        BLOCK_SIZE as usize * 4,
        "tap length must be bs × N"
    );
    for i in 0..BLOCK_SIZE as usize {
        for (c, &level) in levels.iter().enumerate() {
            let got = tap[i * 4 + c];
            assert!(
                (got - level).abs() < 1e-7,
                "frame {i} ch{c}: got {got}, expected {level}"
            );
        }
    }

    // take() drains: a second take with no intervening process is empty.
    assert_eq!(drain_tap(&mut mixer).len(), 0, "tap must drain on take");
}

/// 2. Fader applied: ch0 fader gain 0.5 → tap ch0 = DC × 0.5.
#[wasm_bindgen_test]
fn test_tap_fader_gain_applied() {
    let mut mixer = make_mixer(4);
    mixer.set_channel_tap(4);
    mixer.set_channel_gain(0, 0.5).expect("set fader");
    feed(&mut mixer, 0, &dc(0.1, BLOCK_SIZE as usize));

    mixer.process(BLOCK_SIZE).expect("process");
    let tap = drain_tap(&mut mixer);

    for i in 0..BLOCK_SIZE as usize {
        assert!(
            (tap[i * 4] - 0.05).abs() < 1e-7,
            "frame {i}: fader not applied, got {}",
            tap[i * 4]
        );
    }
}

/// 3. Input gain applied: −6.0206 dB (exactly ½ linear) on ch0 → halved.
#[wasm_bindgen_test]
fn test_tap_input_gain_applied() {
    let mut mixer = make_mixer(4);
    mixer.set_channel_tap(4);
    mixer
        .set_channel_input_gain(0, -6.0206)
        .expect("set input gain");
    feed(&mut mixer, 0, &dc(0.1, BLOCK_SIZE as usize));

    mixer.process(BLOCK_SIZE).expect("process");
    let tap = drain_tap(&mut mixer);

    for i in 0..BLOCK_SIZE as usize {
        assert!(
            (tap[i * 4] - 0.05).abs() < 1e-4,
            "frame {i}: input gain not applied, got {}",
            tap[i * 4]
        );
    }
}

/// 4. Mute: muted channel taps silence; others unaffected.
#[wasm_bindgen_test]
fn test_tap_muted_channel_silent() {
    let mut mixer = make_mixer(4);
    mixer.set_channel_tap(4);
    mixer.set_channel_mute(2, true).expect("mute ch2");

    let levels = [0.1f32, 0.2, 0.3, 0.4];
    for (ch, &level) in levels.iter().enumerate() {
        feed(&mut mixer, ch as u32, &dc(level, BLOCK_SIZE as usize));
    }

    mixer.process(BLOCK_SIZE).expect("process");
    let tap = drain_tap(&mut mixer);

    for i in 0..BLOCK_SIZE as usize {
        for (c, &level) in levels.iter().enumerate() {
            let got = tap[i * 4 + c];
            let expected = if c == 2 { 0.0 } else { level };
            assert!(
                (got - expected).abs() < 1e-7,
                "frame {i} ch{c}: got {got}, expected {expected}"
            );
        }
    }
}

/// 5. Disabled by default: take() is empty before set_channel_tap, and
/// after disabling again with set_channel_tap(0).
#[wasm_bindgen_test]
fn test_tap_disabled_by_default() {
    let mut mixer = make_mixer(2);
    feed(&mut mixer, 0, &dc(0.5, BLOCK_SIZE as usize));
    mixer.process(BLOCK_SIZE).expect("process");

    assert_eq!(
        drain_tap(&mut mixer).len(),
        0,
        "tap must be empty before set_channel_tap"
    );

    // Enable → block appears → disable → empty again.
    mixer.set_channel_tap(2);
    mixer.process(BLOCK_SIZE).expect("process");
    assert_eq!(drain_tap(&mut mixer).len(), BLOCK_SIZE as usize * 2);
    mixer.set_channel_tap(0);
    mixer.process(BLOCK_SIZE).expect("process");
    assert_eq!(
        drain_tap(&mut mixer).len(),
        0,
        "tap must be empty after set_channel_tap(0)"
    );
}

/// 6. Zero-fill for missing input: 4ch tap, only ch0/ch1 fed → ch2/ch3
/// regions zeros (silence, not garbage or stale data).
#[wasm_bindgen_test]
fn test_tap_missing_input_zero_filled() {
    let mut mixer = make_mixer(4);
    mixer.set_channel_tap(4);
    feed(&mut mixer, 0, &dc(0.1, BLOCK_SIZE as usize));
    feed(&mut mixer, 1, &dc(0.2, BLOCK_SIZE as usize));

    mixer.process(BLOCK_SIZE).expect("process");
    let tap = drain_tap(&mut mixer);

    assert_eq!(tap.len(), BLOCK_SIZE as usize * 4);
    for i in 0..BLOCK_SIZE as usize {
        assert!((tap[i * 4] - 0.1).abs() < 1e-7, "frame {i} ch0");
        assert!((tap[i * 4 + 1] - 0.2).abs() < 1e-7, "frame {i} ch1");
        assert!(
            tap[i * 4 + 2].abs() < 1e-9,
            "frame {i} ch2 must be silent, got {}",
            tap[i * 4 + 2]
        );
        assert!(
            tap[i * 4 + 3].abs() < 1e-9,
            "frame {i} ch3 must be silent, got {}",
            tap[i * 4 + 3]
        );
    }
}

/// 7. Solo gating matches the mix: solo ch1 → every non-soloed channel is
/// silent in the tap, soloed channel passes.
#[wasm_bindgen_test]
fn test_tap_solo_gating_matches_mix() {
    let mut mixer = make_mixer(4);
    mixer.set_channel_tap(4);
    mixer.set_channel_solo(1, true).expect("solo ch1");

    let levels = [0.1f32, 0.2, 0.3, 0.4];
    for (ch, &level) in levels.iter().enumerate() {
        feed(&mut mixer, ch as u32, &dc(level, BLOCK_SIZE as usize));
    }

    mixer.process(BLOCK_SIZE).expect("process");
    let tap = drain_tap(&mut mixer);

    for i in 0..BLOCK_SIZE as usize {
        for (c, &level) in levels.iter().enumerate() {
            let got = tap[i * 4 + c];
            let expected = if c == 1 { level } else { 0.0 };
            assert!(
                (got - expected).abs() < 1e-7,
                "frame {i} ch{c}: got {got}, expected {expected}"
            );
        }
    }
}

/// 8. EQ in the path: +6 dB peaking band at 1.5 kHz on ch0 boosts a
/// 1.5 kHz sine in the tap (same signal patterns as eq_honesty.rs).
#[wasm_bindgen_test]
fn test_tap_eq_in_path() {
    let input = sine(1500.0, 0.25, BLOCK_SIZE as usize);

    // Reference: EQ bypassed.
    let mut mixer_ref = make_mixer(1);
    mixer_ref.set_channel_tap(1);
    feed(&mut mixer_ref, 0, &input);

    // Boosted: six-band EQ active, band 3 = 1.5 kHz peaking (Q 1.0), +6 dB.
    let mut mixer_eq = make_mixer(1);
    mixer_eq.set_eq_bypass(0, false).expect("enable eq");
    mixer_eq.set_eq_band_gain(0, 3, 6.0).expect("boost band 3");
    mixer_eq.set_channel_tap(1);
    feed(&mut mixer_eq, 0, &input);

    // Let the biquads settle, then compare steady-state RMS.
    for _ in 0..8 {
        mixer_ref.process(BLOCK_SIZE).expect("process ref");
        mixer_eq.process(BLOCK_SIZE).expect("process eq");
    }
    let tap_ref = drain_tap(&mut mixer_ref);
    let tap_eq = drain_tap(&mut mixer_eq);

    let ratio = rms(&tap_eq[64..]) / rms(&tap_ref[64..]).max(1e-10);
    assert!(
        ratio > 1.5,
        "EQ boost not reflected in tap: ratio {ratio:.3} (expected >1.5; +6dB ≈ 2.0)"
    );
}

/// 9. Master unaffected: process() output is byte-identical before vs
/// after enabling the tap on the SAME mixer instance. (Two instances can
/// differ by an ULP regardless of the tap: `channel_inputs` is a HashMap,
/// so per-channel accumulation to master happens in per-instance random
/// iteration order — same-instance comparison isolates exactly the tap's
/// code path.)
#[wasm_bindgen_test]
fn test_tap_master_output_unaffected() {
    let mut mixer = make_mixer(4);
    let levels = [0.1f32, 0.2, 0.3, 0.4];
    let refill = |m: &mut MixerWasm| {
        for (ch, &level) in levels.iter().enumerate() {
            feed(m, ch as u32, &dc(level, BLOCK_SIZE as usize));
        }
    };
    refill(&mut mixer);

    let collect = |m: &mut MixerWasm| -> Vec<u32> {
        refill(m); // FIFO inputs: re-feed each block (worklet re-feeds)
        let out = m.process(BLOCK_SIZE).expect("process");
        let mut buf = vec![0.0f32; out.length() as usize];
        out.copy_to(&mut buf);
        buf.iter().map(|s| s.to_bits()).collect()
    };

    // Two blocks with the tap off, then two with it on: state evolution
    // (EQ/dynamics/meters) is untouched by the tap path.
    let off1 = collect(&mut mixer);
    let off2 = collect(&mut mixer);

    mixer.set_channel_tap(4);
    let on1 = collect(&mut mixer);
    let on2 = collect(&mut mixer);

    assert_eq!(on1, off1, "master block 1 differs with tap enabled");
    assert_eq!(on2, off2, "master block 2 differs with tap enabled");
}

/// 10. Clamping and resizing: set_channel_tap clamps to 128; re-enabling
/// with a different N resizes the returned block.
#[wasm_bindgen_test]
fn test_tap_clamp_and_resize() {
    let mut mixer = make_mixer(4);

    // Clamp: 999 → 128 channels (bs × 128 floats).
    mixer.set_channel_tap(999);
    feed(&mut mixer, 0, &dc(0.1, BLOCK_SIZE as usize));
    mixer.process(BLOCK_SIZE).expect("process");
    assert_eq!(drain_tap(&mut mixer).len(), BLOCK_SIZE as usize * 128);

    // Resize: 128 → 2 channels.
    mixer.set_channel_tap(2);
    feed(&mut mixer, 0, &dc(0.1, BLOCK_SIZE as usize)); // FIFO: re-feed
    mixer.process(BLOCK_SIZE).expect("process");
    let tap = drain_tap(&mut mixer);
    assert_eq!(tap.len(), BLOCK_SIZE as usize * 2);
    for i in 0..BLOCK_SIZE as usize {
        assert!((tap[i * 2] - 0.1).abs() < 1e-7, "frame {i} ch0");
        assert!(tap[i * 2 + 1].abs() < 1e-9, "frame {i} ch1 must be silent");
    }
}
