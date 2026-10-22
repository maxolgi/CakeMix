//! Live-E2E scene-recall regression (native, no JS runtime).
//!
//! Mirrors the reported live-session repro: PID-mapped PCM channels, an
//! FX chain of the final wave's four effects (cabinet 76 + detune 80 +
//! tuner 81 + looper 82, enabled), save_scene, clear the slots via
//! fx_set_slot(-1), recall_scene_fade(500 ms) while PCM keeps flowing —
//! then asserts the control plane SURVIVED the recall: the chain is
//! restored, set_mute still gates the master, fx_set_slot still sticks,
//! save_scene still hands out fresh ids.
//!
//! Run: cargo test -p mixer-wasm --test scene_recall_repro_test

use mixer_wasm::MixerWasm;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;

const CABINET: i32 = 76;
const DETUNE: i32 = 80;
const TUNER: i32 = 81;
const LOOPER: i32 = 82;

fn mixer() -> MixerWasm {
    MixerWasm::new(SR, BLOCK, 256).expect("ctor")
}

fn sine(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SR as f32).sin())
        .collect()
}

fn max_abs(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |a, &b| a.max(b.abs()))
}

/// The full repro sequence. `feed` controls whether the mapped PID's
/// channels keep receiving PCM while the fade runs (the live session's
/// distinguishing feature vs the tones-only loopback that did NOT repro).
fn repro(feed_during_fade: bool) {
    let mut m = mixer();
    m.set_limiter_enabled(false);

    // Live receive session: one stereo PID mapped at strips 0/1 (the
    // pair the FX chain lives on) — the feed_pcm path.
    m.map_pid_keyed(0x1_0101, 0, 2).expect("map pid");
    let pcm = |n: usize| -> Vec<f32> {
        let l = sine(220.0, 0.4, n);
        let r = sine(277.18, 0.4, n);
        let mut inter = vec![0.0f32; n * 2];
        for i in 0..n {
            inter[i * 2] = l[i];
            inter[i * 2 + 1] = r[i];
        }
        inter
    };
    // Prime the FIFOs (a block of standing depth, like a live stream).
    for _ in 0..4 {
        m.feed_interleaved(0, &pcm(BLOCK as usize), 2)
            .expect("feed");
    }

    // The FX chain under test: cabinet + detune + tuner + looper, all
    // enabled (the looper allocates its tapes at first activate).
    for (slot, fx) in [CABINET, DETUNE, TUNER, LOOPER].into_iter().enumerate() {
        m.fx_set_slot(0, slot as u32, fx).expect("set slot");
        m.fx_set_enabled(0, slot as u32, true).expect("enable");
    }

    let id = m.save_scene();
    assert_eq!(id, 1, "first scene id");

    // Clear the chain via the picker path.
    for slot in 0..4 {
        m.fx_set_slot(0, slot, -1).expect("clear slot");
    }
    let snap = m.console_snapshot();
    for slot in 0..4 {
        assert_eq!(snap.fx_chains[0][slot].fx, -1, "slot {slot} cleared");
    }

    // Recall as a 500 ms fade, then run the console across the fade
    // exactly like the live page (PCM flowing, process per block).
    m.recall_scene_fade(id, 500.0).expect("fade recall");
    let fade_blocks = (500.0 / (BLOCK as f64 * 1000.0 / SR as f64)).ceil() as usize + 4;
    for _ in 0..fade_blocks {
        if feed_during_fade {
            m.feed_interleaved(0, &pcm(BLOCK as usize), 2)
                .expect("feed");
        }
        m.process_block(BLOCK).expect("process");
    }

    // 1. The chain must be restored.
    let snap = m.console_snapshot();
    let restored = [CABINET as i16, DETUNE as i16, TUNER as i16, LOOPER as i16];
    for (slot, &fx) in restored.iter().enumerate() {
        assert_eq!(
            snap.fx_chains[0][slot].fx, fx,
            "slot {slot} restored to fx {fx}"
        );
        assert!(snap.fx_chains[0][slot].enabled, "slot {slot} re-enabled");
    }

    // 2. set_mute still gates the master (the reported silent death).
    m.feed_interleaved(0, &pcm(BLOCK as usize), 2)
        .expect("feed");
    let live = m.process_block(BLOCK).expect("process").to_vec();
    assert!(max_abs(&live) > 1e-3, "master carries signal before mute");
    m.set_channel_mute(0, true).expect("mute ch 0");
    m.set_channel_mute(1, true).expect("mute ch 1");
    m.feed_interleaved(0, &pcm(BLOCK as usize), 2)
        .expect("feed");
    let muted = m.process_block(BLOCK).expect("process").to_vec();
    assert!(
        max_abs(&muted) < 1e-6,
        "set_channel_mute after recall still silences the master (got {})",
        max_abs(&muted)
    );
    m.set_channel_mute(0, false).expect("unmute ch 0");
    m.set_channel_mute(1, false).expect("unmute ch 1");

    // 3. fx_set_slot still sticks (no budget-poisoning residue).
    m.fx_set_slot(0, 5, 8).expect("set a fresh delay slot");
    let snap = m.console_snapshot();
    assert_eq!(snap.fx_chains[0][5].fx, 8, "fresh fx_set_slot sticks");

    // 4. save_scene still returns fresh ids.
    let id2 = m.save_scene();
    assert_eq!(id2, 2, "save_scene survives the recall");
}

#[test]
fn live_session_recall_keeps_control_plane_alive() {
    repro(true);
}

#[test]
fn starved_recall_keeps_control_plane_alive() {
    repro(false);
}
