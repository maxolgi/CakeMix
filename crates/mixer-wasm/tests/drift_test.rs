//! Elastic playout buffer drift known-answer tests (native, no JS).
//!
//! The delivery clock (WebSRT worker TSBPD, `performance.now()`) and the
//! consumption clock (audio hardware) always differ by some ppm. Before the
//! elastic FIFO, a net-fast stream climbed until the 8192-frame drop-oldest
//! cap wrapped into continuous glitching (~28 min at 100 ppm), and a
//! net-slow stream periodically starved. These tests simulate sustained
//! clock drift through `feed_interleaved` + `process_block` exactly as the
//! worklet drives them and verify the drift is reconciled instead of
//! accumulating.
//!
//! Run: cargo test -p mixer-wasm --test drift_test -- --nocapture

use mixer_wasm::MixerWasm;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;
/// Depth statistics window (blocks) — must mirror the binding's constant.
const WINDOW_BLOCKS: usize = 375;

fn mixer() -> MixerWasm {
    let mut m = MixerWasm::new(SR, BLOCK, 256).expect("ctor");
    m.set_limiter_enabled(false);
    for ch in 0..2 {
        m.set_eq_bypass(ch, true).expect("eq bypass");
    }
    m.map_pid(100, 0, 2).expect("map pid");
    m
}

/// Feed one interleaved stereo chunk carrying a slow marker ramp on both
/// channels (frame `f` of the stream carries `0.1 + 1e-4 * (base + f)`).
fn feed_chunk(m: &mut MixerWasm, base_frame: usize, frames: usize) {
    let mut inter = Vec::with_capacity(frames * 2);
    for f in 0..frames {
        let v = 0.1 + 1e-4 * (base_frame + f) as f32;
        inter.push(v);
        inter.push(v);
    }
    m.feed_interleaved(0, &inter, 2).expect("feed");
}

/// Drive `minutes` of playout against a delivery clock running at `rate`
/// (1.0003 = 300 ppm fast). Chunks of 960 frames arrive with a standing
/// lead of at least ~512 frames, mirroring TSBPD-paced delivery. Returns
/// the final mixer.
fn simulate(minutes: usize, rate: f64) -> MixerWasm {
    let mut m = mixer();
    let mut fed = 0usize; // frames delivered
    let mut base = 0usize; // ramp base of next chunk to generate
    let total_blocks = minutes * 60 * SR as usize / BLOCK as usize;

    // Initial preload: 2 chunks (~40 ms).
    for _ in 0..2 {
        feed_chunk(&mut m, base, 960);
        base += 960;
        fed += 960;
    }

    let mut peak_out = 0.0f32;
    for block in 0..total_blocks {
        // Deliver whole chunks until the standing lead exceeds 512 frames.
        while (fed as f64) < block as f64 * BLOCK as f64 * rate + 512.0 {
            feed_chunk(&mut m, base, 960);
            base += 960;
            fed += 960;
        }
        let out = m.process_block(BLOCK).expect("process");
        // L = R = ramp value (identical channels, Linear center pan).
        for s in out.chunks(2) {
            peak_out = peak_out.max(s[0].abs());
        }
        let _ = peak_out;
    }
    m
}

/// Net-fast delivery (300 ppm): trims must engage and hold the standing
/// depth far below the 8192 drop-oldest cap. Without elasticity the FIFO
/// would climb ~14.4 frames/s and hit the cap mid-test, after which every
/// chunk overflows at the head (raw waveform splices = the reported
/// "suddenly glitchy" failure).
#[test]
fn net_fast_drift_stays_bounded() {
    let m = simulate(20, 1.0003);
    let slips = m.elastic_slips();
    let depth = m.fifo_max_depth();
    println!(
        "net-fast 20 min: slips={slips} depth={depth} starved={}",
        m.starved_blocks()
    );
    assert!(slips > 0, "net-fast drift must trigger trim slips");
    assert!(
        depth < 4096,
        "standing depth {depth} not bounded — drift accumulating toward the cap"
    );
    assert_eq!(m.starved_blocks(), 0, "net-fast delivery can never starve");
}

/// Net-slow delivery (200 ppm): inserts must engage before starvation.
/// Without elasticity the deficit (~9.6 frames/s) drains the preload and
/// the output drops to silence ~1 minute in, then glitches permanently as
/// chunk arrival and drain fight.
#[test]
fn net_slow_drift_never_starves() {
    let m = simulate(20, 0.9998);
    let inserts = m.elastic_inserts();
    let starved = m.starved_blocks();
    println!(
        "net-slow 20 min: inserts={inserts} starved={starved} depth={}",
        m.fifo_max_depth()
    );
    assert!(inserts > 0, "net-slow drift must trigger insert slips");
    assert_eq!(
        starved, 0,
        "elastic buffer must bridge a 200 ppm deficit without starving"
    );
}

/// Matched clocks: no correction may fire — jitter inside the hysteresis
/// band must not produce audible slips.
#[test]
fn matched_clocks_never_correct() {
    let m = simulate(3, 1.0);
    assert_eq!(m.elastic_slips(), 0, "trims fired with no drift");
    assert_eq!(m.elastic_inserts(), 0, "inserts fired with no drift");
    assert_eq!(m.starved_blocks(), 0);
}

/// A stopped source must not be time-stretched: once the queue is empty
/// the insert path stays silent (its depth precondition can never hold).
#[test]
fn idle_source_never_inserts() {
    let mut m = mixer();
    feed_chunk(&mut m, 0, 960);
    for _ in 0..(10 * SR as usize / BLOCK as usize) {
        let _ = m.process_block(BLOCK).expect("process");
    }
    assert_eq!(
        m.elastic_inserts(),
        0,
        "inserts fabricated audio for a dead source"
    );
}
