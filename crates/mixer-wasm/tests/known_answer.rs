//! Known-answer test for the WASM mixer binding.
//!
//! Synthesizes two sine waves, mixes them, and verifies the output matches
//! a directly-computed reference within 1e-5 tolerance.
//!
//! Run with: wasm-pack test --node --release

use js_sys::Float32Array;
use wasm_bindgen_test::*;

/// Sample rate for all tests.
const SAMPLE_RATE: u32 = 48_000;
/// Block size (Web Audio render quantum).
const BLOCK_SIZE: u32 = 128;

/// Helper: generate `n` samples of a sine wave at `freq` Hz and `gain` linear.
fn sine_wave(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

#[wasm_bindgen_test]
fn test_basic_sum_two_sines() {
    // Synthesize two sines: 220Hz @ 0.5 gain and 330Hz @ 0.5 gain.
    let sine_a = sine_wave(220.0, 0.5, BLOCK_SIZE as usize);
    let sine_b = sine_wave(330.0, 0.5, BLOCK_SIZE as usize);

    // Direct reference including Linear pan law gain (0.5 at center):
    // out[n] = (0.5*sin_a + 0.5*sin_b) × 0.5
    let pan_gain = 0.5_f32; // Linear pan law at center
    let reference: Vec<f32> = (0..BLOCK_SIZE as usize)
        .map(|n| {
            (0.5 * (2.0 * std::f32::consts::PI * 220.0 * n as f32 / SAMPLE_RATE as f32).sin()
                + 0.5 * (2.0 * std::f32::consts::PI * 330.0 * n as f32 / SAMPLE_RATE as f32).sin())
                * pan_gain
        })
        .collect();

    // Build and configure the mixer.
    let mut mixer =
        mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 4).expect("constructor should succeed");

    // Set per-channel inputs via Float32Array interop.
    let fa = Float32Array::new_with_length(BLOCK_SIZE);
    fa.copy_from(&sine_a);
    mixer.set_channel_input(0, &fa).expect("set ch0 input");

    let fb = Float32Array::new_with_length(BLOCK_SIZE);
    fb.copy_from(&sine_b);
    mixer.set_channel_input(1, &fb).expect("set ch1 input");

    // Process.
    let output = mixer.process(BLOCK_SIZE).expect("process should succeed");
    assert_eq!(
        output.length(),
        BLOCK_SIZE * 2,
        "output should be interleaved stereo of length block_size * 2"
    );

    // Copy output back to Rust.
    let mut out_buf = vec![0.0f32; output.length() as usize];
    output.copy_to(&mut out_buf);

    // Verify: at unity gain, center pan, Linear pan law,
    // each sample L = R = sum of both channel inputs.
    for i in 0..BLOCK_SIZE as usize {
        let left = out_buf[i * 2];
        let right = out_buf[i * 2 + 1];

        // Left and right should be equal (center pan, mono input → both channels).
        assert!(
            (left - right).abs() < 1e-6,
            "L/R mismatch at sample {i}: L={left}, R={right}"
        );

        // Check against reference.
        assert!(
            (left - reference[i]).abs() < 1e-5,
            "sample {i}: actual={left}, reference={}, diff={}",
            reference[i],
            (left - reference[i]).abs()
        );
    }
}

#[wasm_bindgen_test]
fn test_not_silence() {
    // Honesty gate: output must not be silence or all-zeros.
    let sine_a = sine_wave(440.0, 1.0, BLOCK_SIZE as usize);
    let mut mixer =
        mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).expect("constructor should succeed");

    let fa = Float32Array::new_with_length(BLOCK_SIZE);
    fa.copy_from(&sine_a);
    mixer.set_channel_input(0, &fa).expect("set input");

    let output = mixer.process(BLOCK_SIZE).expect("process");
    let mut out_buf = vec![0.0f32; output.length() as usize];
    output.copy_to(&mut out_buf);

    // Must not be all zeros.
    let max_sample = out_buf.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(
        max_sample > 0.01,
        "HONESTY GATE: output is near-silence (max={max_sample}),          suspect fake-stub or wiring error"
    );
}

#[wasm_bindgen_test]
fn test_both_channels_present() {
    // Honesty gate: both channels must contribute to output.
    // Feed ch0 with a signal, ch1 with silence → output should match ch0 alone.
    // Then feed ch1 with signal too → output should increase.
    let sine_a = sine_wave(220.0, 0.5, BLOCK_SIZE as usize);
    let zeros = vec![0.0f32; BLOCK_SIZE as usize];

    let mut mixer =
        mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 4).expect("constructor should succeed");

    let fa = Float32Array::new_with_length(BLOCK_SIZE);
    fa.copy_from(&sine_a);
    mixer.set_channel_input(0, &fa).expect("set ch0");

    let fz = Float32Array::new_with_length(BLOCK_SIZE);
    fz.copy_from(&zeros);
    mixer.set_channel_input(1, &fz).expect("set ch1");

    let out_one = mixer.process(BLOCK_SIZE).expect("process");
    let mut buf_one = vec![0.0f32; out_one.length() as usize];
    out_one.copy_to(&mut buf_one);

    // Now feed ch1 with the same signal.
    mixer
        .set_channel_input(1, &fa)
        .expect("set ch1 with signal");
    let out_two = mixer.process(BLOCK_SIZE).expect("process");
    let mut buf_two = vec![0.0f32; out_two.length() as usize];
    out_two.copy_to(&mut buf_two);

    // With both channels at same gain, output should roughly double.
    let max_one = buf_one.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    let max_two = buf_two.iter().fold(0.0f32, |a, &b| a.max(b.abs()));

    assert!(
        max_two > max_one * 1.5,
        "HONESTY GATE: second channel not summed properly.          one_channel_max={max_one}, two_channel_max={max_two}"
    );
}

#[wasm_bindgen_test]
fn test_mute_channel() {
    // Muting a channel should zero its contribution.
    let sine_a = sine_wave(440.0, 0.5, BLOCK_SIZE as usize);

    let mut mixer =
        mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).expect("constructor should succeed");

    let fa = Float32Array::new_with_length(BLOCK_SIZE);
    fa.copy_from(&sine_a);
    mixer.set_channel_input(0, &fa).expect("set input");
    mixer.set_channel_mute(0, true).expect("mute");

    let output = mixer.process(BLOCK_SIZE).expect("process");
    let mut out_buf = vec![0.0f32; output.length() as usize];
    output.copy_to(&mut out_buf);

    let max_sample = out_buf.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(
        max_sample < 1e-6,
        "Muted channel should produce silence, got max={max_sample}"
    );
}

#[wasm_bindgen_test]
fn test_gain_control() {
    // Setting gain to 0.5 should halve the output.
    let sine_a = sine_wave(440.0, 1.0, BLOCK_SIZE as usize);

    let mut mixer =
        mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).expect("constructor should succeed");

    let fa = Float32Array::new_with_length(BLOCK_SIZE);
    fa.copy_from(&sine_a);
    mixer.set_channel_input(0, &fa).expect("set input");

    // Unity gain.
    let out_unity = mixer.process(BLOCK_SIZE).expect("process");
    let mut buf_unity = vec![0.0f32; out_unity.length() as usize];
    out_unity.copy_to(&mut buf_unity);

    // Half gain.
    mixer.set_channel_gain(0, 0.5).expect("set gain");
    let out_half = mixer.process(BLOCK_SIZE).expect("process");
    let mut buf_half = vec![0.0f32; out_half.length() as usize];
    out_half.copy_to(&mut buf_half);

    // Check that half-gain output is approximately 0.5× unity output.
    for i in 0..BLOCK_SIZE as usize {
        let unity = buf_unity[i * 2];
        let half = buf_half[i * 2];
        if unity.abs() > 1e-4 {
            let ratio = half / unity;
            assert!(
                (ratio - 0.5).abs() < 0.01,
                "Gain ratio at sample {i}: expected ~0.5, got {ratio} (unity={unity}, half={half})"
            );
        }
    }
}
