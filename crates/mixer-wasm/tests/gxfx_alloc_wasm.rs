//! Wasm-side allocation gate for the gxfx interactive path (node).
//!
//! On wasm32 the gxfx C++ malloc family routes through the Rust shims in
//! src/gxfx.rs (std::alloc), so a counting `#[global_allocator]` sees the
//! C++ allocations too — including the megabyte-scale delay/echo tapes
//! `activate(true)` new's. This gate quantifies EXACTLY what each
//! interactive op allocates on the real wasm build and pins the
//! RT-critical invariants:
//!
//!   * steady-state `process_block` with a full enabled chain
//!     (delay + reverb + distortion): ZERO allocations per block —
//!     a per-block allocation would be `memory.grow` on the audio
//!     thread (page zeroing stalls the render quantum).
//!   * param writes interleaved with blocks (the knob-drag shape):
//!     ZERO allocations — param writes are float stores into slot
//!     storage, anything else is a regression.
//!   * enable/disable toggling keeps the documented once-per-activate
//!     shape: re-enabling delay re-allocates exactly its two 2 MiB
//!     tapes (2 allocs, 4,194,336 bytes) and nothing else; echo its
//!     two 1 MiB tapes; reverb/distortion nothing (state inline in the
//!     instance / no lazily-allocated buffers).
//!
//! Measured baseline (2026-10-20, release): set_slot allocates 260 B
//! (delay), ~300 KiB (reverb — inline state), ~9.3 KiB (distortion —
//! resampler); first delay enable allocates 4,194,336 B. The one-time
//! creates are asserted as bounds, the RT-path invariants as exact
//! zeros.
//!
//! Run: wasm-pack test --node --release --test gxfx_alloc_wasm -- --nocapture

use js_sys::Float32Array;
use wasm_bindgen_test::*;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;
/// One stereo s302m PID chunk shape (20 ms interleaved frames).
const CHUNK: usize = 960;
const NPID: u16 = 4;

static ALLOCS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static BYTES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

struct CountingAlloc;

unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::System.alloc(layout) };
        if !p.is_null() {
            ALLOCS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            BYTES.fetch_add(layout.size() as u64, std::sync::atomic::Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// (allocs, bytes) snapshot.
fn rd() -> (u64, u64) {
    (
        ALLOCS.load(std::sync::atomic::Ordering::Relaxed),
        BYTES.load(std::sync::atomic::Ordering::Relaxed),
    )
}

/// Live-console harness: PCM-paced feeds (the QuicSRT delivery shape)
/// around every `process_block`, with a block counter to pace them.
struct Console {
    m: mixer_wasm::MixerWasm,
    fed: usize,
    blocks: usize,
}

impl Console {
    fn new() -> Self {
        let mut m = mixer_wasm::MixerWasm::new(SR, BLOCK, 256).expect("ctor");
        m.set_limiter_enabled(false);
        for pid in 0..NPID {
            m.map_pid_keyed(0x0100 + pid as u32, pid as u32 * 2, 2)
                .expect("map");
        }
        Self {
            m,
            fed: 0,
            blocks: 0,
        }
    }
    fn feed(&mut self, key: u32) {
        let fa = Float32Array::new_with_length((CHUNK * 2) as u32);
        for i in 0..fa.length() as usize {
            fa.set_index(i as u32, 0.02);
        }
        self.m.feed_pcm_keyed(key, &fa).expect("feed");
        self.fed += CHUNK;
    }
    /// One paced block: feeds sit outside any caller measurement window
    /// semantics (delivery side, not RT).
    fn block(&mut self) {
        while self.fed < self.blocks * BLOCK as usize + 2048 {
            for pid in 0..NPID {
                self.feed(0x0100 + pid as u32);
            }
        }
        self.m.process_block(BLOCK).expect("process");
        self.blocks += 1;
    }
}

/// The full E2E "severe variant" chain on pair 0: delay (8) + reverb
/// (10) + distortion (2), created, enabled and driven — then dragged and
/// bypass-toggled while counting every allocation.
#[wasm_bindgen_test]
fn fx_chain_interactive_ops_allocation_profile() {
    let mut c = Console::new();
    for _ in 0..64 {
        c.block();
    }

    // ── creates: one-time, bounded ─────────────────────────────────────
    for (slot, fx, max_bytes) in [
        (0u32, 8i32, 4_194_432u64),
        (1, 10, 1_048_576),
        (2, 2, 262_144),
    ] {
        let (a0, b0) = rd();
        c.m.fx_set_slot(0, slot, fx).expect("set_slot");
        let (a1, b1) = rd();
        wasm_bindgen_test::console_log!("set_slot fx={fx}: {} allocs, {} bytes", a1 - a0, b1 - b0);
        assert!(
            b1 - b0 <= max_bytes,
            "fx {fx}: set_slot allocated {} bytes (bound {max_bytes}) — creates are one-time, but this grew beyond the documented instance state",
            b1 - b0
        );
    }

    // ── enables: first activate allocates the memory-class tapes ──────
    // delay: exactly its two 2 MiB tapes; reverb/distortion: nothing.
    let (a0, b0) = rd();
    c.m.fx_set_enabled(0, 0, true).expect("enable delay");
    let (a1, b1) = rd();
    wasm_bindgen_test::console_log!("enable delay: {} allocs, {} bytes", a1 - a0, b1 - b0);
    assert_eq!(
        (a1 - a0, b1 - b0),
        (2, 4_194_336),
        "delay first activate must allocate exactly the two 2 MiB tapes"
    );
    for slot in [1u32, 2] {
        let (a0, b0) = rd();
        c.m.fx_set_enabled(0, slot, true).expect("enable");
        let (a1, b1) = rd();
        wasm_bindgen_test::console_log!(
            "enable slot={slot}: {} allocs, {} bytes",
            a1 - a0,
            b1 - b0
        );
        assert_eq!(
            (a1 - a0, b1 - b0),
            (0, 0),
            "slot {slot}: no lazily-allocated buffers expected at activate"
        );
    }

    // warm the enabled chain
    for _ in 0..64 {
        c.block();
    }

    // ── steady state: ZERO allocations per block with the full chain ──
    let (a0, _) = rd();
    let n = 200usize;
    for _ in 0..n {
        c.block();
    }
    let (a1, _) = rd();
    wasm_bindgen_test::console_log!("steady chain: {} allocs over {} blocks", a1 - a0, n);
    assert_eq!(
        a1 - a0,
        0,
        "process_block allocated {} times with the full FX chain enabled — the RT path must be allocation-free on wasm (memory.grow = render-quantum stall)",
        a1 - a0
    );

    // ── the knob-drag shape: param writes interleaved with blocks ─────
    // delay ordinals: 1 = R Level, 2 = R Time (the E2E trigger knob).
    let (a0, _) = rd();
    for k in 0..40u32 {
        c.m.fx_set_param(0, 0, 2, 750.0 + k as f32).expect("R Time");
        c.m.fx_set_param(0, 0, 1, -10.0 + k as f32)
            .expect("R Level");
        c.m.fx_set_param(0, 2, 2, 0.5 + k as f32 / 100.0)
            .expect("drive");
        c.block();
    }
    let (a1, _) = rd();
    wasm_bindgen_test::console_log!("drag sim (120 writes + 40 blocks): {} allocs", a1 - a0);
    assert_eq!(
        a1 - a0,
        0,
        "param writes allocated on wasm — they are plain float stores into slot storage; anything else is a regression (the faust ramp machinery is per-sample arithmetic)"
    );

    // ── bypass churn: free + realloc exactly the documented tapes ─────
    for (slot, want_allocs, want_bytes) in [(0u32, 2u64, 4_194_336u64), (1, 0, 0), (2, 0, 0)] {
        let (a0, b0) = rd();
        c.m.fx_set_enabled(0, slot, false).expect("disable");
        let (am, bm) = rd();
        assert_eq!(
            (am - a0, bm - b0),
            (0, 0),
            "slot {slot}: disable must not allocate"
        );
        let (a0, b0) = rd();
        c.m.fx_set_enabled(0, slot, true).expect("re-enable");
        let (a1, b1) = rd();
        wasm_bindgen_test::console_log!(
            "churn slot={slot}: disable 0 B, re-enable {} allocs {} bytes",
            a1 - a0,
            b1 - b0
        );
        assert_eq!(
            (a1 - a0, b1 - b0),
            (want_allocs, want_bytes),
            "slot {slot}: re-enable must re-allocate exactly the documented tape set"
        );
    }
}
