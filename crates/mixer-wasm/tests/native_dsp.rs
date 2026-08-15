//! Native Rust tests for the oximedia-mixer DSP.
//!
//! These run with `cargo test` on the host (no browser/wasm needed).
//! They test the engine directly, bypassing the wasm binding.
//! The wasm binding tests (tests/known_answer.rs) verify the same
//! DSP through the JS interop layer.

use oximedia_audio::ChannelLayout;
use oximedia_mixer::{
    channel::{ChannelType, PanLaw},
    processing::PanLawType,
    AudioMixer, ChannelProcessParams, MixerConfig,
};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: usize = 128;

fn sine(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

fn make_mixer() -> AudioMixer {
    AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 8,
        ..Default::default()
    })
}

/// Test: basic mix of two sines, each fed to its own channel.
///
/// This mirrors the wasm binding's per-channel input approach: each channel
/// gets its OWN audio via a separate process_mix call, then the results are
/// summed. This is necessary because process_mix feeds the SAME input_samples
/// to every channel in the list.
#[test]
fn test_basic_sum_two_sines() {
    let mut mixer = make_mixer();
    let ch0 = mixer
        .add_channel("ch0".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();
    let ch1 = mixer
        .add_channel("ch1".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    // Set Linear pan law and center pan.
    {
        let c = mixer.get_channel_mut(ch0).unwrap();
        c.set_pan_law(PanLaw::Linear);
        c.set_pan(0.0);
        c.set_gain(1.0);
    }
    {
        let c = mixer.get_channel_mut(ch1).unwrap();
        c.set_pan_law(PanLaw::Linear);
        c.set_pan(0.0);
        c.set_gain(1.0);
    }

    let sine_a = sine(220.0, 0.5, BLOCK_SIZE);
    let sine_b = sine(330.0, 0.5, BLOCK_SIZE);

    // Process each channel separately (per-channel input, like the binding).
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
    let (left0, right0) = mixer.engine_mut().process_mix(&params0, &sine_a);

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
    let (left1, right1) = mixer.engine_mut().process_mix(&params1, &sine_b);

    // Sum per-channel outputs to master.
    let pan_gain = 0.5; // Linear center
    for i in 0..BLOCK_SIZE {
        let expected = (sine_a[i] + sine_b[i]) * pan_gain;
        let actual_l = left0[i] + left1[i];
        let actual_r = right0[i] + right1[i];

        assert!(
            (actual_l - expected).abs() < 1e-5,
            "L[{}] = {}, expected {}",
            i,
            actual_l,
            expected
        );
        assert!(
            (actual_r - expected).abs() < 1e-5,
            "R[{}] = {}, expected {}",
            i,
            actual_r,
            expected
        );
    }
}

/// Test: single channel, verify pan law math precisely.
#[test]
fn test_linear_pan_law() {
    let mut mixer = make_mixer();
    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    let input = sine(440.0, 1.0, BLOCK_SIZE);

    // Pan fully left.
    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: -1.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Linear,
        },
    )];

    let (left, right) = mixer.engine_mut().process_mix(&params, &input);

    // Linear pan: pan_norm = (-1+1)*0.5 = 0.0
    // left = 1 - 0 = 1.0, right = 0.0
    for i in 0..BLOCK_SIZE {
        assert!((left[i] - input[i]).abs() < 1e-5, "L[{}] = {}", i, left[i]);
        assert!(right[i].abs() < 1e-5, "R[{}] = {}", i, right[i]);
    }

    // Pan fully right.
    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 1.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Linear,
        },
    )];

    let (left, right) = mixer.engine_mut().process_mix(&params, &input);

    // pan_norm = (1+1)*0.5 = 1.0
    // left = 0.0, right = 1.0
    for i in 0..BLOCK_SIZE {
        assert!(left[i].abs() < 1e-5, "L[{}] = {}", i, left[i]);
        assert!(
            (right[i] - input[i]).abs() < 1e-5,
            "R[{}] = {}",
            i,
            right[i]
        );
    }
}

/// Test: -3dB equal power pan law at center.
#[test]
fn test_minus3db_pan_law() {
    let mut mixer = make_mixer();
    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    let input = sine(440.0, 1.0, BLOCK_SIZE);

    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 0.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Minus3dB,
        },
    )];

    let (left, right) = mixer.engine_mut().process_mix(&params, &input);

    // Equal power at center: pan_norm = 0.5
    // left = cos(0.5 * π/2) = cos(π/4) = √2/2 ≈ 0.7071
    // right = sin(0.5 * π/2) = sin(π/4) = √2/2 ≈ 0.7071
    let expected_gain = std::f32::consts::FRAC_1_SQRT_2;
    for i in 0..BLOCK_SIZE {
        let expected = input[i] * expected_gain;
        assert!(
            (left[i] - expected).abs() < 1e-5,
            "L[{}] = {}, expected {}",
            i,
            left[i],
            expected
        );
        assert!(
            (right[i] - expected).abs() < 1e-5,
            "R[{}] = {}, expected {}",
            i,
            right[i],
            expected
        );
    }
}

/// Test: muted channel produces silence.
#[test]
fn test_muted_channel() {
    let mut mixer = make_mixer();
    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    let input = sine(440.0, 1.0, BLOCK_SIZE);

    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 0.0,
            muted: true,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Linear,
        },
    )];

    let (left, right) = mixer.engine_mut().process_mix(&params, &input);

    for i in 0..BLOCK_SIZE {
        assert!(left[i].abs() < 1e-6, "L[{}] = {}", i, left[i]);
        assert!(right[i].abs() < 1e-6, "R[{}] = {}", i, right[i]);
    }
}

/// Test: input gain in dB.
#[test]
fn test_input_gain_db() {
    let mut mixer = make_mixer();
    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    let input = sine(440.0, 1.0, BLOCK_SIZE);

    // +6 dB ≈ 2x gain
    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 0.0,
            muted: false,
            input_gain_db: 6.0206, // exactly 2x in linear
            phase_inverted: false,
            pan_law: PanLawType::Linear,
        },
    )];

    let (left, _right) = mixer.engine_mut().process_mix(&params, &input);

    // Linear pan center: 0.5 × input_gain_linear(2.0) = 1.0
    for i in 0..BLOCK_SIZE {
        let expected = input[i] * 2.0 * 0.5; // input_gain × pan_gain
        assert!(
            (left[i] - expected).abs() < 1e-4,
            "L[{}] = {}, expected {}",
            i,
            left[i],
            expected
        );
    }
}

/// Test: phase inversion.
#[test]
fn test_phase_inversion() {
    let mut mixer = make_mixer();
    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    let input = sine(440.0, 1.0, BLOCK_SIZE);

    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 0.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: true,
            pan_law: PanLawType::Linear,
        },
    )];

    let (left, _right) = mixer.engine_mut().process_mix(&params, &input);

    // Phase inverted: output = -input × 0.5 (Linear center pan)
    for i in 0..BLOCK_SIZE {
        let expected = -input[i] * 0.5;
        assert!(
            (left[i] - expected).abs() < 1e-5,
            "L[{}] = {}, expected {}",
            i,
            left[i],
            expected
        );
    }
}

/// Test: fader gain scales output.
#[test]
fn test_fader_gain() {
    let mut mixer = make_mixer();
    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    let input = sine(440.0, 1.0, BLOCK_SIZE);

    // Fader at 0.5
    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 0.5,
            pan: 0.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Linear,
        },
    )];

    let (left, _right) = mixer.engine_mut().process_mix(&params, &input);

    // output = input × fader(0.5) × pan(0.5)
    for i in 0..BLOCK_SIZE {
        let expected = input[i] * 0.5 * 0.5;
        assert!(
            (left[i] - expected).abs() < 1e-5,
            "L[{}] = {}, expected {}",
            i,
            left[i],
            expected
        );
    }
}

/// Test: honesty gate — process_mix actually produces output (not silence).
#[test]
fn test_not_silence() {
    let mut mixer = make_mixer();
    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    let input = sine(1000.0, 1.0, BLOCK_SIZE);
    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 1.0,
            pan: 0.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Linear,
        },
    )];

    let (left, right) = mixer.engine_mut().process_mix(&params, &input);

    let max_sample = left
        .iter()
        .chain(right.iter())
        .fold(0.0f32, |a, &b| a.max(b.abs()));

    assert!(
        max_sample > 0.01,
        "HONESTY GATE: output is near-silence (max={max_sample})"
    );
}

/// Test: multiple channels sum independently.
#[test]
fn test_multiple_channel_sum() {
    let mut mixer = make_mixer();
    let ch0 = mixer
        .add_channel("ch0".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();
    let ch1 = mixer
        .add_channel("ch1".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    let sine_a = sine(220.0, 0.5, BLOCK_SIZE);
    let sine_b = sine(330.0, 0.5, BLOCK_SIZE);

    // Channel 0 with sine_a
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
    let (left0, _right0) = mixer.engine_mut().process_mix(&params0, &sine_a);

    // Both channels
    let params_both = vec![
        (
            ch0,
            ChannelProcessParams {
                fader_gain: 1.0,
                pan: 0.0,
                muted: false,
                input_gain_db: 0.0,
                phase_inverted: false,
                pan_law: PanLawType::Linear,
            },
        ),
        (
            ch1,
            ChannelProcessParams {
                fader_gain: 1.0,
                pan: 0.0,
                muted: false,
                input_gain_db: 0.0,
                phase_inverted: false,
                pan_law: PanLawType::Linear,
            },
        ),
    ];
    let combined_input: Vec<f32> = sine_a
        .iter()
        .zip(sine_b.iter())
        .map(|(a, b)| a + b)
        .collect();
    let (left_both, _right_both) = mixer
        .engine_mut()
        .process_mix(&params_both, &combined_input);

    // Both channels should be louder than one.
    let max0 = left0.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    let max_both = left_both.iter().fold(0.0f32, |a, &b| a.max(b.abs()));
    assert!(
        max_both > max0 * 1.5,
        "Multiple channels not summing: max0={max0}, max_both={max_both}"
    );
}
