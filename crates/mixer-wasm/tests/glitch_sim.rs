//! Real-time ceiling simulation driving `MixerWasm::process_block` directly.
//!
//! The binding IS the honest path now (FIFO drain → staging of 256 strips
//! with dynamics + EQ → nine `process_mix_rt` engine instances → bus tail
//! → master gain → limiter → meter), so this simulates the worklet's real
//! workload instead of a hand-rolled replica. Two scenarios:
//!
//! - `realistic_load()`: 30 stereo PIDs (60 strips), 2 buses × 4 slots,
//!   compressor on 4 strips, 16-channel tap armed.
//! - `full_load()`: 64 stereo PIDs (128 strips) + all 8 buses × 16 slots
//!   = 256 strips, comp + gate on strips 0-15 and the bus-0 slots, two
//!   buses off-main, tap armed. Limiter + six-band EQ at defaults.
//!
//! Delivery is paced exactly like `drift_test::simulate`: whole
//! 960-frame (20 ms) interleaved chunks per PID arrive while the
//! standing lead is under 512 frames, at 1.0x (the full-load test also
//! runs a 300 ppm-fast pass to confirm the elastic buffer stays
//! bounded). Only `process_block` is timed — feeds are the delivery
//! side, not the real-time thread — against the 2667 µs budget
//! (128 frames @ 48 k).
//!
//! Allocation pinning lives in `alloc_test.rs`; this file measures time
//! and output sanity only.
//!
//! Run: cargo test -p mixer-wasm --release --test glitch_sim -- --nocapture
//! Full load (ignored): run with `--ignored` (or SIM_MINUTES=1 to set the
//! duration, default 1.0 min):
//!   SIM_MINUTES=1 cargo test -p mixer-wasm --release --test glitch_sim \
//!     -- --ignored --nocapture

use std::time::{Duration, Instant};

use mixer_wasm::MixerWasm;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;
const CHUNK: usize = 960; // 20 ms interleaved frames per PID
/// Standing lead (frames) the pacer maintains before each block.
const LEAD: f64 = 512.0;
/// Per-track tone amplitude — hot enough to work the limiter when ~60
/// strips sum.
const AMP: f32 = 0.3;
/// Real-time budget for one 128-frame block at 48 kHz (µs).
const BUDGET_US: f64 = 2_667.0;

fn sim_minutes() -> f64 {
    std::env::var("SIM_MINUTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0)
}

/// A configured console plus the metadata the runner reports.
struct Rig {
    label: &'static str,
    m: MixerWasm,
    /// Stereo PID mappings feeding strips 0..pids*2.
    pids: u16,
    /// Total live strips (input strips + bus slots).
    strips: usize,
    /// Buses whose published output the worklet drains per block.
    drain_buses: &'static [u32],
}

/// `take_bus_output` constructs a `js_sys::Float32Array`; wasm-bindgen
/// imported functions panic on non-wasm targets, so the per-block drain
/// only runs under wasm (wasm-bindgen-test). Natively it is skipped —
/// `process_block` still publishes `bus_pub` either way; only the JS-side
/// ~1 KB memcpy per bus is left out.
#[cfg(target_family = "wasm")]
fn drain_bus_outputs(m: &mut MixerWasm, buses: &[u32]) {
    for &b in buses {
        let _ = m.take_bus_output(b);
    }
}

#[cfg(not(target_family = "wasm"))]
fn drain_bus_outputs(_m: &mut MixerWasm, _buses: &[u32]) {}

/// 30 stereo PIDs (60 strips), 2 buses × 4 slots, compressor on 4 strips,
/// 16-channel tap armed. Limiter + six-band EQ at defaults.
fn realistic_load() -> Rig {
    let mut m = MixerWasm::new(SR, BLOCK, 256).expect("ctor");
    for pid in 0..30u16 {
        m.map_pid(0x100 + pid, pid as u32 * 2, 2).expect("map pid");
    }
    for bus in 0..2u32 {
        for slot in 0..4u32 {
            m.set_bus_source(bus, slot, (bus * 4 + slot) * 2)
                .expect("bus source");
        }
    }
    for ch in 0..4u32 {
        m.enable_compressor(ch).expect("comp");
    }
    m.set_channel_tap(16);
    Rig {
        label: "realistic load",
        m,
        pids: 30,
        strips: 60 + 8,
        drain_buses: &[],
    }
}

/// Full console: 64 stereo PIDs → 128 input strips; all 8 buses × 16
/// slots (distinct sources, wrapped mod 128) → 256 strips total; comp +
/// gate on strips 0-15 and the bus-0 slots (128-143); buses 6-7 off-main
/// (their published outputs would be drained via take_bus_output);
/// 16-channel tap armed. Limiter + six-band EQ at defaults.
fn full_load() -> Rig {
    let mut m = MixerWasm::new(SR, BLOCK, 256).expect("ctor");
    for pid in 0..64u16 {
        m.map_pid(0x100 + pid, pid as u32 * 2, 2).expect("map pid");
    }
    for bus in 0..8u32 {
        for slot in 0..16u32 {
            m.set_bus_source(bus, slot, (bus * 16 + slot) % 128)
                .expect("bus source");
        }
    }
    for strip in 0..16u32 {
        m.enable_compressor(strip).expect("comp");
        m.enable_gate(strip).expect("gate");
    }
    for slot in 0..16u32 {
        let strip = 128 + slot; // bus-0 slots
        m.enable_compressor(strip).expect("comp");
        m.enable_gate(strip).expect("gate");
    }
    m.set_bus_feeds_main(6, false);
    m.set_bus_feeds_main(7, false);
    m.set_channel_tap(16);
    Rig {
        label: "full load (256 strips)",
        m,
        pids: 64,
        strips: 128 + 128,
        drain_buses: &[6, 7],
    }
}

/// Feed one 960-frame interleaved stereo chunk per PID — delivery side,
/// always outside the timed region. Distinct per-channel sines like the
/// old sim: `100 + (ch*173) % 4000` Hz at amplitude 0.3.
fn feed_chunk(
    m: &mut MixerWasm,
    pids: u16,
    freqs: &[f32],
    phase: &mut [f32],
    inter: &mut Vec<f32>,
) {
    for pid in 0..pids {
        let ch0 = pid as usize * 2;
        inter.clear();
        for _ in 0..CHUNK {
            for c in 0..2usize {
                let ch = ch0 + c;
                phase[ch] = (phase[ch] + 2.0 * std::f32::consts::PI * freqs[ch] / SR as f32)
                    % std::f32::consts::TAU;
                inter.push(AMP * phase[ch].sin());
            }
        }
        m.feed_interleaved(ch0 as u32, inter, 2).expect("feed");
    }
}

struct Report {
    label: &'static str,
    strips: usize,
    rate: f64,
    minutes: f64,
    blocks: usize,
    avg_us: f64,
    p99_us: f64,
    max_us: f64,
    starved: u64,
    zero_runs: u64,
    max_zero_run: usize,
    nan_inf: u64,
    over_dbfs: u64,
    peak: f32,
    slips: u64,
    inserts: u64,
    depth: u32,
    wall: Duration,
}

impl Report {
    fn print(&self) {
        println!(
            "=== {}: {} strips | {:.1} min ({} blocks) @ {:.4}x ===",
            self.label, self.strips, self.minutes, self.blocks, self.rate
        );
        println!(
            "  block time : avg {:>8.1} us | p99 {:>8.1} us | max {:>8.1} us | budget {:.0} us | p99 headroom {:.1}x",
            self.avg_us, self.p99_us, self.max_us, BUDGET_US, BUDGET_US / self.p99_us
        );
        println!(
            "  starvation : {} channel-blocks | zero-runs {} (longest {} blocks)",
            self.starved, self.zero_runs, self.max_zero_run
        );
        println!(
            "  output     : NaN/Inf {} samples | >0 dBFS {} samples | peak {:.1} dBFS",
            self.nan_inf,
            self.over_dbfs,
            if self.peak > 0.0 {
                20.0 * self.peak.log10()
            } else {
                -f32::INFINITY
            }
        );
        println!(
            "  elastic    : slips {} | inserts {} | max fifo depth {} frames",
            self.slips, self.inserts, self.depth
        );
        println!("  wall       : {:?}", self.wall);
    }
}

/// Drive `minutes` of playout with delivery running at `rate`
/// (1.0003 = 300 ppm fast), paced like `drift_test::simulate`. Timing
/// wraps `process_block` only; feeds and bus drains sit outside.
fn run(mut rig: Rig, rate: f64, minutes: f64) -> Report {
    let nch = rig.pids as usize * 2;
    let total_blocks = ((SR as f64 * minutes * 60.0) / BLOCK as f64).max(1.0) as usize;
    let freqs: Vec<f32> = (0..nch)
        .map(|i| 100.0 + (i * 173) as f32 % 4000.0)
        .collect();
    let mut phase = vec![0.0f32; nch];
    let mut inter = Vec::with_capacity(CHUNK * 2);
    let mut fed = 0usize; // frames delivered (per channel)

    // Initial preload: 2 chunks (~40 ms).
    for _ in 0..2 {
        feed_chunk(&mut rig.m, rig.pids, &freqs, &mut phase, &mut inter);
        fed += CHUNK;
    }

    let mut hist: Vec<f64> = Vec::with_capacity(total_blocks);
    let mut nan_inf = 0u64;
    let mut over_dbfs = 0u64; // samples above 0 dBFS (limiter failed)
    let mut peak = 0.0f32;
    let mut zero_runs = 0u64; // consecutive all-silent blocks (starvation)
    let mut cur_zero_run = 0usize;
    let mut max_zero_run = 0usize;

    let t0 = Instant::now();
    for block in 0..total_blocks {
        // Deliver whole chunks until the standing lead exceeds LEAD.
        while (fed as f64) < block as f64 * BLOCK as f64 * rate + LEAD {
            feed_chunk(&mut rig.m, rig.pids, &freqs, &mut phase, &mut inter);
            fed += CHUNK;
        }

        let bt = Instant::now();
        let out = rig.m.process_block(BLOCK).expect("process");
        let us = bt.elapsed().as_secs_f64() * 1e6;
        hist.push(us);

        // Zero-run detection on L (sampled every 8 frames like the old sim).
        let mut block_zero = true;
        for f in (0..BLOCK as usize).step_by(8) {
            if out[f * 2].abs() > 1e-6 {
                block_zero = false;
                break;
            }
        }
        if block_zero {
            if cur_zero_run == 0 {
                zero_runs += 1;
            }
            cur_zero_run += 1;
            max_zero_run = max_zero_run.max(cur_zero_run);
        } else {
            cur_zero_run = 0;
        }

        for &s in out {
            if !s.is_finite() {
                nan_inf += 1;
            } else {
                let a = s.abs();
                if a > 1.0 {
                    over_dbfs += 1;
                }
                if a > peak {
                    peak = a;
                }
            }
        }

        drain_bus_outputs(&mut rig.m, rig.drain_buses);
    }
    let wall = t0.elapsed();

    hist.sort_by(|a, b| a.total_cmp(b));
    let n = hist.len();
    let p99 = hist[((n as f64 * 0.99) as usize).min(n - 1)];
    Report {
        label: rig.label,
        strips: rig.strips,
        rate,
        minutes,
        blocks: total_blocks,
        avg_us: hist.iter().sum::<f64>() / n as f64,
        p99_us: p99,
        max_us: hist[n - 1],
        starved: rig.m.starved_blocks(),
        zero_runs,
        max_zero_run,
        nan_inf,
        over_dbfs,
        peak,
        slips: rig.m.elastic_slips(),
        inserts: rig.m.elastic_inserts(),
        depth: rig.m.fifo_max_depth(),
        wall,
    }
}

/// Production-shaped workload must hold the real-time budget: p99 block
/// time under 2667 µs, no starvation, no NaN.
#[test]
fn realistic_load_within_budget() {
    let minutes = sim_minutes();
    let r = run(realistic_load(), 1.0, minutes);
    r.print();
    assert_eq!(
        r.starved, 0,
        "paced 1.0x delivery starved {} channel-blocks",
        r.starved
    );
    assert_eq!(
        r.zero_runs, 0,
        "output went silent {} times (longest {} blocks)",
        r.zero_runs, r.max_zero_run
    );
    assert_eq!(r.nan_inf, 0, "DSP produced {} NaN/Inf samples", r.nan_inf);
    assert!(
        r.p99_us < BUDGET_US,
        "p99 block time {:.1} us exceeds the {:.0} us real-time budget",
        r.p99_us,
        BUDGET_US
    );
}

/// Full-console ceiling measurement (256 strips, dynamics on 32 strips,
/// every bus slot assigned). This MEASURES, it does not promise — the
/// printed numbers are the Phase C deliverable (they go into
/// progress.md); only sanity is asserted (no NaN, bounded FIFO depth).
/// The 300 ppm-fast pass confirms the elastic buffer reconciles drift
/// under full load instead of climbing toward the cap.
///
/// Ignored by default — run with `--ignored` or `SIM_MINUTES=1` (env
/// tunes the duration, default 1.0 min):
///   SIM_MINUTES=1 cargo test -p mixer-wasm --release --test glitch_sim \
///     -- --ignored --nocapture
#[test]
#[ignore]
fn full_load_256_strips_ceiling() {
    let minutes = sim_minutes();

    let r = run(full_load(), 1.0, minutes);
    r.print();
    assert_eq!(r.nan_inf, 0, "DSP produced {} NaN/Inf samples", r.nan_inf);
    assert!(
        r.depth < 4096,
        "FIFO depth {} not bounded — drift accumulating toward the cap",
        r.depth
    );

    let fast = run(full_load(), 1.0003, minutes);
    fast.print();
    assert_eq!(fast.nan_inf, 0, "fast-drift pass produced NaN/Inf");
    assert!(
        fast.depth < 4096,
        "300 ppm-fast delivery climbed to depth {} — trims not engaging",
        fast.depth
    );
    assert_eq!(
        fast.starved, 0,
        "net-fast delivery can never starve (got {} channel-blocks)",
        fast.starved
    );
}
