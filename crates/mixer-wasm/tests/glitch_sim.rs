//! Long-run stability simulation of the binding's exact process() loop.
//!
//! Replicates crates/mixer-wasm/src/lib.rs process() verbatim (per-channel
//! process_mix calls, master sum, limiter, meter) with the production
//! scenario: 30 stereo PIDs = 60 channels, 20 ms (960-frame) interleaved
//! chunks arriving at 1x realtime, 128-frame blocks drained every 2.67 ms.
//!
//! Measures over a long run (default 5 min of audio):
//! - per-block wall time (max / p99) vs the 2.67 ms real-time budget
//! - process heap growth (VmRSS sampling) — unbounded growth would force
//!   wasm memory.grow stalls on the audio thread in production
//! - output sanity: NaN/Inf, amplitude bounds, zero-runs (starvation)
//!
//! Run: cargo test -p mixer-wasm --test glitch_sim -- --nocapture

// Counting global allocator: tracks live bytes (alloc - dealloc) and the
// high-water mark, so a linear climb proves a real leak (vs glibc RSS
// behavior on freed-but-unreturned pages).
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

static LIVE: AtomicU64 = AtomicU64::new(0);
static HIGH_WATER: AtomicU64 = AtomicU64::new(0);

struct CountingAlloc;

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            let live =
                LIVE.fetch_add(layout.size() as u64, Ordering::Relaxed) + layout.size() as u64;
            HIGH_WATER.fetch_max(live, Ordering::Relaxed);
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

use std::collections::VecDeque;
use std::time::Instant;

use oximedia_audio::ChannelLayout;
use oximedia_mixer::channel::{db_to_linear, ChannelType, PanLaw};
use oximedia_mixer::effects_chain::AudioEffect;
use oximedia_mixer::metering::{Meter, MeterBallistics};
use oximedia_mixer::oversampled_limiter::OversampledLimiter;
use oximedia_mixer::{AudioMixer, ChannelId, ChannelProcessParams, MixerConfig, PanLawType};

const SR: u32 = 48_000;
const BLOCK: usize = 128;
const PIDS: u16 = 30;
const CH_PER_PID: usize = 2;
const NCH: usize = (PIDS as usize) * CH_PER_PID; // 60
const CHUNK: usize = 960; // 20 ms interleaved frames per PID
const MAX_QUEUE: usize = 8192;

struct Sim {
    engine: AudioMixer,
    ids: Vec<ChannelId>,
    queues: Vec<VecDeque<f32>>,
    block_inputs: Vec<f32>,
    eq_scratch: Vec<f32>,
    master_left: Vec<f32>,
    master_right: Vec<f32>,
    stereo_out: Vec<f32>,
    limiter_l: OversampledLimiter,
    limiter_r: OversampledLimiter,
    master_meter: Meter,
    // per-channel EQ state lives in the binding (EqEffect), not the engine;
    // replicate with the engine's ParametricEq via effects chain? No — the
    // binding runs the EQ OUTSIDE process_mix (on eq_scratch) then calls
    // process_mix. We replicate that with the same EqEffect path is not
    // possible natively (it's in the binding crate). Instead we include the
    // 6-band EQ by calling it through the binding's own effects module —
    // it's `pub`.
    eqs: Vec<mixer_wasm::effects::EqEffect>,
}

fn rss_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("VmRSS:") {
            return v.split_whitespace().next()?.parse().ok();
        }
    }
    None
}

impl Sim {
    fn new() -> Self {
        let mut engine = AudioMixer::new(MixerConfig {
            sample_rate: SR,
            buffer_size: BLOCK,
            max_channels: 256,
            ..Default::default()
        });
        let mut ids = Vec::with_capacity(NCH);
        let mut eqs = Vec::with_capacity(NCH);
        for i in 0..NCH {
            let id = engine
                .add_channel(format!("ch{i}"), ChannelType::Mono, ChannelLayout::Mono)
                .unwrap();
            if let Ok(ch) = engine.get_channel_mut(id) {
                ch.set_pan_law(PanLaw::Linear);
            }
            ids.push(id);
            eqs.push(mixer_wasm::effects::EqEffect::six_band(SR));
        }
        Sim {
            engine,
            ids,
            queues: vec![VecDeque::with_capacity(BLOCK); NCH],
            block_inputs: vec![0.0; NCH * BLOCK],
            eq_scratch: vec![0.0; BLOCK],
            master_left: vec![0.0; BLOCK],
            master_right: vec![0.0; BLOCK],
            stereo_out: vec![0.0; BLOCK * 2],
            limiter_l: OversampledLimiter::new(-0.3, 50.0, 4, SR as f32),
            limiter_r: OversampledLimiter::new(-0.3, 50.0, 4, SR as f32),
            master_meter: Meter::new(2, SR, MeterBallistics::Fast),
            eqs,
        }
    }

    /// Feed one 20 ms stereo chunk for pid (2 channels: ch = pid*2, pid*2+1).
    fn feed(&mut self, pid: u16, inter: &[f32]) {
        let frames = inter.len() / CH_PER_PID;
        for c in 0..CH_PER_PID {
            let ch = (pid as usize) * CH_PER_PID + c;
            for f in 0..frames {
                self.queues[ch].push_back(inter[f * CH_PER_PID + c]);
            }
            let q = &mut self.queues[ch];
            let overflow = q.len().saturating_sub(MAX_QUEUE);
            if overflow > 0 {
                q.drain(0..overflow);
            }
        }
    }

    /// One process() block — mirrors the binding exactly (pass 1 + limiter +
    /// meter; buses/slots/tap are idle in this scenario).
    fn process(&mut self) -> (f32, f32) {
        for i in 0..BLOCK {
            self.master_left[i] = 0.0;
            self.master_right[i] = 0.0;
        }
        // Drain FIFOs
        self.block_inputs[..NCH * BLOCK].fill(0.0);
        for (ch, queue) in self.queues.iter_mut().enumerate() {
            let base = ch * BLOCK;
            for d in self.block_inputs[base..base + BLOCK].iter_mut() {
                if let Some(s) = queue.pop_front() {
                    *d = s;
                } else {
                    break;
                }
            }
        }
        // Pass 1: per channel — EQ (binding runs it pre-process_mix), then
        // engine process_mix for gain/pan/sum, exactly like the binding.
        let mut peak_l = 0.0f32;
        let mut peak_r = 0.0f32;
        for ch in 0..NCH {
            let id = self.ids[ch];
            let ch_ref = self.engine.get_channel(id).unwrap();
            let pan_law = match ch_ref.pan_law() {
                PanLaw::Linear => PanLawType::Linear,
                PanLaw::Minus3dB => PanLawType::Minus3dB,
                PanLaw::Minus4Dot5dB => PanLawType::Minus4Dot5dB,
                PanLaw::Minus6dB => PanLawType::Minus6dB,
            };
            let params = ChannelProcessParams {
                fader_gain: ch_ref.gain(),
                pan: ch_ref.pan(),
                muted: ch_ref.is_muted(),
                input_gain_db: ch_ref.input().gain_db,
                phase_inverted: ch_ref.is_phase_inverted(),
                pan_law,
            };
            if params.muted {
                continue;
            }
            let base = ch * BLOCK;
            self.eq_scratch[..BLOCK].copy_from_slice(&self.block_inputs[base..base + BLOCK]);
            self.eqs[ch].process(&mut self.eq_scratch[..BLOCK]);
            let (cl, cr) = self
                .engine
                .engine_mut()
                .process_mix(&[(id, params)], &self.eq_scratch[..BLOCK]);
            for i in 0..BLOCK {
                self.master_left[i] += cl[i];
                self.master_right[i] += cr[i];
            }
        }
        // Limiter
        for i in 0..BLOCK {
            self.master_left[i] = self.limiter_l.process_sample(self.master_left[i]);
            self.master_right[i] = self.limiter_r.process_sample(self.master_right[i]);
            peak_l = peak_l.max(self.master_left[i].abs());
            peak_r = peak_r.max(self.master_right[i].abs());
        }
        for i in 0..BLOCK {
            self.stereo_out[i * 2] = self.master_left[i];
            self.stereo_out[i * 2 + 1] = self.master_right[i];
        }
        self.master_meter.process(&self.stereo_out[..BLOCK * 2]);
        (peak_l, peak_r)
    }
}

#[test]
fn long_run_stability_60ch() {
    let minutes: f32 = std::env::var("SIM_MINUTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5.0);
    let total_frames = (SR as f32 * minutes * 60.0) as usize;
    let total_blocks = total_frames / BLOCK;
    let mut sim = Sim::new();

    // Per-track content: distinct sines, 0.3 amplitude (30 tracks summing hot
    // enough to keep the limiter working).
    let mut phase = vec![0.0f32; NCH];
    let freqs: Vec<f32> = (0..NCH)
        .map(|i| 100.0 + (i * 173) as f32 % 4000.0)
        .collect();

    let mut next_chunk_frame = 0usize; // frame index at which next chunk is due
    let mut chunk_idx = 0usize;
    let rss_start = rss_kb().unwrap_or(0);
    let mut rss_max = rss_start;
    let mut rss_last_sample = rss_start;
    let mut rss_samples: Vec<(usize, u64)> = Vec::new();
    let alloc_base = LIVE.load(Ordering::Relaxed);
    let mut alloc_samples: Vec<(usize, u64)> = Vec::new();

    let mut max_block_us = 0.0f64;
    let mut block_us_sum = 0.0f64;
    let mut block_us_hist: Vec<f64> = Vec::with_capacity(total_blocks);
    let mut nan_inf = 0u64;
    let mut global_peak = 0.0f32;
    let mut zero_runs = 0u64;
    let mut in_zero_run = false;
    let mut max_zero_run = 0usize;
    let mut cur_zero_run = 0usize;
    let mut over_ceiling = 0u64; // samples > 0 dBFS (limiter failed)
    let ceiling = db_to_linear(0.0);

    let t0 = Instant::now();
    for b in 0..total_blocks {
        let block_frame = b * BLOCK;
        // Deliver any chunks due before/at this block (TSBPD-paced 1x).
        while next_chunk_frame <= block_frame {
            for pid in 0..PIDS {
                let mut inter = Vec::with_capacity(CHUNK * CH_PER_PID);
                for _ in 0..CHUNK {
                    for c in 0..CH_PER_PID {
                        let ch = (pid as usize) * CH_PER_PID + c;
                        phase[ch] += 2.0 * std::f32::consts::PI * freqs[ch] / SR as f32;
                        let s = 0.3 * phase[ch].sin();
                        inter.push(s);
                    }
                }
                sim.feed(pid, &inter);
            }
            next_chunk_frame += CHUNK;
            chunk_idx += 1;
        }

        let bt = Instant::now();
        let (pl, pr) = sim.process();
        let bt_us = bt.elapsed().as_secs_f64() * 1e6;
        max_block_us = max_block_us.max(bt_us);
        block_us_sum += bt_us;
        block_us_hist.push(bt_us);

        global_peak = global_peak.max(pl).max(pr);
        if pl.is_nan() || pr.is_nan() || pl.is_infinite() || pr.is_infinite() {
            nan_inf += 1;
        }
        if pl > ceiling || pr > ceiling {
            over_ceiling += 1;
        }
        // Zero-run detection on L (starvation shows as silence)
        let mut block_zero = true;
        for i in (0..BLOCK).step_by(8) {
            if sim.stereo_out[i * 2].abs() > 1e-6 {
                block_zero = false;
                break;
            }
        }
        if block_zero {
            if !in_zero_run {
                zero_runs += 1;
                in_zero_run = true;
                cur_zero_run = 1;
            } else {
                cur_zero_run += 1;
            }
            max_zero_run = max_zero_run.max(cur_zero_run);
        } else {
            in_zero_run = false;
        }

        // RSS + allocator sample every ~2000 blocks
        if b % 2000 == 1999 {
            if let Some(r) = rss_kb() {
                rss_max = rss_max.max(r);
                rss_samples.push((b, r));
                rss_last_sample = r;
            }
            alloc_samples.push((b, LIVE.load(Ordering::Relaxed) - alloc_base));
        }
        let _ = chunk_idx;
    }
    let _ = rss_last_sample;
    let elapsed = t0.elapsed();

    block_us_hist.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p99 = block_us_hist[(total_blocks as f64 * 0.99) as usize];
    let avg = block_us_sum / total_blocks as f64;

    println!("=== long-run 60ch sim: {minutes} min audio, {total_blocks} blocks ===");
    println!("wall: {elapsed:?}");
    println!(
        "block time: avg {avg:.1} us, p99 {p99:.1} us, max {max_block_us:.1} us  (budget 2667 us)"
    );
    println!("global peak: {} dBFS", 20.0 * global_peak.log10());
    println!("NaN/Inf blocks: {nan_inf}");
    println!("blocks over 0 dBFS ceiling: {over_ceiling}");
    println!("zero-runs (starvation): {zero_runs}, longest {max_zero_run} blocks");
    println!("RSS start: {rss_start} kB, max: {rss_max} kB");
    if rss_samples.len() > 1 {
        println!("RSS trajectory (block, kB):");
        for (b, r) in rss_samples.iter().step_by(rss_samples.len() / 8 + 1) {
            println!("  block {b:>7}: {r} kB");
        }
    }
    if alloc_samples.len() > 1 {
        println!("LIVE ALLOC trajectory (block, bytes above baseline):");
        for (b, a) in alloc_samples.iter().step_by(alloc_samples.len() / 8 + 1) {
            println!("  block {b:>7}: {a} B");
        }
        let hi = HIGH_WATER.load(Ordering::Relaxed) - alloc_base;
        println!("allocator high-water above baseline: {hi} B");
        let (b0, a0) = alloc_samples[0];
        let (b1, a1) = *alloc_samples.last().unwrap();
        println!(
            "leak rate: {} B/block over blocks {b0}..{b1}",
            (a1.saturating_sub(a0)) / (b1 - b0) as u64
        );
    }

    assert!(nan_inf == 0, "DSP produced NaN/Inf");
    assert!(
        zero_runs == 0,
        "FIFO starved {zero_runs} times (longest {max_zero_run} blocks)"
    );
    assert!(
        p99 < 2667.0,
        "p99 block time {p99:.1} us exceeds the 2667 us real-time budget"
    );
}
