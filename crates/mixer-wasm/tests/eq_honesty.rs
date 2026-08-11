//! EQ honesty test — verifies that ParametricEq performs real frequency processing.
//!
//! Tests that eq_band::ParametricEq:
//! - Actually boosts signals at the center frequency (peaking filter)
//! - Actually cuts signals at the center frequency
//! - Leaves far-from-center frequencies relatively unchanged

use oximedia_mixer::eq_band::{EqFilterType, ParametricEq};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: usize = 4096;

fn sine(freq: f64, gain: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| gain * (2.0 * std::f64::consts::PI * freq * i as f64 / SAMPLE_RATE as f64).sin())
        .collect()
}

/// Measure the RMS amplitude of a signal (skipping first 256 samples for transient).
fn rms(samples: &[f64]) -> f64 {
    if samples.len() <= 256 {
        return samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64;
    }
    let usable = &samples[256..];
    (usable.iter().map(|s| s * s).sum::<f64>() / usable.len() as f64).sqrt()
}

/// Test: a +12dB peaking filter at 1kHz should amplify 1kHz signal significantly
/// more than a 100Hz signal.
#[test]
fn test_eq_boost_at_center_freq() {
    // Create EQ with a peaking boost at 1kHz, +12dB, Q=1.0
    let mut eq = ParametricEq::new(SAMPLE_RATE, 1);
    eq.add_band("Peak".into(), EqFilterType::Peaking, 1000.0, 12.0, 1.0);

    // Process a 1kHz sine through the EQ.
    let mut signal_1k = sine(1000.0, 0.1, BLOCK_SIZE);
    eq.process_buffer(&mut signal_1k, 1);

    // Process a 100Hz sine through the EQ (should be relatively unaffected).
    let mut signal_100 = sine(100.0, 0.1, BLOCK_SIZE);
    eq.process_buffer(&mut signal_100, 1);

    // Process a 10kHz sine through the EQ (should also be relatively unaffected).
    let mut signal_10k = sine(10000.0, 0.1, BLOCK_SIZE);
    eq.process_buffer(&mut signal_10k, 1);

    let rms_1k = rms(&signal_1k);
    let rms_100 = rms(&signal_100);
    let rms_10k = rms(&signal_10k);

    // The 1kHz signal should be significantly louder than 100Hz and 10kHz.
    // +12dB ≈ 4x in linear, so RMS should be ~4x higher.
    // We check that the ratio is at least 2x (conservative).
    let ratio_100 = rms_1k / rms_100.max(1e-10);
    let ratio_10k = rms_1k / rms_10k.max(1e-10);

    assert!(
        ratio_100 > 2.0,
        "EQ boost FAIL: 1kHz/100Hz RMS ratio = {ratio_100:.3} (expected >2.0). rms_1k={rms_1k:.6}, rms_100={rms_100:.6}"
    );
    assert!(
        ratio_10k > 2.0,
        "EQ boost FAIL: 1kHz/10kHz RMS ratio = {ratio_10k:.3} (expected >2.0). rms_1k={rms_1k:.6}, rms_10k={rms_10k:.6}"
    );
}

/// Test: a -12dB peaking filter at 1kHz should attenuate 1kHz signal.
#[test]
fn test_eq_cut_at_center_freq() {
    let mut eq = ParametricEq::new(SAMPLE_RATE, 1);
    eq.add_band("Cut".into(), EqFilterType::Peaking, 1000.0, -12.0, 1.0);

    let mut signal_1k = sine(1000.0, 0.1, BLOCK_SIZE);
    eq.process_buffer(&mut signal_1k, 1);

    let mut signal_100 = sine(100.0, 0.1, BLOCK_SIZE);
    eq.process_buffer(&mut signal_100, 1);

    let rms_1k = rms(&signal_1k);
    let rms_100 = rms(&signal_100);

    // The 1kHz signal should be significantly quieter than 100Hz.
    // -12dB ≈ 0.25x, so 1kHz should be about 1/4 of 100Hz.
    let ratio = rms_100 / rms_1k.max(1e-10);
    assert!(
        ratio > 2.0,
        "EQ cut FAIL: 100Hz/1kHz RMS ratio = {ratio:.3} (expected >2.0). rms_1k={rms_1k:.6}, rms_100={rms_100:.6}"
    );
}

/// Test: flat EQ (0dB gain on all bands) should pass signal unchanged.
#[test]
fn test_eq_flat_passthrough() {
    let mut eq = ParametricEq::new(SAMPLE_RATE, 1);
    eq.add_band("Flat".into(), EqFilterType::Peaking, 1000.0, 0.0, 1.0);

    let original = sine(1000.0, 0.5, BLOCK_SIZE);
    let mut processed = original.clone();
    eq.process_buffer(&mut processed, 1);

    // With 0dB gain, output should match input (within biquad numerical precision).
    for i in 256..BLOCK_SIZE {
        assert!(
            (processed[i] - original[i]).abs() < 1e-4,
            "Flat EQ changed signal at [{i}]: orig={:.6}, proc={:.6}",
            original[i],
            processed[i]
        );
    }
}

/// Test: high-pass filter should attenuate low frequencies.
#[test]
fn test_eq_high_pass() {
    let mut eq = ParametricEq::new(SAMPLE_RATE, 1);
    eq.add_band("HP".into(), EqFilterType::HighPass, 2000.0, 0.0, 0.707);

    // 100Hz should be significantly attenuated by a 2kHz high-pass.
    let mut signal_low = sine(100.0, 0.1, BLOCK_SIZE);
    eq.process_buffer(&mut signal_low, 1);

    // 10kHz should pass through mostly unchanged.
    let mut signal_high = sine(10000.0, 0.1, BLOCK_SIZE);
    eq.process_buffer(&mut signal_high, 1);

    let rms_low = rms(&signal_low);
    let rms_high = rms(&signal_high);

    let ratio = rms_high / rms_low.max(1e-10);
    assert!(
        ratio > 3.0,
        "HP filter FAIL: high/low RMS ratio = {ratio:.3} (expected >3.0). rms_high={rms_high:.6}, rms_low={rms_low:.6}"
    );
}

/// Test: bypass should pass signal completely unchanged.
#[test]
fn test_eq_bypass() {
    let mut eq = ParametricEq::new(SAMPLE_RATE, 1);
    eq.add_band("Boost".into(), EqFilterType::Peaking, 1000.0, 20.0, 1.0);
    eq.bypass = true;

    let original = sine(1000.0, 0.5, BLOCK_SIZE);
    let mut processed = original.clone();
    eq.process_buffer(&mut processed, 1);

    for i in 0..BLOCK_SIZE {
        assert!(
            (processed[i] - original[i]).abs() < 1e-10,
            "Bypassed EQ changed signal at [{i}]"
        );
    }
}
