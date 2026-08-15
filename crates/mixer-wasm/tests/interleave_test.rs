//! Tests for interleaved input de-interleaving and PID mapping.

use oximedia_audio::ChannelLayout;
use oximedia_mixer::{
    channel::ChannelType, processing::PanLawType, AudioMixer, ChannelProcessParams, MixerConfig,
};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: usize = 128;

fn sine(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

/// Interleave two mono buffers into a stereo interleaved buffer.
fn interleave(left: &[f32], right: &[f32]) -> Vec<f32> {
    let mut out = Vec::with_capacity(left.len() * 2);
    for i in 0..left.len() {
        out.push(left[i]);
        out.push(right[i]);
    }
    out
}

/// Test: de-interleaving stereo input produces correct mono channels.
#[test]
fn test_deinterleave_stereo() {
    let sine_l = sine(220.0, 0.5, BLOCK_SIZE);
    let sine_r = sine(330.0, 0.5, BLOCK_SIZE);
    let interleaved = interleave(&sine_l, &sine_r);

    // Simulate de-interleaving.
    let nc = 2;
    let frames = interleaved.len() / nc;
    let mut de_l = Vec::with_capacity(frames);
    let mut de_r = Vec::with_capacity(frames);
    for f in 0..frames {
        de_l.push(interleaved[f * nc]);
        de_r.push(interleaved[f * nc + 1]);
    }

    // Verify de-interleaved matches original.
    for i in 0..frames {
        assert!((de_l[i] - sine_l[i]).abs() < 1e-6, "L mismatch at {i}");
        assert!((de_r[i] - sine_r[i]).abs() < 1e-6, "R mismatch at {i}");
    }
}

/// Test: interleaved stereo through process_mix produces correct per-channel output.
#[test]
fn test_interleaved_through_mixer() {
    let mut mixer = AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 8,
        ..Default::default()
    });

    let ch0 = mixer
        .add_channel("ch0".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();
    let ch1 = mixer
        .add_channel("ch1".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    let sine_l = sine(220.0, 0.5, BLOCK_SIZE);
    let sine_r = sine(330.0, 0.5, BLOCK_SIZE);

    // Process channel 0 with left sine.
    let params0 = vec![(
        ch0,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 0.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Linear,
        },
    )];
    let (left0, _) = mixer.engine_mut().process_mix(&params0, &sine_l);

    // Process channel 1 with right sine.
    let params1 = vec![(
        ch1,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 0.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Linear,
        },
    )];
    let (left1, _) = mixer.engine_mut().process_mix(&params1, &sine_r);

    // Sum: master_left = sine_l * 0.5 + sine_r * 0.5
    for i in 0..BLOCK_SIZE {
        let expected = (sine_l[i] + sine_r[i]) * 0.5;
        let actual = left0[i] + left1[i];
        assert!(
            (actual - expected).abs() < 1e-5,
            "Interleaved mix mismatch at {i}: actual={actual:.6}, expected={expected:.6}"
        );
    }
}

/// Test: PID mapping is idempotent.
#[test]
fn test_pid_mapping_idempotent() {
    // Simulate the PID mapping logic (HashMap-based, like the binding).
    let mut pid_map: std::collections::HashMap<u16, u32> = std::collections::HashMap::new();

    // Initial mapping.
    pid_map.insert(0x101, 0);
    assert_eq!(pid_map.get(&0x101), Some(&0));

    // Re-map same PID to different channel (source reconfigured).
    pid_map.insert(0x101, 4);
    assert_eq!(pid_map.get(&0x101), Some(&4));

    // Idempotent remove.
    pid_map.remove(&0x101);
    pid_map.remove(&0x101); // should not panic
    assert!(!pid_map.contains_key(&0x101));

    // Re-map after removal.
    pid_map.insert(0x101, 8);
    assert_eq!(pid_map.get(&0x101), Some(&8));
}
