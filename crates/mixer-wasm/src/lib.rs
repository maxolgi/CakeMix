use std::collections::{HashMap, HashSet, VecDeque};
use wasm_bindgen::prelude::*;

use oximedia_audio::ChannelLayout;
use oximedia_mixer::channel::compute_stereo_pan;
use oximedia_mixer::effects_chain::AudioEffect;
use oximedia_mixer::oversampled_limiter::OversampledLimiter;
use oximedia_mixer::{
    bus::{BusId, BusType},
    channel::{db_to_linear, ChannelType, PanLaw},
    ChannelId, MixerConfig,
};

pub mod effects;
use effects::{CompressorEffect, EqEffect, ExpanderEffect, GateEffect};

/// Per-channel input FIFO memory cap (frames @48 k ≈ 170 ms). The elastic
/// playout logic below keeps live depth far below this; the cap is only a
/// last-resort guard against unbounded growth if that logic ever fails.
/// Oldest frames are dropped on overflow.
const MAX_QUEUE_FRAMES: usize = 8192;

// ── Elastic playout buffer ─────────────────────────────────────────────
//
// Delivery is paced by the WebSRT worker's `performance.now()` clock while
// process() consumes at the audio hardware clock. The two always differ by
// some ppm, so a fixed-cap FIFO either accumulates drift until it wraps
// into drop-oldest glitching (net-fast delivery) or drains into periodic
// starvation (net-slow). Instead each elastic FIFO tracks its depth's
// smoothed window-minimum, anchors at the level established around stream
// start, and reconciles drift with tiny crossfaded slips:
//
// - trim: drop SLIP_FRAMES from the queue head, crossfading the splice
//   (net-fast delivery → standing latency would otherwise creep up).
//   Fires when the smoothed depth rises above `anchor + PLAYOUT_BAND`.
// - insert: repeat SLIP_FRAMES at the queue tail, crossfaded (net-slow
//   delivery → the queue would otherwise starve). Fires when the smoothed
//   depth falls below half the anchor (relative — chunked delivery
//   establishes anchors of only a few hundred frames, so an absolute
//   lower band could sit below zero and never trigger), or on live
//   starvation. The burst is capped near the trigger so a between-windows
//   stale statistic cannot overshoot.
//
// All channels of a PID mapping are fed and drained in lockstep, so their
// depths, EWMA and cooldowns stay equal and every channel applies the same
// correction at the same frame boundary (inter-channel phase preserved).
// Corrections are rate-limited (one slip per COOLDOWN blocks) and bounded
// by a hysteresis band around the anchor, so a correction never fires for
// ordinary delivery jitter.
//
// The anchor is reset by `map_pid` — a remap onto the same channels is a
// new stream whose natural standing depth may differ.

/// Window length for depth statistics (blocks): 375 × 128 = 1 s @ 48 k.
const DRIFT_WINDOW_BLOCKS: u64 = 375;
/// EWMA smoothing factor applied to the per-window depth minimum.
const EWMA_ALPHA: f32 = 0.25;
/// Upper hysteresis band above the anchor (frames, ≈ 21 ms @ 48 k): the
/// smoothed depth must rise past `anchor + BAND` before trims arm. The
/// lower trigger is relative (`anchor / 2`) — see the notes above.
const PLAYOUT_BAND_FRAMES: f32 = 1024.0;
/// Correction size (frames, ≈ 1.3 ms @ 48 k) — the length of audio a slip
/// drops or repeats.
const SLIP_FRAMES: usize = 64;
/// Crossfade length over which a splice morphs (frames). Must be ≤ SLIP.
const SLIP_XFADE: usize = 32;
/// Blocks between corrections on the same FIFO (rate limit: a slip every
/// ~10.7 ms → max correction rate ≈ 6000 f/s, far above any real drift).
const SLIP_COOLDOWN_BLOCKS: u32 = 4;
/// A FIFO is "recently fed" if it saw input within this many windows.
/// Insertions are gated on it (never stretch a stopped/disconnected
/// source); trims are not (pre-draining a stall backlog is harmless).
const FEED_RECENCY_WINDOWS: u64 = 2;
/// Consecutive fed windows required before the anchor is set.
const ANCHOR_INIT_WINDOWS: u32 = 2;

/// Per-channel elastic input FIFO with drift bookkeeping.
struct ChannelFifo {
    q: VecDeque<f32>,
    /// Elastic corrections apply (fed via the interleaved PCM path).
    elastic: bool,
    /// Current drift-statistics window id (`block_counter / WINDOW`).
    window_id: u64,
    /// Minimum depth observed in the current window.
    window_min: usize,
    /// Any block starved (queue empty mid-drain) in the current window.
    window_starved: bool,
    /// Smoothed window-min depth; negative = not yet initialized.
    ewma: f32,
    /// Consecutive windows with feed activity (anchor initialization).
    fed_windows: u32,
    /// Established standing depth; `None` until anchored.
    anchor: Option<f32>,
    /// Window id of the last feed.
    last_fed_window: u64,
    /// Blocks remaining before the next correction may fire.
    cooldown: u32,
    /// Diagnostics: total trim / insert corrections applied.
    slips: u64,
    inserts: u64,
}

impl ChannelFifo {
    fn new() -> Self {
        Self {
            q: VecDeque::with_capacity(2048),
            elastic: false,
            window_id: 0,
            window_min: usize::MAX,
            window_starved: false,
            ewma: -1.0,
            fed_windows: 0,
            anchor: None,
            last_fed_window: 0,
            cooldown: 0,
            slips: 0,
            inserts: 0,
        }
    }

    /// Mark fed in `window` and arm elasticity (idempotent).
    fn note_feed(&mut self, window: u64) {
        self.elastic = true;
        self.last_fed_window = window;
    }

    /// Close the statistics window that just ended: fold `window_min`
    /// into the EWMA and initialize the anchor once the feed is stable.
    fn finalize_window(&mut self) {
        let m = self.window_min;
        if self.ewma < 0.0 {
            self.ewma = m as f32;
        } else {
            self.ewma += EWMA_ALPHA * (m as f32 - self.ewma);
        }
        self.window_min = usize::MAX;
        self.window_starved = false;
    }
}

/// Drop `SLIP_FRAMES` from the queue head, crossfading the boundary
/// between the dropped tail and the kept head over `SLIP_XFADE` frames.
/// No-op when the queue is too short to splice safely.
fn slip_trim(q: &mut VecDeque<f32>) {
    let d = SLIP_FRAMES;
    let f = SLIP_XFADE;
    if q.len() <= d + f {
        return;
    }
    for i in 0..f {
        let kept = q[d + i];
        let dropped = q[d - f + i];
        let w = (i + 1) as f32 / (f + 1) as f32;
        q[d + i] = dropped * (1.0 - w) + kept * w;
    }
    q.drain(0..d);
}

/// Append `SLIP_FRAMES` at the queue tail by repeating the tail with a
/// crossfaded splice (time-stretch: net depth grows by SLIP_FRAMES).
/// No-op when the queue is too short to splice safely.
fn slip_insert(q: &mut VecDeque<f32>) {
    let d = SLIP_FRAMES;
    let f = SLIP_XFADE;
    let m = q.len();
    if m < d + f {
        return;
    }
    // Snapshot the pre-splice tail: the morph overwrites samples the
    // appended segment must read in their original form.
    let mut tail = [0.0f32; SLIP_FRAMES + SLIP_XFADE];
    for (i, t) in tail.iter_mut().enumerate() {
        *t = q[m - d - f + i];
    }
    // Morph the last f samples from the original timeline into the
    // delayed-by-d timeline (position p plays original[p - d]).
    for i in 0..f {
        let orig = tail[d + i];
        let delayed = tail[i];
        let w = (i + 1) as f32 / (f + 1) as f32;
        let idx = m - f + i;
        q[idx] = orig * (1.0 - w) + delayed * w;
    }
    // The delayed timeline now continues with original[m - d .. m].
    for j in 0..d {
        q.push_back(tail[f + j]);
    }
}

/// Inline stereo master metering (peak hold + decay, sliding-window RMS,
/// sticky clip flag) replicating the oximedia `Meter` Fast ballistics the
/// binding used previously — with pre-allocated ring buffers so the
/// real-time path never allocates (`Meter::process` built four `Vec`s per
/// block).
struct MasterMeter {
    peak: [f32; 2],
    peak_hold_blocks: [u32; 2],
    hold_blocks: u32,
    rms_buf: Vec<f32>,
    rms_window: usize,
    rms_pos: usize,
    rms_filled: usize,
    rms_sq: [f64; 2],
    clipped: [bool; 2],
    processed_once: bool,
}

impl MasterMeter {
    fn new(sample_rate: u32, block_size: usize) -> Self {
        let hold_blocks = ((sample_rate as f32 / block_size.max(1) as f32).ceil() as u32).max(1);
        let rms_window = (sample_rate as usize / 10).max(1); // Fast = 100 ms
        Self {
            peak: [0.0; 2],
            peak_hold_blocks: [0; 2],
            hold_blocks,
            rms_buf: vec![0.0; rms_window * 2],
            rms_window,
            rms_pos: 0,
            rms_filled: 0,
            rms_sq: [0.0; 2],
            clipped: [false; 2],
            processed_once: false,
        }
    }

    /// Process one block of deinterleaved stereo (`l`/`r`, equal length).
    fn process(&mut self, l: &[f32], r: &[f32]) {
        let n = l.len().min(r.len());
        self.processed_once = true;
        for ch in 0..2 {
            let buf = if ch == 0 { l } else { r };
            let block_peak = buf[..n].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
            if block_peak >= 1.0 {
                self.clipped[ch] = true;
            }
            if block_peak > self.peak[ch] {
                self.peak[ch] = block_peak;
                self.peak_hold_blocks[ch] = self.hold_blocks;
            } else if self.peak_hold_blocks[ch] > 0 {
                self.peak_hold_blocks[ch] -= 1;
            } else {
                self.peak[ch] *= 0.95;
            }
        }
        // Sliding-window RMS per channel over an interleaved ring.
        for i in 0..n {
            for ch in 0..2 {
                let s = if ch == 0 { l[i] } else { r[i] };
                let ring_ch = self.rms_pos + ch * self.rms_window;
                let old = self.rms_buf[ring_ch];
                self.rms_sq[ch] -= f64::from(old * old);
                self.rms_sq[ch] += f64::from(s * s);
                self.rms_buf[ring_ch] = s;
            }
            self.rms_pos += 1;
            if self.rms_pos >= self.rms_window {
                self.rms_pos = 0;
            }
        }
        self.rms_filled = (self.rms_filled + n).min(self.rms_window);
    }

    fn peak_db(&self, ch: usize) -> f32 {
        if !self.processed_once {
            return -f32::INFINITY;
        }
        master_lin_to_db(self.peak[ch])
    }

    fn rms_db(&self, ch: usize) -> f32 {
        if !self.processed_once || self.rms_filled == 0 {
            return -f32::INFINITY;
        }
        let mean = self.rms_sq[ch] / self.rms_filled as f64;
        master_lin_to_db(mean.sqrt() as f32)
    }
}

/// dBFS conversion matching the engine meter's floor (`linear_to_db`).
fn master_lin_to_db(lin: f32) -> f32 {
    if lin <= 0.0 {
        -120.0
    } else {
        20.0 * lin.log10()
    }
}

/// Gain-stage parameters for one channel, snapshotted once per block.
#[derive(Clone, Copy)]
struct MixParams {
    muted: bool,
    input_gain_db: f32,
    phase_inverted: bool,
    fader_gain: f32,
    pan: f32,
    pan_law: PanLaw,
}

struct ChannelDynamics {
    compressor: Option<CompressorEffect>,
    gate: Option<GateEffect>,
    expander: Option<ExpanderEffect>,
}

impl Default for ChannelDynamics {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelDynamics {
    fn new() -> Self {
        Self {
            compressor: None,
            gate: None,
            expander: None,
        }
    }

    fn process(&mut self, samples: &mut [f32]) {
        if let Some(g) = &mut self.gate {
            g.process(samples);
        }
        if let Some(e) = &mut self.expander {
            e.process(samples);
        }
        if let Some(c) = &mut self.compressor {
            c.process(samples);
        }
    }
}

/// WASM binding for the oximedia-mixer audio engine.
///
/// # DSP chain per channel (all wired, all real):
/// 1. Input gain + phase inversion (engine)
/// 2. Gate (if enabled)
/// 3. Compressor (if enabled)
/// 4. Parametric EQ (6-band, always present, bypassable)
/// 5. Fader gain × VCA (engine)
/// 6. Pan (engine)
/// → summed to master bus → OversampledLimiter → output
///
/// Channel direct-out tap: `set_channel_tap()` captures each input
/// channel's MONO signal at the pan-stage input (post input gain/phase,
/// post gate/comp/EQ, post fader) during `process()` for external
/// publishing (WebSRT Nch out).
#[wasm_bindgen]
pub struct MixerWasm {
    engine: oximedia_mixer::AudioMixer,
    buffer_size: usize,
    channel_ids: Vec<Option<ChannelId>>,
    /// Per-channel elastic input FIFOs. PCM arrives in ~23 ms chunks while
    /// process() consumes 128 frames every 2.67 ms — buffers must queue,
    /// not replace (replace = only each chunk's first block ever plays).
    /// See the "Elastic playout buffer" constants above for the drift
    /// reconciliation applied to `elastic` FIFOs.
    channel_inputs: HashMap<u32, ChannelFifo>,
    /// Per-block drained snapshot of the input FIFOs: flat
    /// [channel][frame] (128 channels × buffer_size), zero on starvation.
    /// Filled at the top of process(); passes 1 and 2 read from it.
    block_inputs: Vec<f32>,
    pid_map: HashMap<u16, PidMapping>,
    master_meter: MasterMeter,
    /// Monotonic process() call count (drives drift-statistics windows).
    block_counter: u64,
    /// Diagnostics: channel-blocks that drained an empty FIFO.
    starved_channel_blocks: u64,
    // ── Pre-allocated scratch buffers ──
    master_left: Vec<f32>,
    master_right: Vec<f32>,
    stereo_out: Vec<f32>,
    eq_scratch: Vec<f32>,
    deinterleave_scratch: Vec<Vec<f32>>,
    raw_input: Vec<f32>,
    /// Reused per-block snapshot of input-channel gain params (pass 1).
    pass1_params: Vec<(u32, ChannelId, MixParams)>,
    // ── Per-channel state ──
    soloed_channels: HashSet<u32>,
    user_muted: HashSet<u32>,
    eq_chains: HashMap<u32, EqEffect>,
    dynamics_chains: HashMap<u32, ChannelDynamics>,
    // ── Master limiter (stereo: independent L/R) ──
    limiter_l: OversampledLimiter,
    limiter_r: OversampledLimiter,
    limiter_enabled: bool,
    limiter_ceiling: f32,
    limiter_release_ms: f32,
    // ── Master gain ──
    master_gain: f32,
    // ── Per-channel metering ──
    channel_peak: HashMap<u32, f32>,
    channel_rms: HashMap<u32, f32>,
    // ── Bus routing ──
    bus_map: HashMap<u32, BusId>,
    bus_counter: u32,
    // ── 8 summing buses: each bus sums its 16 slot channels ──
    // Slot index (u32) = 128 + bus*16 + slot, bus 0-7, slot 0-15.
    bus_sources: Vec<Vec<Option<u32>>>,
    bus_gains: Vec<f32>,
    bus_muted: Vec<bool>,
    // Bus accumulators: sum of the bus's slot outputs (pre bus-gain)
    bus_left: Vec<Vec<f32>>,
    bus_right: Vec<Vec<f32>>,
    bus_peak: HashMap<u32, f32>,
    bus_rms: HashMap<u32, f32>,
    // ── Channel direct-out tap (post-chain, post-fader, pre-pan mono) ──
    // 0 = disabled; otherwise channels 0..tap_channels are tapped.
    tap_channels: u32,
    // Interleaved [frame][channel]: index i * N + c is channel c, frame i.
    channel_tap_buf: Vec<f32>,
    // Frames valid in the most recent processed block; 0 = nothing new.
    tap_frames: usize,
    // ── Counters ──
    unmapped_pid_drops: u64,
    sample_rate: u32,
}

#[derive(Clone, Copy, Debug)]
struct PidMapping {
    ch_start: u32,
    channel_count: u32,
    subscribed: bool,
}

#[wasm_bindgen]
impl MixerWasm {
    #[wasm_bindgen(constructor)]
    pub fn new(
        sample_rate: u32,
        buffer_size: u32,
        max_channels: u32,
    ) -> Result<MixerWasm, JsValue> {
        console_error_panic_hook::set_once();

        let bs = buffer_size as usize;
        // Internally support at least 256 channels: 128 inputs (0-127) +
        // 128 bus slots (128-255, slot = 128 + bus*16 + slot).
        let n = (max_channels as usize).max(256);
        let config = MixerConfig {
            sample_rate,
            buffer_size: bs,
            max_channels: n,
            ..Default::default()
        };
        let engine = oximedia_mixer::AudioMixer::new(config);

        Ok(MixerWasm {
            engine,
            buffer_size: bs,
            channel_ids: vec![None; n],
            channel_inputs: HashMap::new(),
            block_inputs: vec![0.0; 128 * bs],
            pid_map: HashMap::new(),
            master_meter: MasterMeter::new(sample_rate, bs),
            block_counter: 0,
            starved_channel_blocks: 0,
            master_left: vec![0.0; bs],
            master_right: vec![0.0; bs],
            stereo_out: vec![0.0; bs * 2],
            eq_scratch: vec![0.0; bs],
            deinterleave_scratch: Vec::new(),
            raw_input: Vec::new(),
            pass1_params: Vec::new(),
            soloed_channels: HashSet::new(),
            user_muted: HashSet::new(),
            eq_chains: HashMap::new(),
            dynamics_chains: HashMap::new(),
            limiter_l: OversampledLimiter::new(-0.3, 50.0, 4, sample_rate as f32),
            limiter_r: OversampledLimiter::new(-0.3, 50.0, 4, sample_rate as f32),
            limiter_enabled: true,
            limiter_ceiling: -0.3,
            limiter_release_ms: 50.0,
            master_gain: 1.0,
            channel_peak: HashMap::new(),
            channel_rms: HashMap::new(),
            bus_map: HashMap::new(),
            bus_counter: 0,
            bus_sources: (0..8).map(|_| vec![None; 16]).collect(),
            bus_gains: vec![1.0; 8],
            bus_muted: vec![false; 8],
            bus_left: (0..8).map(|_| vec![0.0; bs]).collect(),
            bus_right: (0..8).map(|_| vec![0.0; bs]).collect(),
            bus_peak: HashMap::new(),
            bus_rms: HashMap::new(),
            tap_channels: 0,
            channel_tap_buf: Vec::new(),
            tap_frames: 0,
            unmapped_pid_drops: 0,
            sample_rate,
        })
    }

    fn ensure_channel(&mut self, idx: u32) -> Result<ChannelId, JsValue> {
        let i = idx as usize;
        if i >= self.channel_ids.len() {
            return Err(JsValue::from_str(&format!(
                "channel index {idx} out of range (max {})",
                self.channel_ids.len()
            )));
        }
        if self.channel_ids[i].is_none() {
            let id = self
                .engine
                .add_channel(format!("ch{idx}"), ChannelType::Mono, ChannelLayout::Mono)
                .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
            if let Ok(ch) = self.engine.get_channel_mut(id) {
                ch.set_pan_law(PanLaw::Linear);
            }
            self.channel_ids[i] = Some(id);
            self.eq_chains
                .insert(idx, EqEffect::six_band(self.sample_rate));
            self.dynamics_chains.insert(idx, ChannelDynamics::new());
        }
        Ok(self.channel_ids[i].unwrap())
    }

    /// Snapshot the gain-stage parameters for a channel index (input or
    /// slot). Pan-law gains are applied by `process()` directly via
    /// `compute_stereo_pan` — the engine's `process_mix` is bypassed
    /// (it allocates several `Vec`s per channel per block, which is not
    /// survivable on the 2.67 ms real-time budget at high strip counts).
    fn mix_params_for(&self, ch_idx: u32) -> Option<(ChannelId, MixParams)> {
        let id = self.channel_ids.get(ch_idx as usize)?.as_ref().copied()?;
        let ch = self.engine.get_channel(id).ok()?;
        Some((
            id,
            MixParams {
                muted: ch.is_muted(),
                input_gain_db: ch.input().gain_db,
                phase_inverted: ch.is_phase_inverted(),
                fader_gain: ch.gain(),
                pan: ch.pan(),
                pan_law: ch.pan_law(),
            },
        ))
    }

    // ── Input ──────────────────────────────────────────

    pub fn set_channel_input(
        &mut self,
        ch: u32,
        data: &js_sys::Float32Array,
    ) -> Result<(), JsValue> {
        self.ensure_channel(ch)?;
        let len = data.length() as usize;
        self.raw_input.resize(len, 0.0);
        data.copy_to(&mut self.raw_input);
        // Mono per-block feeds (test tones) are not drift-managed: plain FIFO.
        let slot = self
            .channel_inputs
            .entry(ch)
            .or_insert_with(ChannelFifo::new);
        slot.q.extend(self.raw_input.iter().copied());
        let overflow = slot.q.len().saturating_sub(MAX_QUEUE_FRAMES);
        if overflow > 0 {
            slot.q.drain(0..overflow);
        }
        Ok(())
    }

    /// Core interleaved feed (shared by the wasm binding and native
    /// tests): deinterleave `interleaved` into channels
    /// `ch_start..ch_start + num_channels`, appending to each channel's
    /// elastic FIFO.
    pub fn feed_interleaved(
        &mut self,
        ch_start: u32,
        interleaved: &[f32],
        num_channels: u32,
    ) -> Result<(), JsValue> {
        let nc = num_channels as usize;
        if nc == 0 {
            return Err(JsValue::from_str("num_channels must be > 0"));
        }
        let total = interleaved.len();
        if !total.is_multiple_of(nc) {
            return Err(JsValue::from_str(&format!(
                "length {total} not divisible by {nc}"
            )));
        }
        let frames = total / nc;
        if self.deinterleave_scratch.len() < nc {
            self.deinterleave_scratch.resize(nc, Vec::new());
        }
        let window = self.block_counter / DRIFT_WINDOW_BLOCKS;
        for c in 0..nc {
            self.deinterleave_scratch[c].clear();
            self.deinterleave_scratch[c].reserve(frames);
        }
        for f in 0..frames {
            for c in 0..nc {
                self.deinterleave_scratch[c].push(interleaved[f * nc + c]);
            }
        }
        for c in 0..nc {
            let ch = ch_start + c as u32;
            self.ensure_channel(ch)?;
            let slot = self
                .channel_inputs
                .entry(ch)
                .or_insert_with(ChannelFifo::new);
            slot.note_feed(window);
            slot.q.extend(self.deinterleave_scratch[c].iter().copied());
            let overflow = slot.q.len().saturating_sub(MAX_QUEUE_FRAMES);
            if overflow > 0 {
                slot.q.drain(0..overflow);
            }
        }
        Ok(())
    }

    pub fn set_channel_input_interleaved(
        &mut self,
        ch_start: u32,
        data: &js_sys::Float32Array,
        num_channels: u32,
    ) -> Result<(), JsValue> {
        let len = data.length() as usize;
        self.raw_input.resize(len, 0.0);
        data.copy_to(&mut self.raw_input);
        let raw = std::mem::take(&mut self.raw_input);
        let r = self.feed_interleaved(ch_start, &raw, num_channels);
        self.raw_input = raw;
        r
    }

    // ── PID mapping ────────────────────────────────────

    pub fn map_pid(&mut self, pid: u16, ch_start: u32, channel_count: u32) -> Result<(), JsValue> {
        for i in 0..channel_count {
            self.ensure_channel(ch_start + i)?;
        }
        // A (re)map is a new stream: drop the elastic anchors on its
        // channels so the standing depth re-anchors at the new source's
        // natural level instead of correcting toward the old stream's.
        for i in 0..channel_count {
            if let Some(fifo) = self.channel_inputs.get_mut(&(ch_start + i)) {
                fifo.anchor = None;
                fifo.ewma = -1.0;
                fifo.fed_windows = 0;
                fifo.window_min = usize::MAX;
                fifo.window_starved = false;
            }
        }
        self.pid_map.insert(
            pid,
            PidMapping {
                ch_start,
                channel_count,
                subscribed: true,
            },
        );
        Ok(())
    }
    pub fn unmap_pid(&mut self, pid: u16) {
        self.pid_map.remove(&pid);
    }
    pub fn pid_channel(&self, pid: u16) -> i32 {
        self.pid_map
            .get(&pid)
            .map(|m| m.ch_start as i32)
            .unwrap_or(-1)
    }
    pub fn pid_channel_count(&self, pid: u16) -> u32 {
        self.pid_map.get(&pid).map(|m| m.channel_count).unwrap_or(0)
    }
    pub fn subscribe_pid(&mut self, pid: u16) {
        if let Some(m) = self.pid_map.get_mut(&pid) {
            m.subscribed = true;
            for i in 0..m.channel_count {
                if let Some(&Some(id)) = self.channel_ids.get((m.ch_start + i) as usize) {
                    if let Ok(ch) = self.engine.get_channel_mut(id) {
                        ch.set_muted(false);
                    }
                }
            }
        }
    }
    pub fn unsubscribe_pid(&mut self, pid: u16) {
        if let Some(m) = self.pid_map.get_mut(&pid) {
            m.subscribed = false;
            for i in 0..m.channel_count {
                if let Some(&Some(id)) = self.channel_ids.get((m.ch_start + i) as usize) {
                    if let Ok(ch) = self.engine.get_channel_mut(id) {
                        ch.set_muted(true);
                    }
                }
            }
        }
    }

    pub fn feed_pcm(&mut self, pid: u16, data: &js_sys::Float32Array) -> Result<(), JsValue> {
        let Some(mapping) = self.pid_map.get(&pid).copied() else {
            self.unmapped_pid_drops += 1;
            return Ok(());
        };
        if !mapping.subscribed {
            return Ok(());
        }
        self.set_channel_input_interleaved(mapping.ch_start, data, mapping.channel_count)
    }

    /// Count of PCM packets dropped due to unmapped PID (for diagnostics).
    pub fn unmapped_pid_count(&self) -> u64 {
        self.unmapped_pid_drops
    }

    // ── Channel controls ───────────────────────────────

    pub fn set_channel_gain(&mut self, ch: u32, gain: f32) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        self.engine
            .set_channel_gain(id, gain)
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }
    pub fn set_channel_pan(&mut self, ch: u32, pan: f32) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        self.engine
            .set_channel_pan(id, pan)
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }
    pub fn set_channel_mute(&mut self, ch: u32, muted: bool) -> Result<(), JsValue> {
        let _ = self.ensure_channel(ch)?;
        if muted {
            self.user_muted.insert(ch);
        } else {
            self.user_muted.remove(&ch);
        }
        // Apply: muted if user-muted OR (something is soloed and this isn't)
        let effective = muted || (self.solo_active() && !self.soloed_channels.contains(&ch));
        self.set_engine_mute(ch, effective);
        Ok(())
    }
    pub fn set_channel_solo(&mut self, ch: u32, soloed: bool) -> Result<(), JsValue> {
        let _ = self.ensure_channel(ch)?;
        if soloed {
            self.soloed_channels.insert(ch);
        } else {
            self.soloed_channels.remove(&ch);
        }
        // Re-evaluate all channels' effective mute state
        let ids: Vec<(u32, ChannelId)> = self
            .channel_ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| id.map(|id| (i as u32, id)))
            .collect();
        for (i, id) in ids {
            let effective = self.user_muted.contains(&i)
                || (self.solo_active() && !self.soloed_channels.contains(&i));
            if let Ok(ch) = self.engine.get_channel_mut(id) {
                ch.set_muted(effective);
            }
        }
        Ok(())
    }

    fn solo_active(&self) -> bool {
        !self.soloed_channels.is_empty()
    }

    fn set_engine_mute(&mut self, ch: u32, muted: bool) {
        if let Some(&Some(id)) = self.channel_ids.get(ch as usize) {
            if let Ok(channel) = self.engine.get_channel_mut(id) {
                channel.set_muted(muted);
            }
        }
    }

    // ── EQ controls ────────────────────────────────────

    pub fn set_eq_band_gain(&mut self, ch: u32, band: usize, gain_db: f32) -> Result<(), JsValue> {
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            if band < eq.inner().bands.len() {
                eq.inner_mut().bands[band].set_gain_db(gain_db as f64);
                eq.inner_mut().bands[band].update_coefficients();
            }
        }
        Ok(())
    }
    pub fn set_eq_band_freq(&mut self, ch: u32, band: usize, freq_hz: f32) -> Result<(), JsValue> {
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            if band < eq.inner().bands.len() {
                eq.inner_mut().bands[band].set_frequency(freq_hz as f64);
                eq.inner_mut().bands[band].update_coefficients();
            }
        }
        Ok(())
    }
    pub fn set_eq_band_q(&mut self, ch: u32, band: usize, q: f32) -> Result<(), JsValue> {
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            if band < eq.inner().bands.len() {
                eq.inner_mut().bands[band].set_q(q as f64);
                eq.inner_mut().bands[band].update_coefficients();
            }
        }
        Ok(())
    }
    pub fn set_eq_bypass(&mut self, ch: u32, bypassed: bool) -> Result<(), JsValue> {
        self.ensure_channel(ch)?;
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            eq.set_bypassed(bypassed);
        }
        Ok(())
    }

    // ── Dynamics controls ──────────────────────────────

    /// Enable compressor on a channel with broadcast defaults (-12 dB threshold, 3:1 ratio).
    pub fn enable_compressor(&mut self, ch: u32) -> Result<(), JsValue> {
        let _ = self.ensure_channel(ch)?;
        self.dynamics_chains.entry(ch).or_default().compressor =
            Some(CompressorEffect::broadcast(self.sample_rate));
        Ok(())
    }
    pub fn disable_compressor(&mut self, ch: u32) {
        if let Some(d) = self.dynamics_chains.get_mut(&ch) {
            d.compressor = None;
        }
    }
    pub fn enable_gate(&mut self, ch: u32) -> Result<(), JsValue> {
        let _ = self.ensure_channel(ch)?;
        self.dynamics_chains.entry(ch).or_default().gate =
            Some(GateEffect::denoise(self.sample_rate));
        Ok(())
    }
    pub fn disable_gate(&mut self, ch: u32) {
        if let Some(d) = self.dynamics_chains.get_mut(&ch) {
            d.gate = None;
        }
    }

    pub fn enable_expander(&mut self, ch: u32) -> Result<(), JsValue> {
        let _ = self.ensure_channel(ch)?;
        self.dynamics_chains.entry(ch).or_default().expander =
            Some(ExpanderEffect::gentle(self.sample_rate));
        Ok(())
    }
    pub fn disable_expander(&mut self, ch: u32) {
        if let Some(d) = self.dynamics_chains.get_mut(&ch) {
            d.expander = None;
        }
    }

    pub fn set_comp_param(&mut self, ch: u32, param: u32, value: f32) -> Result<(), JsValue> {
        if let Some(d) = self.dynamics_chains.get_mut(&ch) {
            if let Some(c) = &mut d.compressor {
                c.update_config(|cfg| match param {
                    0 => cfg.threshold_db = value,
                    1 => cfg.ratio = value,
                    2 => cfg.attack_ms = value,
                    3 => cfg.release_ms = value,
                    4 => cfg.makeup_gain_db = value,
                    5 => cfg.knee_db = value,
                    _ => (),
                });
            }
        }
        Ok(())
    }

    pub fn set_gate_param(&mut self, ch: u32, param: u32, value: f32) -> Result<(), JsValue> {
        if let Some(d) = self.dynamics_chains.get_mut(&ch) {
            if let Some(g) = &mut d.gate {
                g.update_config(|cfg| match param {
                    0 => cfg.threshold_db = value,
                    1 => cfg.hysteresis_db = value,
                    2 => cfg.attack_ms = value,
                    3 => cfg.release_ms = value,
                    4 => cfg.hold_ms = value,
                    _ => (),
                });
            }
        }
        Ok(())
    }

    pub fn set_expander_param(&mut self, ch: u32, param: u32, value: f32) -> Result<(), JsValue> {
        if let Some(d) = self.dynamics_chains.get_mut(&ch) {
            if let Some(e) = &mut d.expander {
                e.update_config(|cfg| match param {
                    0 => cfg.threshold_db = value,
                    1 => cfg.ratio = value,
                    2 => cfg.attack_ms = value,
                    3 => cfg.release_ms = value,
                    _ => (),
                });
            }
        }
        Ok(())
    }

    // ── Channel-level controls ─────────────────────────

    pub fn set_channel_input_gain(&mut self, ch: u32, gain_db: f32) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        if let Ok(c) = self.engine.get_channel_mut(id) {
            c.input_mut().gain_db = gain_db;
        }
        Ok(())
    }

    pub fn set_channel_phase(&mut self, ch: u32, inverted: bool) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        if let Ok(c) = self.engine.get_channel_mut(id) {
            c.set_phase_inverted(inverted);
        }
        Ok(())
    }

    pub fn set_channel_pan_law(&mut self, ch: u32, law: u32) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        let pan_law = match law {
            0 => PanLaw::Linear,
            1 => PanLaw::Minus3dB,
            2 => PanLaw::Minus4Dot5dB,
            3 => PanLaw::Minus6dB,
            _ => return Err(JsValue::from_str("invalid pan law")),
        };
        if let Ok(c) = self.engine.get_channel_mut(id) {
            c.set_pan_law(pan_law);
        }
        Ok(())
    }

    pub fn set_channel_name(&mut self, ch: u32, name: String) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        if let Ok(c) = self.engine.get_channel_mut(id) {
            c.set_name(name);
        }
        Ok(())
    }

    // ── Per-channel metering ───────────────────────────

    pub fn channel_peak_db(&self, ch: u32) -> f32 {
        self.channel_peak
            .get(&ch)
            .copied()
            .unwrap_or(-f32::INFINITY)
    }
    pub fn channel_rms_db(&self, ch: u32) -> f32 {
        self.channel_rms.get(&ch).copied().unwrap_or(-f32::INFINITY)
    }
    pub fn channel_meters_json(&self) -> String {
        let mut s = String::from("[");
        let mut first = true;
        for (&ch, &peak) in &self.channel_peak {
            if !first {
                s.push(',');
            }
            first = false;
            let rms = self.channel_rms.get(&ch).copied().unwrap_or(-200.0);
            s.push_str(&format!(
                "{{\"ch\":{ch},\"peak\":{peak:.1},\"rms\":{rms:.1}}}"
            ));
        }
        s.push(']');
        s
    }

    // ── Master controls ────────────────────────────────

    pub fn set_master_gain(&mut self, gain: f32) {
        self.master_gain = gain.clamp(0.0, 2.0);
    }

    pub fn set_limiter_ceiling(&mut self, ceiling_db: f32) {
        self.limiter_ceiling = ceiling_db;
        let sr = self.sample_rate as f32;
        self.limiter_l = OversampledLimiter::new(ceiling_db, self.limiter_release_ms, 4, sr);
        self.limiter_r = OversampledLimiter::new(ceiling_db, self.limiter_release_ms, 4, sr);
    }
    pub fn set_limiter_release(&mut self, release_ms: f32) {
        self.limiter_release_ms = release_ms;
        let sr = self.sample_rate as f32;
        self.limiter_l = OversampledLimiter::new(self.limiter_ceiling, release_ms, 4, sr);
        self.limiter_r = OversampledLimiter::new(self.limiter_ceiling, release_ms, 4, sr);
    }

    // ── Bus routing ────────────────────────────────────

    pub fn add_bus(&mut self, name: String, bus_type: u32) -> Result<u32, JsValue> {
        let bt = match bus_type {
            0 => BusType::Group,
            1 => BusType::Auxiliary,
            2 => BusType::Matrix,
            _ => return Err(JsValue::from_str("invalid bus type")),
        };
        let bus_id = self
            .engine
            .add_bus(name, bt, ChannelLayout::Stereo)
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
        let js_id = self.bus_counter;
        self.bus_counter += 1;
        self.bus_map.insert(js_id, bus_id);
        Ok(js_id)
    }

    pub fn set_aux_send(
        &mut self,
        ch: u32,
        _send_idx: u32,
        bus_id: u32,
        level: f32,
        pre_fader: bool,
    ) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        let Some(&bid) = self.bus_map.get(&bus_id) else {
            return Err(JsValue::from_str("unknown bus"));
        };
        self.engine
            .add_aux_send(id, bid, level, pre_fader)
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    pub fn remove_aux_send(&mut self, ch: u32, send_idx: u32) -> Result<(), JsValue> {
        let Some(id) = self.channel_ids.get(ch as usize).and_then(|o| *o) else {
            return Ok(());
        };
        if let Some(sends) = self.engine.engine_mut().channel_sends.get_mut(&id) {
            if (send_idx as usize) < sends.len() {
                sends.remove(send_idx as usize);
            }
        }
        Ok(())
    }

    // ── Bus mixing (8 buses × 16 full-channel-strip slots) ──

    pub fn set_bus_source(&mut self, bus: u32, slot: u32, ch: u32) -> Result<(), JsValue> {
        if bus >= 8 {
            return Err(JsValue::from_str("bus index out of range (max 8)"));
        }
        if slot >= 16 {
            return Err(JsValue::from_str("slot out of range (max 16)"));
        }
        self.bus_sources[bus as usize][slot as usize] = Some(ch);
        // Lazily create the slot's engine channel / EQ / dynamics (idx 128-255)
        self.ensure_channel(128 + bus * 16 + slot)?;
        Ok(())
    }

    pub fn clear_bus_source(&mut self, bus: u32, slot: u32) {
        if (bus as usize) < 8 && (slot as usize) < 16 {
            self.bus_sources[bus as usize][slot as usize] = None;
        }
    }

    pub fn set_bus_gain(&mut self, bus: u32, gain: f32) {
        if (bus as usize) < 8 {
            self.bus_gains[bus as usize] = gain.clamp(0.0, 2.0);
        }
    }
    pub fn set_bus_mute(&mut self, bus: u32, muted: bool) {
        if (bus as usize) < 8 {
            self.bus_muted[bus as usize] = muted;
        }
    }

    // ── Bus metering ───────────────────────────────────

    pub fn bus_peak_db(&self, bus: u32) -> f32 {
        self.bus_peak.get(&bus).copied().unwrap_or(-f32::INFINITY)
    }
    pub fn bus_rms_db(&self, bus: u32) -> f32 {
        self.bus_rms.get(&bus).copied().unwrap_or(-f32::INFINITY)
    }
    pub fn bus_meters_json(&self) -> String {
        let mut s = String::from("[");
        for i in 0..8u32 {
            if i > 0 {
                s.push(',');
            }
            let peak = self.bus_peak.get(&i).copied().unwrap_or(-200.0);
            let rms = self.bus_rms.get(&i).copied().unwrap_or(-200.0);
            s.push_str(&format!(
                "{{\"bus\":{i},\"peak\":{peak:.1},\"rms\":{rms:.1}}}"
            ));
        }
        s.push(']');
        s
    }

    // ── Master limiter ─────────────────────────────────

    pub fn set_limiter_enabled(&mut self, enabled: bool) {
        self.limiter_enabled = enabled;
    }
    pub fn limiter_gain_reduction_db(&self) -> f32 {
        self.limiter_l.gain_reduction_db()
    }

    // ── Channel direct-out tap ────────────────────────

    /// Enable the per-channel direct-out tap: during `process()`, channels
    /// `0..channels` are captured MONO at the pan-stage input — post input
    /// gain and phase inversion, post gate/comp/EQ chain, post fader gain,
    /// PRE pan (pan is a stereo-bus concept; direct outs are mono per
    /// channel). Drain the captured block with `take_channel_tap()`.
    ///
    /// * `channels = 0` (or never calling this) disables the tap. Clamped
    ///   to 0..=128; slot channels (128-255, bus slots) are never tapped.
    /// * Idempotent; re-enabling with a different N resizes the accumulator.
    /// * Muted channels tap SILENCE, and solo gating applies exactly as it
    ///   does to the mix: when any channel is soloed, non-soloed channels
    ///   are silent in the tap too (a muted/solo-gated channel produces no
    ///   output anywhere, direct out included).
    /// * Channels with no input data contribute zeros.
    /// * `process()` output is identical whether the tap is on or off.
    pub fn set_channel_tap(&mut self, channels: u32) {
        let n = channels.min(128) as usize;
        self.tap_channels = n as u32;
        self.channel_tap_buf.clear();
        self.channel_tap_buf.resize(n * self.buffer_size, 0.0);
        self.tap_frames = 0;
    }

    /// Drain the channel direct-out tap captured by the last `process()`
    /// call: one block of `bs` frames × N channels, interleaved per frame
    /// (frame i: ch0[i], ch1[i], ..., chN-1[i]). Empties the accumulator —
    /// call once per processed block.
    ///
    /// Returns an empty Float32Array when the tap is disabled or no block
    /// has been processed since the last take. If two `process()` calls
    /// happen without a take in between, only the LATEST block is kept.
    pub fn take_channel_tap(&mut self) -> js_sys::Float32Array {
        if self.tap_channels == 0 || self.tap_frames == 0 {
            return js_sys::Float32Array::new_with_length(0);
        }
        let len = self.tap_frames * self.tap_channels as usize;
        let out = js_sys::Float32Array::new_with_length(len as u32);
        out.copy_from(&self.channel_tap_buf[..len]);
        self.tap_frames = 0;
        out
    }

    // ── Processing ─────────────────────────────────────

    /// Process one block and return the interleaved stereo output.
    ///
    /// Thin wrapper over [`MixerWasm::process_block`] (the pure-Rust core,
    /// also used by native tests) that copies the result into a fresh
    /// `Float32Array` for JS.
    pub fn process(&mut self, block_size: u32) -> Result<js_sys::Float32Array, JsValue> {
        let out = self.process_block(block_size)?;
        let js = js_sys::Float32Array::new_with_length(out.len() as u32);
        js.copy_from(out);
        Ok(js)
    }
}

impl MixerWasm {
    /// Process one block through the full console (pure Rust, no JS
    /// interop — callable from native tests). Returns the interleaved
    /// stereo output slice (`block_size × 2` frames), valid until the
    /// next call.
    ///
    /// Zero-allocation in the steady state: every buffer is pre-allocated
    /// and reused (regression-tested in `tests/alloc_test.rs`).
    pub fn process_block(&mut self, block_size: u32) -> Result<&[f32], JsValue> {
        let bs = self.buffer_size.min(block_size as usize).max(1);

        // Clear master bus
        for i in 0..self.buffer_size {
            self.master_left[i] = 0.0;
            self.master_right[i] = 0.0;
        }

        // Clear bus accumulators
        for b in 0..8 {
            for s in 0..self.buffer_size {
                self.bus_left[b][s] = 0.0;
                self.bus_right[b][s] = 0.0;
            }
        }

        // ── Channel direct-out tap: start a fresh block ──
        // Zeroed here so muted / solo-gated / input-less channels surface
        // as silence; active channels overwrite their slots in pass 1.
        if self.tap_channels > 0 {
            self.channel_tap_buf[..bs * self.tap_channels as usize].fill(0.0);
            self.tap_frames = bs;
        }

        // ── Drain input FIFOs into the per-block snapshot ──
        // bs frames per input channel (0-127); starved channels surface as
        // zeros (clean dropout). Draining regardless of mute/solo keeps the
        // FIFOs rate-balanced so nothing accumulates while unheard.
        // Elastic FIFOs additionally run drift reconciliation (see the
        // playout-buffer notes at the top): window statistics fold into an
        // EWMA, and a rate-limited crossfaded slip fires when the smoothed
        // depth leaves the anchor's hysteresis band.
        self.block_counter += 1;
        let window = self.block_counter / DRIFT_WINDOW_BLOCKS;
        self.block_inputs[..128 * bs].fill(0.0);
        for (&ch_idx, fifo) in self.channel_inputs.iter_mut() {
            if ch_idx >= 128 {
                continue;
            }
            let base = ch_idx as usize * bs;
            let mut starved = false;
            for d in self.block_inputs[base..base + bs].iter_mut() {
                if let Some(s) = fifo.q.pop_front() {
                    *d = s;
                } else {
                    starved = true;
                    break;
                }
            }
            if !fifo.elastic {
                continue;
            }
            if starved {
                self.starved_channel_blocks += 1;
                fifo.window_starved = true;
            }
            // Window rollover: fold the finished window's minimum into the
            // EWMA and, once feeding steadily, establish the anchor.
            if fifo.window_id != window {
                fifo.finalize_window();
                fifo.window_id = window;
                let fed_recent =
                    window.saturating_sub(fifo.last_fed_window) <= FEED_RECENCY_WINDOWS;
                fifo.fed_windows = if fed_recent { fifo.fed_windows + 1 } else { 0 };
                if fifo.anchor.is_none() && fifo.fed_windows >= ANCHOR_INIT_WINDOWS {
                    fifo.anchor = Some(fifo.ewma);
                }
            }
            let depth = fifo.q.len();
            if depth < fifo.window_min {
                fifo.window_min = depth;
            }
            // Rate-limited correction. Channels of a PID mapping share
            // feed/drain history, so every channel of the mapping makes
            // the same decision on the same frame boundary — inter-channel
            // phase stays intact.
            if fifo.cooldown > 0 {
                fifo.cooldown -= 1;
                continue;
            }
            let Some(anchor) = fifo.anchor else { continue };
            let fed_recent = window.saturating_sub(fifo.last_fed_window) <= FEED_RECENCY_WINDOWS;
            let settle_to = anchor + PLAYOUT_BAND_FRAMES * 0.5;
            let anchor_half = anchor * 0.5;
            if fifo.ewma > anchor + PLAYOUT_BAND_FRAMES
                && depth as f32 > settle_to
                && depth > SLIP_FRAMES + SLIP_XFADE
            {
                // Sustained net-fast delivery: standing latency creeps up —
                // trim the head back toward the anchor.
                slip_trim(&mut fifo.q);
                fifo.slips += 1;
                fifo.cooldown = SLIP_COOLDOWN_BLOCKS;
            } else if fed_recent
                && depth >= SLIP_FRAMES + SLIP_XFADE
                && (fifo.window_starved || fifo.ewma < anchor_half)
                && (depth as f32) < anchor_half + (2.0 * SLIP_FRAMES as f32)
            {
                // Sustained net-slow delivery (or live starvation): the
                // queue would run dry — stretch the tail. The depth cap
                // just above the trigger bounds the burst against a
                // between-windows stale statistic.
                slip_insert(&mut fifo.q);
                fifo.inserts += 1;
                fifo.cooldown = SLIP_COOLDOWN_BLOCKS;
            }
        }

        // Snapshot input-channel gain params into the reused scratch vec.
        // Pass 1 covers input channels only (indices 0-127); slot channels
        // (128-255) are handled in pass 2 below.
        self.pass1_params.clear();
        for &ch_idx in self.channel_inputs.keys() {
            if ch_idx < 128 {
                if let Some((id, p)) = self.mix_params_for(ch_idx) {
                    self.pass1_params.push((ch_idx, id, p));
                }
            }
        }

        // ── Pass 1: input channels ──
        for &(ch_idx, id, params) in &self.pass1_params {
            if params.muted {
                continue;
            }

            // This block's drained input (zeros on starvation)
            let base = ch_idx as usize * bs;
            let samples = &self.block_inputs[base..base + bs];

            // ── Gate → Compressor → EQ (in-place on scratch) ──
            // Copy to pre-allocated scratch (no per-channel Vec alloc)
            self.eq_scratch[..bs].copy_from_slice(samples);

            // Gate
            if let Some(d) = self.dynamics_chains.get_mut(&ch_idx) {
                d.process(&mut self.eq_scratch[..bs]);
            }

            // EQ (always process unless explicitly bypassed)
            if let Some(eq) = self.eq_chains.get_mut(&ch_idx) {
                if !eq.is_bypassed() {
                    eq.process(&mut self.eq_scratch[..bs]);
                }
            }

            // Per-channel metering (post-dynamics, post-EQ, pre-fader)
            let peak = self.eq_scratch[..bs]
                .iter()
                .map(|s| s.abs())
                .fold(0.0f32, f32::max);
            let sq_sum: f32 = self.eq_scratch[..bs].iter().map(|s| s * s).sum();
            let rms = (sq_sum / bs as f32).sqrt();
            self.channel_peak.insert(
                ch_idx,
                if peak > 1e-10 {
                    20.0 * peak.log10()
                } else {
                    -200.0
                },
            );
            self.channel_rms.insert(
                ch_idx,
                if rms > 1e-10 {
                    20.0 * rms.log10()
                } else {
                    -200.0
                },
            );

            // ── Direct-out tap: capture the pan-stage input ──
            // eq_scratch holds the post gate/comp/EQ signal; apply the same
            // input gain × phase × fader × VCA the engine would apply, but
            // NOT pan → the classic post-fader mono direct out.
            // (Muted/solo-gated channels `continue`d above, so their slots
            // stay zeroed.)
            let (lg, rg) = compute_stereo_pan(params.pan, &params.pan_law);
            let phase: f32 = if params.phase_inverted { -1.0 } else { 1.0 };
            let vca = self.engine.engine().vca_gain_for_channel(id);
            let g = db_to_linear(params.input_gain_db) * phase * params.fader_gain * vca;
            if self.tap_channels > ch_idx {
                let n = self.tap_channels as usize;
                for i in 0..bs {
                    self.channel_tap_buf[i * n + ch_idx as usize] = self.eq_scratch[i] * g;
                }
            }

            // Gain stage: input gain × phase × fader × VCA, then pan-law
            // gains, accumulated straight into the master bus. This is the
            // same math the engine applies in process_mix (compute_stereo_pan
            // is the engine's own function) minus its per-channel Vec
            // allocations — the RT budget does not survive those at high
            // strip counts.
            //
            // Direct to master — always, unconditionally. Bus slots tap the
            // RAW input buffer in parallel (pass 2), so bus assignment never
            // diverts or silences the direct input path.
            let gl = g * lg;
            let gr = g * rg;
            for i in 0..bs {
                self.master_left[i] += self.eq_scratch[i] * gl;
                self.master_right[i] += self.eq_scratch[i] * gr;
            }
        }

        // ── Pass 2: bus slots (indices 128-255) are full channel strips ──
        // Each slot taps the RAW mono input buffer of its assigned source
        // channel in parallel with the input's own strip: the input channel's
        // mute/fader/EQ/dynamics have zero effect on the bus path. The slot
        // runs its own complete chain (dynamics → EQ → fader/pan via the
        // engine) and feeds its bus accumulator.
        for bus_idx in 0..8u32 {
            for slot in 0..16u32 {
                let slot_idx = 128 + bus_idx * 16 + slot;

                // Which raw input does this slot tap?
                let Some(src) = self.bus_sources[bus_idx as usize][slot as usize] else {
                    continue;
                };
                if src >= 128 {
                    continue; // only input channels (0-127) have drained blocks
                }

                // Skip muted/missing slot channels (defensive: set_bus_source
                // lazily creates the engine channel with defaults).
                let Some((id, params)) = self.mix_params_for(slot_idx) else {
                    continue;
                };
                if params.muted {
                    continue;
                }

                // The source's drained block for this process() call — the
                // same frames pass 1 consumed (zero-padded on starvation).
                let base = src as usize * bs;
                self.eq_scratch[..bs].copy_from_slice(&self.block_inputs[base..base + bs]);

                // Slot's own dynamics → EQ (in-place on scratch)
                if let Some(d) = self.dynamics_chains.get_mut(&slot_idx) {
                    d.process(&mut self.eq_scratch[..bs]);
                }
                if let Some(eq) = self.eq_chains.get_mut(&slot_idx) {
                    if !eq.is_bypassed() {
                        eq.process(&mut self.eq_scratch[..bs]);
                    }
                }

                // Slot metering (post-dynamics, post-EQ, pre-fader)
                let peak = self.eq_scratch[..bs]
                    .iter()
                    .map(|s| s.abs())
                    .fold(0.0f32, f32::max);
                let sq_sum: f32 = self.eq_scratch[..bs].iter().map(|s| s * s).sum();
                let rms = (sq_sum / bs as f32).sqrt();
                self.channel_peak.insert(
                    slot_idx,
                    if peak > 1e-10 {
                        20.0 * peak.log10()
                    } else {
                        -200.0
                    },
                );
                self.channel_rms.insert(
                    slot_idx,
                    if rms > 1e-10 {
                        20.0 * rms.log10()
                    } else {
                        -200.0
                    },
                );

                // Gain stage on the mono tap → stereo result into the bus
                // accumulator (same direct math as pass 1 — see note there).
                let (lg, rg) = compute_stereo_pan(params.pan, &params.pan_law);
                let phase: f32 = if params.phase_inverted { -1.0 } else { 1.0 };
                let vca = self.engine.engine().vca_gain_for_channel(id);
                let g = db_to_linear(params.input_gain_db) * phase * params.fader_gain * vca;
                let gl = g * lg;
                let gr = g * rg;
                let bi = bus_idx as usize;
                for i in 0..bs {
                    self.bus_left[bi][i] += self.eq_scratch[i] * gl;
                    self.bus_right[bi][i] += self.eq_scratch[i] * gr;
                }
            }
        }

        // ── Pass 3: buses to master ──
        for bus_idx in 0..8u32 {
            if self.bus_muted[bus_idx as usize] {
                continue;
            }

            // Bus metering on the accumulator (sum of slot outputs, pre-gain)
            let bl = &self.bus_left[bus_idx as usize];
            let br = &self.bus_right[bus_idx as usize];
            let peak = bl[..bs]
                .iter()
                .chain(br[..bs].iter())
                .map(|s| s.abs())
                .fold(0.0f32, f32::max);
            let sq: f32 = bl[..bs].iter().chain(br[..bs].iter()).map(|s| s * s).sum();
            let rms = (sq / (bs as f32 * 2.0)).sqrt();
            self.bus_peak.insert(
                bus_idx,
                if peak > 1e-10 {
                    20.0 * peak.log10()
                } else {
                    -200.0
                },
            );
            self.bus_rms.insert(
                bus_idx,
                if rms > 1e-10 {
                    20.0 * rms.log10()
                } else {
                    -200.0
                },
            );

            // Apply bus gain and sum to master
            let bg = self.bus_gains[bus_idx as usize];
            for i in 0..bs {
                self.master_left[i] += self.bus_left[bus_idx as usize][i] * bg;
                self.master_right[i] += self.bus_right[bus_idx as usize][i] * bg;
            }
        }

        // ── Master gain ──
        if self.master_gain != 1.0 {
            for i in 0..bs {
                self.master_left[i] *= self.master_gain;
                self.master_right[i] *= self.master_gain;
            }
        }

        // ── Master limiter (brick-wall) ──
        if self.limiter_enabled {
            for i in 0..bs {
                self.master_left[i] = self.limiter_l.process_sample(self.master_left[i]);
                self.master_right[i] = self.limiter_r.process_sample(self.master_right[i]);
            }
        }

        // Interleave to stereo output
        for i in 0..bs {
            self.stereo_out[i * 2] = self.master_left[i];
            self.stereo_out[i * 2 + 1] = self.master_right[i];
        }

        self.master_meter
            .process(&self.master_left[..bs], &self.master_right[..bs]);

        Ok(&self.stereo_out[..bs * 2])
    }
}

// ── Metering + diagnostics (exported) ────────────────────────────────
#[wasm_bindgen]
impl MixerWasm {
    pub fn master_peak_db_l(&self) -> f32 {
        self.master_meter.peak_db(0)
    }
    pub fn master_peak_db_r(&self) -> f32 {
        self.master_meter.peak_db(1)
    }
    pub fn master_rms_db_l(&self) -> f32 {
        self.master_meter.rms_db(0)
    }
    pub fn master_rms_db_r(&self) -> f32 {
        self.master_meter.rms_db(1)
    }
    pub fn master_clipping(&self) -> bool {
        self.master_meter.clipped[0] || self.master_meter.clipped[1]
    }

    // ── Elastic playout diagnostics ────────────────────
    // Live counters for the drift reconciliation (surfaced by the worklet
    // in its meter messages; see web/worklet-template.js).

    /// Total trim slips applied (net-fast delivery corrections).
    pub fn elastic_slips(&self) -> u64 {
        self.channel_inputs.values().map(|f| f.slips).sum()
    }
    /// Total insert slips applied (net-slow delivery corrections).
    pub fn elastic_inserts(&self) -> u64 {
        self.channel_inputs.values().map(|f| f.inserts).sum()
    }
    /// Channel-blocks that drained an empty FIFO (starvation events).
    pub fn starved_blocks(&self) -> u64 {
        self.starved_channel_blocks
    }
    /// Deepest current FIFO depth across all channels (frames).
    pub fn fifo_max_depth(&self) -> u32 {
        self.channel_inputs
            .values()
            .map(|f| f.q.len())
            .max()
            .unwrap_or(0) as u32
    }
}
