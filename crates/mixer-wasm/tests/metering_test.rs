//! Metering tests — verifies master output metering works.

use oximedia_mixer::metering::{Meter, MeterBallistics};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: usize = 4096;

fn sine(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

/// Interleave two mono buffers.
fn interleave(l: &[f32], r: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(l.len() * 2);
    for i in 0..l.len() {
        out.push(l[i]);
        out.push(r[i]);
    }
    out
}

/// Test: meter reads a loud signal at high dB.
#[test]
fn test_meter_reads_loud_signal() {
    let mut meter = Meter::new(2, SAMPLE_RATE, MeterBallistics::Fast);

    let loud = sine(440.0, 0.9, BLOCK_SIZE);
    let interleaved = interleave(&loud, &loud);
    meter.process(&interleaved);

    let peak_l = meter.data().peak[0].current_db;
    let peak_r = meter.data().peak[1].current_db;

    // 0.9 linear ≈ 0.915 dB.
    assert!(
        peak_l > -1.5,
        "Peak L too low: {peak_l:.2} dB (expected > -1.5)"
    );
    assert!(
        (peak_l - peak_r).abs() < 0.1,
        "L/R peak mismatch: L={peak_l:.2}, R={peak_r:.2}"
    );
}

/// Test: meter reads a quiet signal at low dB.
#[test]
fn test_meter_reads_quiet_signal() {
    let mut meter = Meter::new(2, SAMPLE_RATE, MeterBallistics::Fast);

    let quiet = sine(440.0, 0.001, BLOCK_SIZE);
    let interleaved = interleave(&quiet, &quiet);
    meter.process(&interleaved);

    let peak_l = meter.data().peak[0].current_db;

    // 0.001 linear ≈ -60 dB.
    assert!(
        peak_l < -55.0,
        "Peak L too high for quiet signal: {peak_l:.2} dB (expected < -55)"
    );
}

/// Test: clipping is detected.
#[test]
fn test_meter_detects_clipping() {
    let mut meter = Meter::new(2, SAMPLE_RATE, MeterBallistics::Fast);

    // Full-scale + overshoot.
    let clipping = sine(440.0, 1.5, BLOCK_SIZE);
    let interleaved = interleave(&clipping, &clipping);
    meter.process(&interleaved);

    assert!(
        meter.data().peak[0].clipped,
        "Clipping not detected on L channel"
    );
    assert!(
        meter.data().peak[1].clipped,
        "Clipping not detected on R channel"
    );
}

/// Test: silence reads as very low dB.
#[test]
fn test_meter_silence() {
    let mut meter = Meter::new(2, SAMPLE_RATE, MeterBallistics::Fast);

    let silence = vec![0.0f32; BLOCK_SIZE * 2];
    meter.process(&silence);

    let peak = meter.data().peak[0].current_db;
    assert!(peak < -100.0, "Silence peak too high: {peak:.2} dB");
}

/// Test: RMS is lower than peak for a sine wave.
#[test]
fn test_meter_rms_below_peak() {
    let mut meter = Meter::new(2, SAMPLE_RATE, MeterBallistics::Fast);

    let signal = sine(440.0, 0.5, BLOCK_SIZE);
    let interleaved = interleave(&signal, &signal);

    // Process multiple blocks to let RMS integration settle.
    for _ in 0..10 {
        meter.process(&interleaved);
    }

    let peak = meter.data().peak[0].current_db;
    let rms = meter.data().rms[0].current_db;

    // For a sine wave, RMS is ~3 dB below peak (crest factor).
    assert!(
        rms < peak,
        "RMS ({rms:.2}) should be below peak ({peak:.2})"
    );
    let diff = peak - rms;
    assert!(
        diff > 1.0 && diff < 6.0,
        "Peak-RMS diff = {diff:.2} dB (expected 1-6 dB for sine wave)"
    );
}
