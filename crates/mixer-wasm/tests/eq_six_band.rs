//! 6-band EQ honesty tests.

use oximedia_mixer::eq_band::{EqFilterType, ParametricEq};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: usize = 4096;

fn sine(freq: f64, gain: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| gain * (2.0 * std::f64::consts::PI * freq * i as f64 / SAMPLE_RATE as f64).sin())
        .collect()
}

fn rms(samples: &[f64]) -> f64 {
    if samples.len() <= 256 {
        return samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64;
    }
    let usable = &samples[256..];
    (usable.iter().map(|s| s * s).sum::<f64>() / usable.len() as f64).sqrt()
}

#[test]
fn six_band_has_six_bands() {
    let mut eq = ParametricEq::new(SAMPLE_RATE, 1);
    eq.add_band("HPF".into(), EqFilterType::HighPass, 80.0, 0.0, 0.707);
    eq.add_band("Low".into(), EqFilterType::LowShelf, 120.0, 0.0, 0.707);
    eq.add_band("Lo-Mid".into(), EqFilterType::Peaking, 400.0, 0.0, 1.0);
    eq.add_band("Mid".into(), EqFilterType::Peaking, 1500.0, 0.0, 1.0);
    eq.add_band("Hi-Mid".into(), EqFilterType::Peaking, 5000.0, 0.0, 1.0);
    eq.add_band("High".into(), EqFilterType::HighShelf, 10000.0, 0.0, 0.707);

    assert_eq!(eq.num_bands(), 6);
    assert_eq!(eq.bands[0].name, "HPF");
    assert_eq!(eq.bands[5].name, "High");
}

#[test]
fn six_band_flat_is_near_unity() {
    let mut eq = ParametricEq::new(SAMPLE_RATE, 1);
    eq.add_band("HPF".into(), EqFilterType::HighPass, 80.0, 0.0, 0.707);
    eq.add_band("Low".into(), EqFilterType::LowShelf, 120.0, 0.0, 0.707);
    eq.add_band("Lo-Mid".into(), EqFilterType::Peaking, 400.0, 0.0, 1.0);
    eq.add_band("Mid".into(), EqFilterType::Peaking, 1500.0, 0.0, 1.0);
    eq.add_band("Hi-Mid".into(), EqFilterType::Peaking, 5000.0, 0.0, 1.0);
    eq.add_band("High".into(), EqFilterType::HighShelf, 10000.0, 0.0, 0.707);

    // Flat EQ should not significantly change signal at 1 kHz
    let mut signal = sine(1000.0, 0.1, BLOCK_SIZE);
    let input_rms = rms(&signal);
    eq.process_buffer(&mut signal, 1);
    let output_rms = rms(&signal);

    let change_db = 20.0 * (output_rms / input_rms).log10();
    assert!(
        change_db.abs() < 1.0,
        "Flat 6-band EQ should be near unity at 1 kHz, got {change_db:.1} dB"
    );
}

#[test]
fn six_band_low_shelf_boosts_bass() {
    let mut eq = ParametricEq::new(SAMPLE_RATE, 1);
    eq.add_band("Low".into(), EqFilterType::LowShelf, 120.0, 12.0, 0.707);

    let mut bass = sine(100.0, 0.1, BLOCK_SIZE);
    let bass_in = rms(&bass);
    eq.process_buffer(&mut bass, 1);
    let bass_out = rms(&bass);

    let boost_db = 20.0 * (bass_out / bass_in).log10();
    assert!(
        boost_db > 6.0,
        "Low shelf +12 dB should boost 100 Hz by >6 dB, got {boost_db:.1}"
    );
}

#[test]
fn six_band_mid_peak_cuts_mids() {
    let mut eq = ParametricEq::new(SAMPLE_RATE, 1);
    eq.add_band("Mid".into(), EqFilterType::Peaking, 1500.0, -12.0, 1.0);

    let mut mid = sine(1500.0, 0.1, BLOCK_SIZE);
    let mid_in = rms(&mid);
    eq.process_buffer(&mut mid, 1);
    let mid_out = rms(&mid);

    let cut_db = 20.0 * (mid_out / mid_in).log10();
    assert!(
        cut_db < -6.0,
        "Mid peak -12 dB should cut 1.5 kHz by >6 dB, got {cut_db:.1}"
    );
}
