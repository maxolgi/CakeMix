//! Steady-state allocation regression test (native, no JS).
//!
//! `process_block()` runs on the AudioWorklet real-time thread with a
//! 2.67 ms budget (128 frames @ 48 k). Heap allocation there is illegal:
//! wasm `memory.grow` page-zeroing and JS-side GC of accumulated garbage
//! stall the render callback past its deadline, and a stall also drains
//! the near-zero input FIFOs into starvation clicks.
//!
//! Before the fix, one `process()` allocated per channel per block inside
//! the engine's `process_mix` (working vec, unconditional pre-fader clone,
//! stereo pair, per-call master vecs) plus a Vec-per-channel in
//! `Meter::process` — ~95k allocs/s at 30 stereo tracks. This test pins
//! the steady state at ZERO allocations per block with the full console
//! active (60 input channels of EQ, limiter, metering, 2 buses × 4 slots,
//! 16-channel direct-out tap armed).
//!
//! A timed scene cross-fade (`recall_scene_fade`) also runs ACROSS the
//! measured window: the two scene snapshots are allocated once at recall
//! start (control plane, outside the measurement), and the per-block
//! interpolation must be just as allocation-free as the rest of the
//! block. The fade is sized so t crosses 0.5 inside the window — the
//! boolean snaps (mute, dynamics enable) then exercise the HashSet
//! insert and lazily-constructed-module paths mid-measurement, which is
//! exactly why recall_scene_fade pre-reserves the mute/solo sets.
//!
//! Run: cargo test -p mixer-wasm --test alloc_test -- --nocapture

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use mixer_wasm::MixerWasm;

static LIVE: AtomicU64 = AtomicU64::new(0);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            LIVE.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;
const PIDS: u16 = 30;
const CHUNK: usize = 960; // 20 ms interleaved frames per PID

/// Full-console steady state: 30 stereo PIDs, limiter on (EQ on — the
/// six-band chain is the default), 2 buses with 4 sourced slots each,
/// 16-channel tap armed. After warmup (FIFO capacities, EQ scratch,
/// HashMap entries all settled), 200 consecutive `process_block` calls
/// must not allocate a single byte.
#[test]
fn process_block_is_alloc_free_steady_state() {
    let mut m = MixerWasm::new(SR, BLOCK, 256).expect("ctor");
    // Limiter ON (default) — include its per-sample path.
    for pid in 0..PIDS {
        let ch_start = pid as u32 * 2;
        m.map_pid(0x100 + pid, ch_start, 2).expect("map");
    }
    for bus in 0..2u32 {
        for slot in 0..4u32 {
            m.set_bus_source(bus, slot, slot * 2).expect("bus source");
        }
    }
    m.set_channel_tap(16);

    // Interleaved ramp chunk per PID.
    let chunk: Vec<f32> = (0..CHUNK)
        .flat_map(|f| {
            let v = 0.02 + 1e-5 * f as f32;
            [v, v]
        })
        .collect();

    // Warmup: settle every capacity (FIFO, EQ f64 scratch, pass1 vec,
    // meter rings, HashMap entries, engine channels).
    for _ in 0..64 {
        for pid in 0..PIDS {
            let ch_start = pid as u32 * 2;
            m.feed_interleaved(ch_start, &chunk, 2).expect("feed");
        }
        let out = m.process_block(BLOCK).expect("process");
        assert!(out.len() == BLOCK as usize * 2);
    }

    // Scene cross-fade across the measured window (see the module docs):
    // target scene = pre-mutation state (ch 3 muted, comp on ch 5, gain
    // 1.0, default EQ/master); from-state = mutated. 150 blocks of fade:
    // blocks 1-150 interpolate (t crosses 0.5 at block 75 — mute snap on,
    // comp snap on), blocks 151-200 return to fade-free steady state.
    m.set_channel_mute(3, true).expect("mute");
    m.enable_compressor(5).expect("comp on");
    let scene_id = m.save_scene();
    m.set_channel_mute(3, false).expect("unmute");
    m.disable_compressor(5);
    m.set_channel_gain(0, 0.5).expect("gain");
    m.set_eq_band_freq(0, 2, 250.0).expect("eq freq");
    m.set_bus_gain(0, 0.7);
    m.set_master_gain(0.8);
    m.recall_scene_fade(scene_id, 150.0 * BLOCK as f64 * 1000.0 / SR as f64)
        .expect("fade recall");

    let mut blocks_measured = 0u32;
    let mut total_allocs = 0u64;
    let mut worst_block = String::new();
    for i in 0..200u32 {
        // Feeds sit OUTSIDE the measurement (delivery side, not RT).
        for pid in 0..PIDS {
            let ch_start = pid as u32 * 2;
            m.feed_interleaved(ch_start, &chunk, 2).expect("feed");
        }
        let before = LIVE.load(Ordering::Relaxed);
        let out = m.process_block(BLOCK).expect("process");
        let after = LIVE.load(Ordering::Relaxed);
        if after > before {
            let bytes = after - before;
            total_allocs += bytes;
            if worst_block.is_empty() {
                worst_block = format!("block {i}: {bytes} bytes");
            }
        }
        assert!(out.len() == BLOCK as usize * 2);
        blocks_measured += 1;
    }

    println!(
        "measured {blocks_measured} blocks: {total_allocs} bytes allocated total; first offender: {worst_block}"
    );
    assert_eq!(
        total_allocs, 0,
        "process_block allocated on the RT thread — first offender: {worst_block}"
    );

    // Honesty: the console must have actually mixed audio (not a silent
    // trivial path that would pass by doing nothing).
    let out = m.process_block(BLOCK).expect("process");
    let peak = out.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
    assert!(peak > 0.001, "suspect silent path — peak {peak}");
}
