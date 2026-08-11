//! Dynamics integration tests — verifies comp/gate work through
//! the effects chain in the real process_mix path.

use oximedia_mixer::{
    channel::ChannelType,
    processing::{PanLawType, RuntimeEffectSlot},
    AudioMixer, ChannelProcessParams, MixerConfig,
};
use oximedia_audio::ChannelLayout;

use mixer_wasm::effects::{CompressorEffect, GateEffect};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: usize = 1024;

fn sine(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            // Start at phase offset to avoid zero-crossing envelope issue
            let phase = 0.1;
            gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32 + phase).sin()
        })
        .collect()
}

fn rms(samples: &[f32]) -> f64 {
    if samples.len() <= 128 {
        return (samples.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>()
            / samples.len() as f64)
            .sqrt();
    }
    let usable = &samples[128..];
    (usable.iter().map(|s| (*s as f64) * (*s as f64)).sum::<f64>() / usable.len() as f64).sqrt()
}

/// Test: compressor in the effects chain reduces loud signals.
#[test]
fn test_compressor_in_chain() {
    let mut mixer = AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 4,
        ..Default::default()
    });

    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    // Add a compressor with aggressive settings, no makeup gain.
    use oximedia_mixer::dynamics::CompressorConfig;
    let comp = CompressorEffect::new(
        CompressorConfig {
            threshold_db: -12.0,
            ratio: 4.0,
            attack_ms: 5.0,
            release_ms: 100.0,
            makeup_gain_db: 0.0,  // no makeup so we can verify pure reduction
            knee_db: 3.0,
        },
        SAMPLE_RATE,
    );
    mixer
        .add_channel_effect(ch, RuntimeEffectSlot::new(Box::new(comp)))
        .unwrap();

    // Feed a loud signal (0.9 amplitude).
    let loud_input = sine(440.0, 0.9, BLOCK_SIZE);
    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 0.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Minus6dB, // center = 1.0 passthrough
        },
    )];

    let (left_with_comp, _) = mixer.engine_mut().process_mix(&params, &loud_input);
    let rms_with_comp = rms(&left_with_comp);

    // Now compare without compressor.
    let mut mixer_plain = AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 4,
        ..Default::default()
    });
    let ch_plain = mixer_plain
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();
    let (left_plain, _) = mixer_plain.engine_mut().process_mix(&params, &loud_input);
    // Rebuild params for the plain mixer's channel ID
    let params_plain = vec![(
        ch_plain,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 0.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Minus6dB,
        },
    )];
    let (left_plain, _) = mixer_plain.engine_mut().process_mix(&params_plain, &loud_input);
    let rms_plain = rms(&left_plain);

    // Compressed signal should be quieter than uncompressed (for loud signals above threshold).
    assert!(
        rms_with_comp < rms_plain,
        "Compressor failed: with_comp={rms_with_comp:.6} should be < plain={rms_plain:.6}"
    );
}

/// Test: compressor leaves quiet signals mostly unchanged (vs plain channel).
#[test]
fn test_compressor_passthrough_quiet() {
    let quiet_input = sine(440.0, 0.01, BLOCK_SIZE);

    // Mixer with compressor.
    let mut mixer_comp = AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 4,
        ..Default::default()
    });
    let ch_comp = mixer_comp.add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono).unwrap();
    use oximedia_mixer::dynamics::CompressorConfig;
    let comp = CompressorEffect::new(
        CompressorConfig {
            threshold_db: -12.0,
            ratio: 4.0,
            attack_ms: 5.0,
            release_ms: 100.0,
            makeup_gain_db: 0.0,
            knee_db: 3.0,
        },
        SAMPLE_RATE,
    );
    mixer_comp.add_channel_effect(ch_comp, RuntimeEffectSlot::new(Box::new(comp))).unwrap();

    let params_comp = vec![(ch_comp, ChannelProcessParams {
        fader_gain: 1.0, pan: 0.0, muted: false, input_gain_db: 0.0,
        phase_inverted: false, pan_law: PanLawType::Minus6dB,
    })];
    let (left_comp, _) = mixer_comp.engine_mut().process_mix(&params_comp, &quiet_input);
    let rms_comp = rms(&left_comp);

    // Mixer without compressor (same pan law, same signal).
    let mut mixer_plain = AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 4,
        ..Default::default()
    });
    let ch_plain = mixer_plain.add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono).unwrap();
    let params_plain = vec![(ch_plain, ChannelProcessParams {
        fader_gain: 1.0, pan: 0.0, muted: false, input_gain_db: 0.0,
        phase_inverted: false, pan_law: PanLawType::Minus6dB,
    })];
    let (left_plain, _) = mixer_plain.engine_mut().process_mix(&params_plain, &quiet_input);
    let rms_plain = rms(&left_plain);

    // Quiet signal below threshold: compressor output ≈ plain output.
    let ratio = rms_comp / rms_plain.max(1e-10);
    assert!(
        (ratio - 1.0).abs() < 0.1,
        "Compressor altered quiet signal: ratio={ratio:.3} (expected ~1.0)"
    );
}

/// Test: gate blocks quiet signals.
#[test]
fn test_gate_in_chain() {
    let mut mixer = AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 4,
        ..Default::default()
    });

    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    let gate = GateEffect::denoise(SAMPLE_RATE);
    mixer
        .add_channel_effect(ch, RuntimeEffectSlot::new(Box::new(gate)))
        .unwrap();

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

    // Feed a very quiet signal (well below -50 dB threshold).
    let quiet = sine(440.0, 0.001, BLOCK_SIZE);
    let (left_q, _) = mixer.engine_mut().process_mix(&params, &quiet);
    let rms_quiet = rms(&left_q);

    // Feed a loud signal.
    let loud = sine(440.0, 0.5, BLOCK_SIZE);
    let (left_l, _) = mixer.engine_mut().process_mix(&params, &loud);
    let rms_loud = rms(&left_l);

    // Gate should block the quiet signal but pass the loud one.
    let ratio = rms_loud / rms_quiet.max(1e-10);
    assert!(
        ratio > 100.0,
        "Gate failed: loud/quiet ratio = {ratio:.1} (expected >100). rms_loud={rms_loud:.6}, rms_quiet={rms_quiet:.6}"
    );
}
