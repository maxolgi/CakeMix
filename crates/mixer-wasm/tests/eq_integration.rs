//! EQ integration test — verifies EQ works through the effects chain
//! in the real process_mix path.

use oximedia_audio::ChannelLayout;
use oximedia_mixer::{
    channel::ChannelType,
    effects_chain::AudioEffect,
    eq_band::{EqFilterType, ParametricEq},
    processing::{PanLawType, RuntimeEffectSlot},
    AudioMixer, ChannelProcessParams, MixerConfig,
};

use mixer_wasm::effects::EqEffect;

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: usize = 1024;

fn sine(freq: f64, gain: f64, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            (gain * (2.0 * std::f64::consts::PI * freq * i as f64 / SAMPLE_RATE as f64).sin())
                as f32
        })
        .collect()
}

fn rms(samples: &[f32]) -> f64 {
    if samples.len() <= 128 {
        return (samples
            .iter()
            .map(|s| (*s as f64) * (*s as f64))
            .sum::<f64>()
            / samples.len() as f64)
            .sqrt();
    }
    let usable = &samples[128..];
    (usable
        .iter()
        .map(|s| (*s as f64) * (*s as f64))
        .sum::<f64>()
        / usable.len() as f64)
        .sqrt()
}

/// Test: EQ effect in the channel effects chain boosts the target frequency.
#[test]
fn test_eq_in_processing_chain() {
    let mut mixer = AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 4,
        ..Default::default()
    });

    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    // Add a +12dB peaking EQ at 1kHz to the channel effects chain.
    let eq_effect = EqEffect::peaking(SAMPLE_RATE, 1000.0, 12.0, 1.0);
    mixer
        .add_channel_effect(ch, RuntimeEffectSlot::new(Box::new(eq_effect)))
        .unwrap();

    // Process 1kHz sine (should be amplified by EQ).
    let input_1k = sine(1000.0, 0.1, BLOCK_SIZE);
    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 0.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Minus6dB,
        },
    )];

    let (left_1k, _right_1k) = mixer.engine_mut().process_mix(&params, &input_1k);
    let rms_1k = rms(&left_1k);

    // Process 100Hz sine (should be relatively unaffected by the 1kHz EQ).
    let input_100 = sine(100.0, 0.1, BLOCK_SIZE);
    let (left_100, _right_100) = mixer.engine_mut().process_mix(&params, &input_100);
    let rms_100 = rms(&left_100);

    // The 1kHz signal should be significantly louder due to the EQ boost.
    let ratio = rms_1k / rms_100.max(1e-10);
    assert!(
        ratio > 2.0,
        "EQ in chain FAIL: 1kHz/100Hz RMS ratio = {ratio:.3} (expected >2.0). rms_1k={rms_1k:.6}, rms_100={rms_100:.6}"
    );
}

/// Test: EQ adapter implements AudioEffect correctly.
#[test]
fn test_eq_adapter_audio_effect() {
    let mut effect = EqEffect::peaking(SAMPLE_RATE, 1000.0, 12.0, 1.0);

    // Process a buffer.
    let mut samples = sine(1000.0, 0.1, 512);
    let original_rms = rms(&samples);

    // Process through the effect.
    AudioEffect::process(&mut effect, &mut samples);

    let processed_rms = rms(&samples);

    // Should be amplified.
    assert!(
        processed_rms > original_rms * 2.0,
        "AudioEffect::process failed: original={original_rms:.6}, processed={processed_rms:.6}"
    );

    // Name should be correct.
    assert_eq!(effect.name(), "ParametricEQ");
}

/// Test: multiple EQ bands in the chain.
#[test]
fn test_eq_multiple_bands() {
    let mut eq = ParametricEq::new(SAMPLE_RATE, 1);
    // Boost at 1kHz and cut at 100Hz.
    eq.add_band("Peak1k".into(), EqFilterType::Peaking, 1000.0, 12.0, 1.0);
    eq.add_band("Cut100".into(), EqFilterType::Peaking, 100.0, -12.0, 1.0);

    let effect = EqEffect::from_eq(eq);
    assert_eq!(effect.inner().num_bands(), 2);
}
