//! Wasm-side regression gate for the c_floor calloc header bug (node).
//!
//! The live-E2E control-plane death (2026-10-21): the gxfx tuner's
//! `fftwf_malloc` + trailing `memset` is folded by LLVM into `calloc`;
//! c_floor's calloc allocated WITHOUT the 16-byte header while `free`
//! reconstructed the dealloc layout as `size + HEADER` — a GlobalAllocator
//! contract violation that made dlmalloc's free assert (panic → trap) on
//! the FIRST calloc'd chunk, i.e. the first tuner slot teardown. The
//! escaping panic left wasm-bindgen's reentrancy guard poisoned, so every
//! later `&mut self` call on the MixerWasm ("recursive use of an object
//! detected") threw and the worklet's blanket catches swallowed it: every
//! console setter, scene save/recall, PCM feed and process() died
//! silently while `&self` getters (fx_out_param, console_params_json)
//! kept answering. Natively glibc absorbs the mismatch (no header
//! contract), so no native test can catch this — it must run on wasm32.
//!
//! Run: wasm-pack test --node --release --test gxfx_calloc_wasm -- --nocapture

use js_sys::Float32Array;
use wasm_bindgen_test::*;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;
const CABINET: i32 = 76;
const DETUNE: i32 = 80;
const TUNER: i32 = 81;
const LOOPER: i32 = 82;

fn mixer() -> mixer_wasm::MixerWasm {
    let mut m = mixer_wasm::MixerWasm::new(SR, BLOCK, 256).expect("ctor");
    m.set_limiter_enabled(false);
    m
}

/// The minimal trap: tuner (id 81) create → destroy cycles. Before the
/// fix the FIRST destroy trapped with dlmalloc's free assertion; eight
/// cycles also churn the heap so a stale-chunk reuse can't hide it.
#[wasm_bindgen_test]
fn tuner_slot_cycles_do_not_corrupt_the_allocator() {
    let mut m = mixer();
    // Strip 0 must exist: the rack holds chains regardless, but the
    // SNAPSHOT (asserted below) only covers pairs whose owner strip
    // exists (same rule as gxfx_console_test's validation test).
    m.set_channel_gain(0, 1.0).expect("strip 0");
    for _ in 0..8 {
        m.fx_set_slot(0, 0, TUNER).expect("tuner set");
        m.fx_set_slot(0, 0, -1).expect("tuner clear");
    }
    // The control plane survived: a fresh set sticks, a scene saves, a
    // block processes (all &mut self — exactly the calls the poisoned
    // reentrancy guard used to reject).
    m.fx_set_slot(0, 1, 8).expect("delay set");
    assert_eq!(m.console_snapshot().fx_chains[0][1].fx, 8);
    assert_eq!(m.save_scene(), 1, "save_scene after tuner cycles");
    m.process_block(BLOCK).expect("process after tuner cycles");
}

/// The full live-E2E repro at the wasm level (the surface the bug was
/// reported on): keyed PID mapped (the feed_pcm path), cabinet + detune
/// + tuner + looper enabled, save, clear the chain, recall as a 500 ms
/// fade while PCM flows and blocks process — then the chain must be
/// restored and the whole control surface still work.
#[wasm_bindgen_test]
fn live_session_recall_keeps_control_plane_alive() {
    let mut m = mixer();
    // Live receive session: one stereo PID at strips 0/1 (feed_pcm path).
    m.map_pid_keyed(0x1_0101, 0, 2).expect("map pid");
    let pcm = Float32Array::new_with_length(BLOCK * 2);
    for i in 0..pcm.length() as usize {
        let f = (i % BLOCK as usize) as f32;
        pcm.set_index(
            i as u32,
            0.3 * f32::sin(2.0 * std::f32::consts::PI * 220.0 * f / SR as f32),
        );
    }
    for _ in 0..4 {
        m.feed_pcm_keyed(0x1_0101, &pcm).expect("prime fifo");
    }

    for (slot, fx) in [CABINET, DETUNE, TUNER, LOOPER].into_iter().enumerate() {
        m.fx_set_slot(0, slot as u32, fx).expect("set slot");
        m.fx_set_enabled(0, slot as u32, true).expect("enable");
    }
    let id = m.save_scene();
    assert_eq!(id, 1, "first scene id");

    for slot in 0..4 {
        m.fx_set_slot(0, slot, -1).expect("clear slot");
    }

    m.recall_scene_fade(id, 500.0).expect("fade recall");
    let fade_blocks = (500.0 / (BLOCK as f64 * 1000.0 / SR as f64)).ceil() as usize + 4;
    for _ in 0..fade_blocks {
        m.feed_pcm_keyed(0x1_0101, &pcm).expect("feed");
        m.process_block(BLOCK).expect("process");
    }

    // 1. The chain is restored (the reported "recall restores NOTHING").
    let snap = m.console_snapshot();
    for (slot, &fx) in [CABINET as i16, DETUNE as i16, TUNER as i16, LOOPER as i16]
        .iter()
        .enumerate()
    {
        assert_eq!(snap.fx_chains[0][slot].fx, fx, "slot {slot} restored");
        assert!(snap.fx_chains[0][slot].enabled, "slot {slot} re-enabled");
    }

    // 2. set_mute still gates the master (the reported silent death).
    fn master_energy(m: &mut mixer_wasm::MixerWasm, pcm: &Float32Array) -> f32 {
        m.feed_pcm_keyed(0x1_0101, pcm).expect("feed");
        m.process_block(BLOCK)
            .expect("process")
            .iter()
            .fold(0.0f32, |a, &b| a.max(b.abs()))
    }
    assert!(master_energy(&mut m, &pcm) > 1e-3, "signal before mute");
    m.set_channel_mute(0, true).expect("mute");
    m.set_channel_mute(1, true).expect("mute");
    assert!(master_energy(&mut m, &pcm) < 1e-6, "mute gates the master");
    m.set_channel_mute(0, false).expect("unmute");
    m.set_channel_mute(1, false).expect("unmute");

    // 3. fx_set_slot still sticks; 4. save_scene still returns fresh ids.
    m.fx_set_slot(0, 5, 8).expect("fresh slot set");
    assert_eq!(m.console_snapshot().fx_chains[0][5].fx, 8);
    assert_eq!(m.save_scene(), 2, "save_scene after recall");
}
