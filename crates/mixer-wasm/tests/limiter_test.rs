//! Limiter test — verifies the master bus limiter prevents overs.

use oximedia_audio::ChannelLayout;
use oximedia_mixer::{
    channel::ChannelType, processing::PanLawType, AudioMixer, ChannelProcessParams, MixerConfig,
};

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: usize = 128;
/// Master-bus limiter latency: 1 ms lookahead delay line (48 samples @ 48 kHz)
/// plus one sample from the 4x oversample/decimate path.
const LOOKAHEAD: usize = 49;

fn sine(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

/// Test: limiter prevents signal from exceeding ceiling.
#[test]
fn test_limiter_prevents_overs() {
    let mut mixer = AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 4,
        ..Default::default()
    });

    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    // Enable the limiter (default: -0.3 dBFS ceiling).
    mixer.set_limiter_enabled(true);

    // Feed a loud signal (full-scale sine, multiple channels summing to >1.0).
    let loud_input = sine(440.0, 1.0, BLOCK_SIZE);
    let params = vec![(
        ch,
        ChannelProcessParams {
            fader_gain: 2.0, // Boost to 2x
            pan: 0.0,
            muted: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLawType::Linear, // center = 0.5 per side → up to 1.0 total
        },
    )];

    // Process multiple blocks to let the limiter settle.
    let mut max_output = 0.0f32;
    for _ in 0..10 {
        let (left, right) = mixer.engine_mut().process_mix(&params, &loud_input);
        // Apply limiter manually (since process() does this, but process_mix doesn't).
        let mut limited_l = vec![0.0f32; BLOCK_SIZE];
        let mut limited_r = vec![0.0f32; BLOCK_SIZE];
        mixer
            .master_limiter_l_mut()
            .process_block(&left, &mut limited_l);
        mixer
            .master_limiter_r_mut()
            .process_block(&right, &mut limited_r);

        for &s in &limited_l {
            max_output = max_output.max(s.abs());
        }
        for &s in &limited_r {
            max_output = max_output.max(s.abs());
        }
    }

    // The limiter ceiling is -0.3 dBFS ≈ 0.966 in linear.
    // Output should not exceed this significantly.
    let ceiling = 10.0_f32.powf(-0.3 / 20.0); // ≈ 0.966
    assert!(
        max_output <= ceiling * 1.01, // 1% tolerance for limiter transient
        "Limiter failed to prevent overs: max_output={max_output:.4}, ceiling={ceiling:.4}"
    );
}

/// Test: limiter passes quiet signals unchanged.
#[test]
fn test_limiter_passthrough_quiet() {
    let mut mixer = AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 4,
        ..Default::default()
    });

    let ch = mixer
        .add_channel("ch".into(), ChannelType::Mono, ChannelLayout::Mono)
        .unwrap();

    mixer.set_limiter_enabled(true);

    let quiet_input = sine(440.0, 0.1, BLOCK_SIZE);
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

    let (left, _right) = mixer.engine_mut().process_mix(&params, &quiet_input);

    // The master limiter (oversampled lookahead) delays the signal by
    // LOOKAHEAD samples. Process the block twice: the first pass fills the
    // delay line, the second is steady state.
    let mut limited_l = vec![0.0f32; BLOCK_SIZE];
    mixer
        .master_limiter_l_mut()
        .process_block(&left, &mut limited_l);
    mixer
        .master_limiter_l_mut()
        .process_block(&left, &mut limited_l);

    // Quiet signal: gain envelope stays at unity — the steady-state output
    // equals the input rotated by the lookahead delay (the same block is fed
    // twice, so rotation models the delay-line wrap).
    let mut expected = left.clone();
    expected.rotate_right(LOOKAHEAD);
    for i in 0..BLOCK_SIZE {
        assert!(
            (limited_l[i] - expected[i]).abs() < 1e-5,
            "Limiter altered quiet signal at [{i}]: out={:.5}, expected={:.5}",
            limited_l[i],
            expected[i]
        );
    }
}

/// Test: limiter gain reduction is reported.
#[test]
fn test_limiter_gain_reduction() {
    let mut mixer = AudioMixer::new(MixerConfig {
        sample_rate: SAMPLE_RATE,
        buffer_size: BLOCK_SIZE,
        max_channels: 4,
        ..Default::default()
    });

    mixer.set_limiter_enabled(true);

    // Feed a full-scale signal.
    let loud_input = sine(440.0, 2.0, BLOCK_SIZE);
    let mut limited = vec![0.0f32; BLOCK_SIZE];
    mixer
        .master_limiter_l_mut()
        .process_block(&loud_input, &mut limited);

    // The limiter reports the current gain multiplier in dB.
    // gain_reduction_db() returns linear_to_db(gain_env) where gain_env < 1.0
    // when limiting is active. So negative dB = gain reduction happening.
    let gr = mixer.master_limiter_l().gain_reduction_db();
    assert!(
        gr < -0.1,
        "Expected gain reduction from limiter (negative dB), got {gr} dB"
    );
}
