use std::collections::{HashMap, HashSet};
use wasm_bindgen::prelude::*;

use oximedia_audio::ChannelLayout;
use oximedia_mixer::effects_chain::AudioEffect;
use oximedia_mixer::oversampled_limiter::OversampledLimiter;
use oximedia_mixer::{
    channel::{db_to_linear, ChannelType, PanLaw},
    ChannelId, ChannelProcessParams, MixerConfig, PanLawType,
};

// Module map: `fifo` (elastic input FIFOs + drift slips), `meter`
// (master metering), `scene` (console scenes + interpolation helpers),
// `console_json` (UI-state JSON). This file holds the MixerWasm binding.
pub mod console_json;
pub mod effects;
pub mod fifo;
mod meter;
mod scene;

use console_json::{
    bus_params_json, comp_enable_defaults, expander_enable_defaults, gate_enable_defaults,
    json_escape,
};
use fifo::{
    slip_insert, slip_trim, ChannelFifo, ANCHOR_INIT_WINDOWS, DRIFT_WINDOW_BLOCKS,
    FEED_RECENCY_WINDOWS, MAX_QUEUE_FRAMES, PLAYOUT_BAND_FRAMES, SLIP_COOLDOWN_BLOCKS, SLIP_FRAMES,
    SLIP_XFADE,
};
use meter::MasterMeter;
use scene::{lerp_bool, lerp_gain_db, lerp_lin, lerp_log2, pan_law_to_wire, scene_err, FadeState};
pub use scene::{
    BusScene, CompScene, ConsoleScene, EqBandScene, ExpanderScene, GateScene, StripScene,
};

use effects::{CompressorEffect, EqEffect, ExpanderEffect, GateEffect};

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

/// Map the channel module's `PanLaw` onto the processing engine's enum.
fn pan_law_to_engine(law: PanLaw) -> PanLawType {
    match law {
        PanLaw::Linear => PanLawType::Linear,
        PanLaw::Minus3dB => PanLawType::Minus3dB,
        PanLaw::Minus4Dot5dB => PanLawType::Minus4Dot5dB,
        PanLaw::Minus6dB => PanLawType::Minus6dB,
    }
}

/// Placeholder parameters for the stack-allocated engine call lists in
/// `process_block` (every slot is overwritten before use; the dummy only
/// exists so the fixed-size arrays can be initialized).
const NULL_PROCESS_PARAMS: ChannelProcessParams = ChannelProcessParams {
    fader_gain: 0.0,
    pan: 0.0,
    muted: false,
    input_gain_db: 0.0,
    phase_inverted: false,
    pan_law: PanLawType::Linear,
};

/// WASM binding for the oximedia-mixer audio engine.
///
/// # Architecture per block (staging → 9 engine instances → bus tail)
///
/// 1. FIFO drain: elastic per-channel input FIFOs → `block_inputs`
///    (raw mono rows, 128 × bs; starvation = zeros).
/// 2. STAGING: every existing strip's raw source signal is copied into
///    its `staged` row, then the strip's own dynamics
///    (gate → expander → compressor) and 6-band EQ run in place.
///    Input strips (0-127) stage their own drained row; bus slots
///    (128-255) stage the RAW row of their assigned source channel —
///    a parallel raw tap, so the source strip's mute/fader/EQ/dynamics
///    have ZERO effect on the bus path (pinned by tests/bus_parallel_test.rs).
/// 3. Per-channel metering (post-dynamics/EQ, pre-fader) and the
///    channel direct-out tap (staged × input gain × phase × fader × VCA,
///    pre-pan; muted/solo-gated strips tap silence).
/// 4. NINE engine instances perform ALL summing via `process_mix_rt`
///    (input gain/phase → effects → fader × VCA → pan → PDC → sends →
///    routing): `main` mixes strips 0-127 that exist, are not
///    effectively muted, and have `main_assign`; bus engine i mixes its
///    16 slots under the same rules.
/// 5. BUS TAIL: each bus's engine output × bus gain (mute → silence) →
///    `bus_pub` (the bus's own published stereo output); buses with
///    `feeds_main` && !bus_muted accumulate into master.
/// 6. Master gain → master limiter (L/R) → interleave → MasterMeter.
///
/// Channel direct-out tap: `set_channel_tap()` captures each input
/// channel's MONO signal at the pan-stage input (post input gain/phase,
/// post gate/comp/EQ, post fader) during `process()` for external
/// publishing (WebSRT Nch out).
#[wasm_bindgen]
pub struct MixerWasm {
    // ── Engine instances: all summing happens here ──
    // One main console (input strips 0-127, max 128 channels) + 8 bus
    // consoles (slots 128-255, 16 slots each). Flat slot index
    // 128 + bus*16 + slot lives in `buses[bus]` under its own
    // `add_channel` id. Every gain/mute/pan lookup resolves through the
    // OWNING instance (`owning_engine`).
    main: oximedia_mixer::AudioMixer,
    buses: Vec<oximedia_mixer::AudioMixer>,
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
    /// Filled at the top of process(); the staging pass reads from it.
    block_inputs: Vec<f32>,
    /// Staged per-strip working buffer: flat [strip][frame]
    /// (256 strips × buffer_size). After the STAGING pass each row holds
    /// the strip's raw source signal processed through its own dynamics
    /// and EQ; the engine call lists below point into these rows.
    staged: Vec<f32>,
    /// PID → channel mapping, keyed by an opaque u32: the JS side encodes
    /// `(sessionId << 16) | pid` so multiple WebSRT sessions can reuse
    /// the same TS PID numbers without colliding. The u16 accessors use
    /// the bare PID as the key (= session 0).
    pid_map: HashMap<u32, PidMapping>,
    master_meter: MasterMeter,
    /// Monotonic process() call count (drives drift-statistics windows).
    block_counter: u64,
    /// Diagnostics: channel-blocks that drained an empty FIFO.
    starved_channel_blocks: u64,
    // ── Pre-allocated scratch buffers ──
    master_left: Vec<f32>,
    master_right: Vec<f32>,
    stereo_out: Vec<f32>,
    deinterleave_scratch: Vec<Vec<f32>>,
    raw_input: Vec<f32>,
    /// Reused per-block snapshot of every existing strip's gain params
    /// (indices 0-255, input strips and bus slots alike), read from each
    /// strip's OWNING engine instance. Drives the direct-out tap and the
    /// nine engine call lists.
    strip_params: Vec<(u32, ChannelId, MixParams)>,
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
    // ── 8 summing buses: each bus sums its 16 slot channels ──
    // Slot index (u32) = 128 + bus*16 + slot, bus 0-7, slot 0-15.
    bus_sources: Vec<Vec<Option<u32>>>,
    bus_gains: Vec<f32>,
    bus_muted: Vec<bool>,
    /// Whether each bus accumulates into the master mix (default true).
    /// Independent of `take_bus_output`, which always publishes bus_pub.
    feeds_main: Vec<bool>,
    /// Whether each input strip (0-127) is in the main engine's call
    /// list (default true). Unassigned strips are still staged, metered,
    /// tapped, and still feed any bus slots assigned to them.
    main_assign: Vec<bool>,
    // ── Scenes (control-plane only; never touched by process_block) ──
    /// Stored console snapshots (`save_scene`/`recall_scene`). Binding-
    /// owned by design — see the Scenes block near the top of this file.
    scenes: HashMap<u32, ConsoleScene>,
    /// Next scene id `save_scene` hands out (starts at 1: 0 stays free
    /// as a JS-friendly "no scene" sentinel).
    next_scene_id: u32,
    /// In-progress timed cross-fade (`recall_scene_fade`), advanced per
    /// block inside `process_block`. None = no fade (the steady state).
    fade: Option<FadeState>,
    /// True while `process_block`'s fade driver reapplies parameters
    /// through the public setters — their cancel-on-set must not fire
    /// against the fade's own writes. The single boolean behind the
    /// cancel policy; see `cancel_fade`.
    fade_applying: bool,
    // Bus engine outputs: what bus i's engine instance summed this block
    // (pre bus-gain). The bus tail applies gain/mute into bus_pub.
    bus_left: Vec<Vec<f32>>,
    bus_right: Vec<Vec<f32>>,
    // Bus published stereo output (post bus-gain, mute → silence): the
    // bus's own output, drained per block by take_bus_output and
    // accumulated into master when feeds_main.
    bus_pub_l: Vec<Vec<f32>>,
    bus_pub_r: Vec<Vec<f32>>,
    // Interleave scratch for take_bus_output (drain path, not RT).
    bus_out_buf: Vec<f32>,
    // Frames valid in the most recent processed block; 0 = nothing new.
    bus_out_frames: usize,
    // Per-bus "new block available" flags (drained by take_bus_output).
    bus_out_ready: [bool; 8],
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
        // The console is a fixed 256 strips: 128 input strips (0-127) +
        // 128 bus slots (128-255, slot = 128 + bus*16 + slot). The
        // max_channels argument is advisory — internal arrays are always
        // full-console (as before, where it was clamped up to 256).
        let _ = max_channels;
        let n = 256;
        let main = oximedia_mixer::AudioMixer::new(MixerConfig {
            sample_rate,
            buffer_size: bs,
            max_channels: 128,
            ..Default::default()
        });
        let buses: Vec<oximedia_mixer::AudioMixer> = (0..8)
            .map(|_| {
                oximedia_mixer::AudioMixer::new(MixerConfig {
                    sample_rate,
                    buffer_size: bs,
                    max_channels: 16,
                    ..Default::default()
                })
            })
            .collect();

        Ok(MixerWasm {
            main,
            buses,
            buffer_size: bs,
            channel_ids: vec![None; n],
            channel_inputs: HashMap::new(),
            block_inputs: vec![0.0; 128 * bs],
            staged: vec![0.0; 256 * bs],
            pid_map: HashMap::new(),
            master_meter: MasterMeter::new(sample_rate, bs),
            block_counter: 0,
            starved_channel_blocks: 0,
            master_left: vec![0.0; bs],
            master_right: vec![0.0; bs],
            stereo_out: vec![0.0; bs * 2],
            deinterleave_scratch: Vec::new(),
            raw_input: Vec::new(),
            strip_params: Vec::new(),
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
            bus_sources: (0..8).map(|_| vec![None; 16]).collect(),
            bus_gains: vec![1.0; 8],
            bus_muted: vec![false; 8],
            feeds_main: vec![true; 8],
            main_assign: vec![true; 128],
            scenes: HashMap::new(),
            next_scene_id: 1,
            fade: None,
            fade_applying: false,
            bus_left: (0..8).map(|_| vec![0.0; bs]).collect(),
            bus_right: (0..8).map(|_| vec![0.0; bs]).collect(),
            bus_pub_l: (0..8).map(|_| vec![0.0; bs]).collect(),
            bus_pub_r: (0..8).map(|_| vec![0.0; bs]).collect(),
            bus_out_buf: vec![0.0; bs * 2],
            bus_out_frames: 0,
            bus_out_ready: [false; 8],
            bus_peak: HashMap::new(),
            bus_rms: HashMap::new(),
            tap_channels: 0,
            channel_tap_buf: Vec::new(),
            tap_frames: 0,
            unmapped_pid_drops: 0,
            sample_rate,
        })
    }

    /// The engine instance that OWNS flat strip index `idx`: `main` for
    /// input strips (0-127), `buses[(idx-128)/16]` for bus slots
    /// (128-255). `None` beyond the console.
    fn owning_engine(&self, idx: u32) -> Option<&oximedia_mixer::AudioMixer> {
        if idx < 128 {
            Some(&self.main)
        } else {
            self.buses.get(((idx - 128) / 16) as usize)
        }
    }

    fn owning_engine_mut(&mut self, idx: u32) -> Option<&mut oximedia_mixer::AudioMixer> {
        if idx < 128 {
            Some(&mut self.main)
        } else {
            self.buses.get_mut(((idx - 128) / 16) as usize)
        }
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
            let engine = self
                .owning_engine_mut(idx)
                .expect("idx < channel_ids.len() always has an owning instance");
            let id = engine
                .add_channel(format!("ch{idx}"), ChannelType::Mono, ChannelLayout::Mono)
                .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
            if let Ok(ch) = engine.get_channel_mut(id) {
                ch.set_pan_law(PanLaw::Linear);
            }
            self.channel_ids[i] = Some(id);
            self.eq_chains
                .insert(idx, EqEffect::six_band(self.sample_rate));
            self.dynamics_chains.insert(idx, ChannelDynamics::new());
        }
        Ok(self.channel_ids[i].unwrap())
    }

    /// Snapshot the gain-stage parameters for a strip index (input or
    /// slot), read from the OWNING engine instance (`main` for 0-127,
    /// the bus engine for 128-255). The engine instances apply these in
    /// `process_mix_rt`; the direct-out tap applies the same gain math
    /// minus pan.
    fn mix_params_for(&self, ch_idx: u32) -> Option<(ChannelId, MixParams)> {
        let id = self.channel_ids.get(ch_idx as usize)?.as_ref().copied()?;
        let engine = self.owning_engine(ch_idx)?;
        let ch = engine.get_channel(id).ok()?;
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
    //
    // Two addressing modes over one map: the legacy u16 accessors key by
    // the bare PID (= session 0) and simply delegate; the `*_keyed`
    // variants take the raw map key so multiple WebSRT sessions can
    // reuse the same TS PID numbers (see the `pid_map` field comment).

    pub fn map_pid(&mut self, pid: u16, ch_start: u32, channel_count: u32) -> Result<(), JsValue> {
        self.map_pid_keyed(pid as u32, ch_start, channel_count)
    }
    pub fn unmap_pid(&mut self, pid: u16) {
        self.unmap_pid_keyed(pid as u32);
    }
    pub fn pid_channel(&self, pid: u16) -> i32 {
        self.pid_channel_keyed(pid as u32)
    }
    pub fn pid_channel_count(&self, pid: u16) -> u32 {
        self.pid_channel_count_keyed(pid as u32)
    }
    pub fn subscribe_pid(&mut self, pid: u16) {
        self.subscribe_pid_keyed(pid as u32);
    }
    pub fn unsubscribe_pid(&mut self, pid: u16) {
        self.unsubscribe_pid_keyed(pid as u32);
    }
    pub fn feed_pcm(&mut self, pid: u16, data: &js_sys::Float32Array) -> Result<(), JsValue> {
        self.feed_pcm_keyed(pid as u32, data)
    }

    pub fn map_pid_keyed(
        &mut self,
        key: u32,
        ch_start: u32,
        channel_count: u32,
    ) -> Result<(), JsValue> {
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
            key,
            PidMapping {
                ch_start,
                channel_count,
                subscribed: true,
            },
        );
        Ok(())
    }
    pub fn unmap_pid_keyed(&mut self, key: u32) {
        self.pid_map.remove(&key);
    }
    pub fn pid_channel_keyed(&self, key: u32) -> i32 {
        self.pid_map
            .get(&key)
            .map(|m| m.ch_start as i32)
            .unwrap_or(-1)
    }
    pub fn pid_channel_count_keyed(&self, key: u32) -> u32 {
        self.pid_map.get(&key).map(|m| m.channel_count).unwrap_or(0)
    }
    pub fn subscribe_pid_keyed(&mut self, key: u32) {
        let Some(mut m) = self.pid_map.get(&key).copied() else {
            return;
        };
        m.subscribed = true;
        self.pid_map.insert(key, m);
        for i in 0..m.channel_count {
            let ch = m.ch_start + i;
            if let Some(&Some(id)) = self.channel_ids.get(ch as usize) {
                if let Some(engine) = self.owning_engine_mut(ch) {
                    if let Ok(c) = engine.get_channel_mut(id) {
                        c.set_muted(false);
                    }
                }
            }
        }
    }
    pub fn unsubscribe_pid_keyed(&mut self, key: u32) {
        let Some(mut m) = self.pid_map.get(&key).copied() else {
            return;
        };
        m.subscribed = false;
        self.pid_map.insert(key, m);
        for i in 0..m.channel_count {
            let ch = m.ch_start + i;
            if let Some(&Some(id)) = self.channel_ids.get(ch as usize) {
                if let Some(engine) = self.owning_engine_mut(ch) {
                    if let Ok(c) = engine.get_channel_mut(id) {
                        c.set_muted(true);
                    }
                }
            }
        }
    }

    /// Keyed twin of `feed_pcm`: unmapped key → counted drop (never an
    /// error), unsubscribed → silent drop; otherwise the JS buffer is
    /// copied once and routed through the pure-Rust `feed_interleaved`
    /// (same copy-then-inner structure as `set_channel_input_interleaved`,
    /// so native tests can exercise the same path without a JS runtime).
    pub fn feed_pcm_keyed(&mut self, key: u32, data: &js_sys::Float32Array) -> Result<(), JsValue> {
        let Some(mapping) = self.pid_map.get(&key).copied() else {
            self.unmapped_pid_drops += 1;
            return Ok(());
        };
        if !mapping.subscribed {
            return Ok(());
        }
        let len = data.length() as usize;
        self.raw_input.resize(len, 0.0);
        data.copy_to(&mut self.raw_input);
        let raw = std::mem::take(&mut self.raw_input);
        let r = self.feed_interleaved(mapping.ch_start, &raw, mapping.channel_count);
        self.raw_input = raw;
        r
    }

    /// Count of PCM packets dropped due to unmapped PID (for diagnostics).
    pub fn unmapped_pid_count(&self) -> u64 {
        self.unmapped_pid_drops
    }

    // ── Channel controls ───────────────────────────────

    pub fn set_channel_gain(&mut self, ch: u32, gain: f32) -> Result<(), JsValue> {
        self.cancel_fade();
        let id = self.ensure_channel(ch)?;
        self.owning_engine_mut(ch)
            .expect("ensured channel has an owning instance")
            .set_channel_gain(id, gain)
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }
    pub fn set_channel_pan(&mut self, ch: u32, pan: f32) -> Result<(), JsValue> {
        self.cancel_fade();
        let id = self.ensure_channel(ch)?;
        self.owning_engine_mut(ch)
            .expect("ensured channel has an owning instance")
            .set_channel_pan(id, pan)
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }
    pub fn set_channel_mute(&mut self, ch: u32, muted: bool) -> Result<(), JsValue> {
        self.cancel_fade();
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
        self.cancel_fade();
        let _ = self.ensure_channel(ch)?;
        if soloed {
            self.soloed_channels.insert(ch);
        } else {
            self.soloed_channels.remove(&ch);
        }
        // Re-evaluate all strips' effective mute state in their OWNING
        // instances — solo is global across all nine instances (input
        // strips and bus slots alike).
        for i in 0..self.channel_ids.len() {
            let Some(id) = self.channel_ids[i] else {
                continue;
            };
            let idx = i as u32;
            let effective = self.user_muted.contains(&idx)
                || (self.solo_active() && !self.soloed_channels.contains(&idx));
            if let Some(engine) = self.owning_engine_mut(idx) {
                if let Ok(c) = engine.get_channel_mut(id) {
                    c.set_muted(effective);
                }
            }
        }
        Ok(())
    }

    fn solo_active(&self) -> bool {
        !self.soloed_channels.is_empty()
    }

    fn set_engine_mute(&mut self, ch: u32, muted: bool) {
        if let Some(&Some(id)) = self.channel_ids.get(ch as usize) {
            if let Some(engine) = self.owning_engine_mut(ch) {
                if let Ok(channel) = engine.get_channel_mut(id) {
                    channel.set_muted(muted);
                }
            }
        }
    }

    // ── EQ controls ────────────────────────────────────

    pub fn set_eq_band_gain(&mut self, ch: u32, band: usize, gain_db: f32) -> Result<(), JsValue> {
        self.cancel_fade();
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            if band < eq.inner().bands.len() {
                eq.inner_mut().bands[band].set_gain_db(gain_db as f64);
                eq.inner_mut().bands[band].update_coefficients();
            }
        }
        Ok(())
    }
    pub fn set_eq_band_freq(&mut self, ch: u32, band: usize, freq_hz: f32) -> Result<(), JsValue> {
        self.cancel_fade();
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            if band < eq.inner().bands.len() {
                eq.inner_mut().bands[band].set_frequency(freq_hz as f64);
                eq.inner_mut().bands[band].update_coefficients();
            }
        }
        Ok(())
    }
    pub fn set_eq_band_q(&mut self, ch: u32, band: usize, q: f32) -> Result<(), JsValue> {
        self.cancel_fade();
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            if band < eq.inner().bands.len() {
                eq.inner_mut().bands[band].set_q(q as f64);
                eq.inner_mut().bands[band].update_coefficients();
            }
        }
        Ok(())
    }
    pub fn set_eq_bypass(&mut self, ch: u32, bypassed: bool) -> Result<(), JsValue> {
        self.cancel_fade();
        self.ensure_channel(ch)?;
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            eq.set_bypassed(bypassed);
        }
        Ok(())
    }

    // ── Dynamics controls ──────────────────────────────

    /// Enable compressor on a channel with broadcast defaults (-12 dB threshold, 3:1 ratio).
    pub fn enable_compressor(&mut self, ch: u32) -> Result<(), JsValue> {
        self.cancel_fade();
        let _ = self.ensure_channel(ch)?;
        self.dynamics_chains.entry(ch).or_default().compressor =
            Some(CompressorEffect::broadcast(self.sample_rate));
        Ok(())
    }
    pub fn disable_compressor(&mut self, ch: u32) {
        self.cancel_fade();
        if let Some(d) = self.dynamics_chains.get_mut(&ch) {
            d.compressor = None;
        }
    }
    pub fn enable_gate(&mut self, ch: u32) -> Result<(), JsValue> {
        self.cancel_fade();
        let _ = self.ensure_channel(ch)?;
        self.dynamics_chains.entry(ch).or_default().gate =
            Some(GateEffect::denoise(self.sample_rate));
        Ok(())
    }
    pub fn disable_gate(&mut self, ch: u32) {
        self.cancel_fade();
        if let Some(d) = self.dynamics_chains.get_mut(&ch) {
            d.gate = None;
        }
    }

    pub fn enable_expander(&mut self, ch: u32) -> Result<(), JsValue> {
        self.cancel_fade();
        let _ = self.ensure_channel(ch)?;
        self.dynamics_chains.entry(ch).or_default().expander =
            Some(ExpanderEffect::gentle(self.sample_rate));
        Ok(())
    }
    pub fn disable_expander(&mut self, ch: u32) {
        self.cancel_fade();
        if let Some(d) = self.dynamics_chains.get_mut(&ch) {
            d.expander = None;
        }
    }

    pub fn set_comp_param(&mut self, ch: u32, param: u32, value: f32) -> Result<(), JsValue> {
        self.cancel_fade();
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
        self.cancel_fade();
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
        self.cancel_fade();
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
        self.cancel_fade();
        let id = self.ensure_channel(ch)?;
        if let Some(engine) = self.owning_engine_mut(ch) {
            if let Ok(c) = engine.get_channel_mut(id) {
                c.input_mut().gain_db = gain_db;
            }
        }
        Ok(())
    }

    pub fn set_channel_phase(&mut self, ch: u32, inverted: bool) -> Result<(), JsValue> {
        self.cancel_fade();
        let id = self.ensure_channel(ch)?;
        if let Some(engine) = self.owning_engine_mut(ch) {
            if let Ok(c) = engine.get_channel_mut(id) {
                c.set_phase_inverted(inverted);
            }
        }
        Ok(())
    }

    pub fn set_channel_pan_law(&mut self, ch: u32, law: u32) -> Result<(), JsValue> {
        self.cancel_fade();
        let id = self.ensure_channel(ch)?;
        let pan_law = match law {
            0 => PanLaw::Linear,
            1 => PanLaw::Minus3dB,
            2 => PanLaw::Minus4Dot5dB,
            3 => PanLaw::Minus6dB,
            _ => return Err(JsValue::from_str("invalid pan law")),
        };
        if let Some(engine) = self.owning_engine_mut(ch) {
            if let Ok(c) = engine.get_channel_mut(id) {
                c.set_pan_law(pan_law);
            }
        }
        Ok(())
    }

    pub fn set_channel_name(&mut self, ch: u32, name: String) -> Result<(), JsValue> {
        self.cancel_fade();
        let id = self.ensure_channel(ch)?;
        if let Some(engine) = self.owning_engine_mut(ch) {
            if let Ok(c) = engine.get_channel_mut(id) {
                c.set_name(name);
            }
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
    /// The strip's compressor gain reduction (dB, positive = attenuation)
    /// from the most recent processed block. 0.0 when the strip has no
    /// compressor.
    pub fn channel_comp_gr_db(&self, ch: u32) -> f32 {
        self.dynamics_chains
            .get(&ch)
            .and_then(|d| d.compressor.as_ref())
            .map(|c| c.gain_reduction_db())
            .unwrap_or(0.0)
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
            let gr = self.channel_comp_gr_db(ch);
            s.push_str(&format!(
                "{{\"ch\":{ch},\"peak\":{peak:.1},\"rms\":{rms:.1},\"gr\":{gr:.1}}}"
            ));
        }
        s.push(']');
        s
    }

    // ── Master controls ────────────────────────────────

    pub fn set_master_gain(&mut self, gain: f32) {
        self.cancel_fade();
        self.master_gain = gain.clamp(0.0, 2.0);
    }

    /// Live-update the limiter ceiling. Assigns `threshold_db` on both L/R
    /// limiters and calls `recalculate()`, which recomputes only the
    /// derived coefficients — `gain_env`, look-ahead delay and oversample
    /// state survive, so turning the knob mid-stream is click-free
    /// (the previous reconstruct-via-`new()` reset the envelope, causing
    /// an audible gain jump).
    pub fn set_limiter_ceiling(&mut self, ceiling_db: f32) {
        self.limiter_ceiling = ceiling_db;
        self.limiter_l.threshold_db = ceiling_db;
        self.limiter_r.threshold_db = ceiling_db;
        self.limiter_l.recalculate();
        self.limiter_r.recalculate();
    }
    /// Live-update the limiter release. Same rationale as
    /// `set_limiter_ceiling`: `recalculate()` preserves the gain envelope,
    /// so release changes never reset limiting mid-stream.
    pub fn set_limiter_release(&mut self, release_ms: f32) {
        self.limiter_release_ms = release_ms;
        self.limiter_l.release_ms = release_ms;
        self.limiter_r.release_ms = release_ms;
        self.limiter_l.recalculate();
        self.limiter_r.recalculate();
    }

    // ── Bus mixing (8 buses × 16 full-channel-strip slots) ──

    /// Assign bus `bus` slot `slot` to tap input channel `ch` (a parallel
    /// RAW tap — `ch`'s own mute/fader/EQ/dynamics never affect the bus
    /// path). Lazily creates the slot's engine channel / EQ / dynamics
    /// (flat strip index 128 + bus*16 + slot).
    pub fn set_bus_source(&mut self, bus: u32, slot: u32, ch: u32) -> Result<(), JsValue> {
        self.cancel_fade();
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
        self.cancel_fade();
        if (bus as usize) < 8 && (slot as usize) < 16 {
            self.bus_sources[bus as usize][slot as usize] = None;
        }
    }

    /// Include/exclude an input strip from the main-bus mix (strips
    /// 0-127 only; other indices are ignored). Default true. Purely
    /// removes the strip from the main engine's call list — the strip is
    /// still staged, metered, tapped, and still feeds any bus slots
    /// assigned to it.
    pub fn set_channel_main_assign(&mut self, ch: u32, on: bool) {
        self.cancel_fade();
        if (ch as usize) < self.main_assign.len() {
            self.main_assign[ch as usize] = on;
        }
    }

    /// Whether bus `bus` accumulates into the master mix (default true).
    /// Independent of the bus's own published output
    /// (`take_bus_output` still returns signal).
    pub fn set_bus_feeds_main(&mut self, bus: u32, on: bool) {
        self.cancel_fade();
        if (bus as usize) < 8 {
            self.feeds_main[bus as usize] = on;
        }
    }

    pub fn set_bus_gain(&mut self, bus: u32, gain: f32) {
        self.cancel_fade();
        if (bus as usize) < 8 {
            self.bus_gains[bus as usize] = gain.clamp(0.0, 2.0);
        }
    }
    pub fn set_bus_mute(&mut self, bus: u32, muted: bool) {
        self.cancel_fade();
        if (bus as usize) < 8 {
            self.bus_muted[bus as usize] = muted;
        }
    }

    // ── Scenes ─────────────────────────────────────────

    /// Capture the current console state and store it under a fresh
    /// scene id. Saving is read-only — it never disturbs audio or
    /// control state. Returns the new id (ids start at 1).
    pub fn save_scene(&mut self) -> u32 {
        let id = self.next_scene_id;
        self.next_scene_id += 1;
        self.scenes.insert(id, self.console_snapshot());
        id
    }

    /// Instantly recall scene `id`: every captured parameter is
    /// reapplied through the SAME setter paths the JS surface uses, so
    /// engine state, staged EQ/dynamics params and the derived mute/solo
    /// state all stay consistent. Any in-progress cross-fade is dropped
    /// (an instant recall is itself a scene change — the user takes
    /// over). Unknown id → Err.
    pub fn recall_scene(&mut self, id: u32) -> Result<(), JsValue> {
        let Some(scene) = self.scenes.get(&id) else {
            return Err(scene_err(format!("unknown scene id {id}")));
        };
        let scene = scene.clone();
        self.fade = None;
        self.apply_scene(&scene);
        Ok(())
    }

    /// Recall scene `id` as a timed cross-fade over `fade_ms`: the
    /// current console (snapshotted at call time) is the "from" state,
    /// the stored scene the "to" state, and `process_block` interpolates
    /// between them at block granularity (audio-rate, no timers or
    /// threads — see `apply_scene_interp` for the per-parameter domains;
    /// booleans snap at the half-way point).
    ///
    /// * Unknown id → Err (same as instant recall). `fade_ms < 0` (or
    ///   NaN) → Err. `fade_ms == 0` delegates to instant `recall_scene`.
    /// * Cancel-on-set: any scene-affecting setter called while the fade
    ///   is running drops it immediately — the user takes over from the
    ///   already-applied interpolation (nothing further is applied).
    ///   Stream-state operations (PID mapping, PCM feeds, subscribe,
    ///   limiter, tap) do NOT cancel: they aren't console state.
    /// * Control-plane allocations happen HERE only (the two scene
    ///   snapshots, plus HashSets pre-reserved so boolean snaps can
    ///   never grow them on the audio thread); the per-block
    ///   interpolation is allocation-free (pinned by alloc_test).
    /// * Non-audio state snaps immediately: strip NAMES (String clones
    ///   are control-plane-only work; the RT path never touches them).
    /// * Replaces any fade already in progress (from-state = whatever
    ///   the console holds at call time).
    pub fn recall_scene_fade(&mut self, id: u32, fade_ms: f64) -> Result<(), JsValue> {
        let Some(scene) = self.scenes.get(&id) else {
            return Err(scene_err(format!("unknown scene id {id}")));
        };
        if fade_ms.is_nan() || fade_ms < 0.0 {
            return Err(scene_err(format!("fade_ms must be >= 0 (got {fade_ms})")));
        }
        if fade_ms == 0.0 {
            return self.recall_scene(id);
        }
        let to = scene.clone();
        let from = self.console_snapshot();
        // Snap the target's labels now: names are inert String state and
        // the interpolation path must stay allocation-free.
        for (i, s) in to.strips.iter().enumerate() {
            if s.exists {
                let _ = self.set_channel_name(i as u32, s.name.clone());
            }
        }
        // The mute/solo boolean snap (t >= 0.5) can INSERT keys on the
        // audio thread. Reserving the full console up front means those
        // inserts can never grow (allocate) the sets mid-fade.
        self.user_muted.reserve(256);
        self.soloed_channels.reserve(256);
        self.fade = Some(FadeState {
            from,
            to,
            pos_ms: 0.0,
            dur_ms: fade_ms,
        });
        Ok(())
    }

    /// Drop scene `id` (unknown ids are a no-op).
    pub fn delete_scene(&mut self, id: u32) {
        self.scenes.remove(&id);
    }

    /// Number of stored scenes.
    pub fn scene_count(&self) -> u32 {
        self.scenes.len() as u32
    }

    /// Serialize the console state for the UI as JSON — the reply to
    /// the worklet's `get-params` pull after a `scene-recalled`
    /// notification. Field names mirror the SolidJS stores exactly
    /// (see `console_params_json_inner`, the pure-Rust core). During a
    /// timed cross-fade the fade target serializes, so the UI shows the
    /// recalled state the moment a fade starts.
    pub fn console_params_json(&self) -> String {
        self.console_params_json_inner()
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

    /// Drain bus `bus`'s own stereo output from the most recent
    /// `process()` call: one block of bs frames, interleaved L/R, post
    /// bus-gain (a muted bus publishes silence). Per-bus drain contract
    /// matching `take_channel_tap`:
    ///
    /// * Empty Float32Array when the bus index is invalid (≥ 8) or no
    ///   new block has been processed since the last take of that bus.
    /// * Taking bus A does not consume bus B's block; if two `process()`
    ///   calls happen without a take in between, only the LATEST block
    ///   is kept.
    pub fn take_bus_output(&mut self, bus: u32) -> js_sys::Float32Array {
        let b = bus as usize;
        if b >= 8 || !self.bus_out_ready[b] || self.bus_out_frames == 0 {
            return js_sys::Float32Array::new_with_length(0);
        }
        self.bus_out_ready[b] = false;
        let n = self.bus_out_frames;
        // Interleave into the pre-allocated scratch (drain path, not RT)
        // so the copy into JS is a single memcpy, like take_channel_tap.
        self.bus_out_buf.clear();
        for i in 0..n {
            self.bus_out_buf.push(self.bus_pub_l[b][i]);
            self.bus_out_buf.push(self.bus_pub_r[b][i]);
        }
        let out = js_sys::Float32Array::new_with_length((n * 2) as u32);
        out.copy_from(&self.bus_out_buf[..n * 2]);
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
    /// Capture the entire binding console state as a [`ConsoleScene`].
    ///
    /// The same read primitive `save_scene` uses — public plain Rust
    /// (not wasm-exported) so native tests can verify scene round-trips
    /// field by field, and so future control-plane code (e.g. cross-fade
    /// recall) can diff current state against a target scene.
    ///
    /// Control-plane only: reads the owning engines' channel state and
    /// the binding's staging maps; allocates the returned scene and
    /// nothing else; never mutates anything.
    pub fn console_snapshot(&self) -> ConsoleScene {
        let mut scene = ConsoleScene::default();
        for (i, s) in scene.strips.iter_mut().enumerate() {
            let idx = i as u32;
            s.exists = self.channel_ids.get(i).is_some_and(|c| c.is_some());
            if let Some((_, p)) = self.mix_params_for(idx) {
                s.gain = p.fader_gain;
                s.pan = p.pan;
                s.input_gain_db = p.input_gain_db;
                s.phase_inverted = p.phase_inverted;
                s.pan_law = p.pan_law;
            }
            if let (Some(&Some(id)), Some(engine)) =
                (self.channel_ids.get(i), self.owning_engine(idx))
            {
                if let Ok(ch) = engine.get_channel(id) {
                    s.name = ch.name().to_string();
                }
            }
            // Mute/solo are captured as USER intent; the derived engine
            // mute is recomputed by the setters on recall.
            s.mute = self.user_muted.contains(&idx);
            s.solo = self.soloed_channels.contains(&idx);
            s.main_assign = self.main_assign.get(i).copied().unwrap_or(false);
            if let Some(eq) = self.eq_chains.get(&idx) {
                s.eq_bypass = eq.is_bypassed();
                for (b, bs) in s.eq_bands.iter_mut().enumerate() {
                    if let Some(band) = eq.inner().bands.get(b) {
                        *bs = EqBandScene {
                            gain_db: band.gain_db as f32,
                            freq_hz: band.frequency as f32,
                            q: band.q as f32,
                        };
                    }
                }
            }
            if let Some(d) = self.dynamics_chains.get(&idx) {
                s.comp = d.compressor.as_ref().map(|c| {
                    let cfg = c.config();
                    CompScene {
                        threshold_db: cfg.threshold_db,
                        ratio: cfg.ratio,
                        attack_ms: cfg.attack_ms,
                        release_ms: cfg.release_ms,
                        makeup_gain_db: cfg.makeup_gain_db,
                        knee_db: cfg.knee_db,
                    }
                });
                s.gate = d.gate.as_ref().map(|g| {
                    let cfg = g.config();
                    GateScene {
                        threshold_db: cfg.threshold_db,
                        hysteresis_db: cfg.hysteresis_db,
                        attack_ms: cfg.attack_ms,
                        release_ms: cfg.release_ms,
                        hold_ms: cfg.hold_ms,
                    }
                });
                s.expander = d.expander.as_ref().map(|e| {
                    let cfg = e.config();
                    ExpanderScene {
                        threshold_db: cfg.threshold_db,
                        ratio: cfg.ratio,
                        attack_ms: cfg.attack_ms,
                        release_ms: cfg.release_ms,
                    }
                });
            }
        }
        for (b, bs) in scene.buses.iter_mut().enumerate() {
            for (slot, src) in bs.sources.iter_mut().enumerate() {
                *src = self.bus_sources[b].get(slot).copied().flatten();
            }
            bs.gain = self.bus_gains[b];
            bs.muted = self.bus_muted[b];
            bs.feeds_main = self.feeds_main[b];
        }
        scene.master_gain = self.master_gain;
        scene
    }

    /// Serialize the console state for the UI as JSON (the wasm half of
    /// the scene-recall loop — see the Console-state JSON block above).
    ///
    /// While a timed cross-fade is running the fade TARGET is
    /// serialized, not the transient interpolation: the UI must show
    /// where the console lands the moment a recall starts (recall posts
    /// `scene-recalled` before the first interpolated block). Strips
    /// that don't exist are skipped; all 8 buses are always included.
    ///
    /// Control-plane only (recall-rate, user clicks — allocation fine);
    /// the RT path never calls this. Pure Rust inner fn behind the
    /// `console_params_json` wasm wrapper so native tests exercise the
    /// same serialization (`process`/`process_block` pattern).
    pub fn console_params_json_inner(&self) -> String {
        match &self.fade {
            Some(fade) => self.scene_params_json(&fade.to),
            None => self.scene_params_json(&self.console_snapshot()),
        }
    }

    /// Render a full console snapshot as the UI-store JSON document.
    fn scene_params_json(&self, scene: &ConsoleScene) -> String {
        let mut j = String::from("{\"masterGain\":");
        j.push_str(&scene.master_gain.to_string());
        j.push_str(",\"strips\":[");
        let mut first = true;
        for (i, s) in scene.strips.iter().enumerate() {
            if !s.exists {
                continue;
            }
            if !first {
                j.push(',');
            }
            first = false;
            j.push_str(&self.strip_params_json(i as u32, s));
        }
        j.push_str("],\"buses\":[");
        for (b, bus) in scene.buses.iter().enumerate() {
            if b > 0 {
                j.push(',');
            }
            j.push_str(&bus_params_json(bus));
        }
        j.push_str("]}");
        j
    }

    /// One strip's JSON object, fields mirroring `ChannelState` in the
    /// UI store (gain/pan are LINEAR, panLaw the wire u32 0-3, EQ the
    /// 6 bands' {gainDb,freqHz,q}). Dynamics params serialize even when
    /// the module is disabled — the enable-constructor defaults, i.e.
    /// exactly what a later enable would create.
    fn strip_params_json(&self, ch: u32, s: &StripScene) -> String {
        let (comp_on, comp) = match &s.comp {
            Some(c) => (true, c.clone()),
            None => (false, comp_enable_defaults(self.sample_rate)),
        };
        let (gate_on, gate) = match &s.gate {
            Some(g) => (true, g.clone()),
            None => (false, gate_enable_defaults(self.sample_rate)),
        };
        let (exp_on, exp) = match &s.expander {
            Some(e) => (true, e.clone()),
            None => (false, expander_enable_defaults(self.sample_rate)),
        };
        let mut j = format!(
            "{{\"ch\":{ch},\"name\":\"{}\",\"gain\":{},\"pan\":{},\"inputGainDb\":{},\"phaseInverted\":{},\"panLaw\":{}",
            json_escape(&s.name),
            s.gain,
            s.pan,
            s.input_gain_db,
            s.phase_inverted,
            pan_law_to_wire(s.pan_law),
        );
        j.push_str(&format!(
            ",\"muted\":{},\"soloed\":{},\"mainAssigned\":{},\"eqBypassed\":{}",
            s.mute, s.solo, s.main_assign, s.eq_bypass
        ));
        j.push_str(",\"eqBands\":[");
        for (b, band) in s.eq_bands.iter().enumerate() {
            if b > 0 {
                j.push(',');
            }
            j.push_str(&format!(
                "{{\"gainDb\":{},\"freqHz\":{},\"q\":{}}}",
                band.gain_db, band.freq_hz, band.q
            ));
        }
        j.push(']');
        j.push_str(&format!(
            ",\"compEnabled\":{comp_on},\"compThresholdDb\":{},\"compRatio\":{},\"compKneeDb\":{},\"compAttackMs\":{},\"compReleaseMs\":{},\"compMakeupDb\":{}",
            comp.threshold_db, comp.ratio, comp.knee_db, comp.attack_ms, comp.release_ms, comp.makeup_gain_db
        ));
        j.push_str(&format!(
            ",\"gateEnabled\":{gate_on},\"gateThresholdDb\":{},\"gateHysteresisDb\":{},\"gateAttackMs\":{},\"gateReleaseMs\":{},\"gateHoldMs\":{}",
            gate.threshold_db, gate.hysteresis_db, gate.attack_ms, gate.release_ms, gate.hold_ms
        ));
        j.push_str(&format!(
            ",\"expanderEnabled\":{exp_on},\"expanderThresholdDb\":{},\"expanderRatio\":{},\"expanderAttackMs\":{},\"expanderReleaseMs\":{}",
            exp.threshold_db, exp.ratio, exp.attack_ms, exp.release_ms
        ));
        j.push('}');
        j
    }

    /// Cancel-on-set policy for timed cross-fades: any scene-affecting
    /// setter called while a fade is in progress drops it — the user
    /// takes over from the already-applied interpolation. No-op when no
    /// fade is live, and while the fade driver itself reapplies through
    /// the public setters (`fade_applying`) — the single boolean that
    /// distinguishes the fade's own writes from user writes.
    fn cancel_fade(&mut self) {
        if !self.fade_applying {
            self.fade = None;
        }
    }

    /// Reapply a scene exactly — the instant-recall path. Names are
    /// String clones (control-plane-only work), so they are applied
    /// here and NOT in `apply_scene_interp`; `recall_scene_fade` snaps
    /// them at fade start instead.
    fn apply_scene(&mut self, scene: &ConsoleScene) {
        for (i, s) in scene.strips.iter().enumerate() {
            if s.exists {
                let _ = self.set_channel_name(i as u32, s.name.clone());
            }
        }
        self.apply_scene_interp(scene, scene, 1.0);
    }

    /// Reapply scene state through the SAME code paths the per-parameter
    /// setters use (never bypassing their side effects: engine state,
    /// staged EQ/dynamics params, derived mute/solo) — either exactly
    /// (`t = 1.0`: apply `to` verbatim; the instant-recall path) or as
    /// the interpolation of two scenes at `t` (the timed-fade path,
    /// driven per block by `process_block`).
    ///
    /// Interpolation domains (endpoint lerps short-circuit, so an exact
    /// apply writes stored values verbatim):
    /// - linear gains (strip fader, bus gain, master gain): dB domain,
    ///   0 linear = the −200 dB meter floor
    /// - pan: linear; EQ frequency and Q: log2 domain
    /// - every other numeric param (dB-denominated gains, ratios,
    ///   times): linear in the stored value
    /// - booleans, pan laws and bus slot sources snap to `to` at
    ///   t >= 0.5 (the engine mix_scene lerp convention); a dynamics
    ///   module enabled on only one side applies that side's params
    ///   verbatim while it is the active side
    ///
    /// Mute/solo stay LAST, as USER intent, behind the foreign-solo
    /// guard — their setters recompute the derived engine mute (user
    /// mute OR solo-gate); a live solo on a strip `to` doesn't cover
    /// would leak into every covered strip's derived mute.
    ///
    /// Allocation-free by construction (this runs on the audio thread
    /// during fades): no String clones, no collects, and the mute/solo
    /// HashSets are pre-reserved at fade start so boolean snaps can't
    /// grow them. Mute/solo no-op writes are skipped: at fade start the
    /// console equals `from` (a snapshot), and any later user change
    /// cancels the fade, so an unchanged value is already in place.
    /// Mid-fade (t < 1.0) strips/buses identical in both scenes are
    /// skipped entirely for the same reason.
    fn apply_scene_interp(&mut self, from: &ConsoleScene, to: &ConsoleScene, t: f32) {
        let mid_fade = t < 1.0;
        // Strip parameters, only for strips that existed at save time
        // (strips are never destroyed, so the setters below never lazily
        // create anything; strips created after the save are left alone —
        // a scene has no opinion about strips it never saw). Every
        // setter can only fail on out-of-range indices, which the fixed
        // 256-strip console excludes — ignore results.
        for i in 0..to.strips.len() {
            let ts = &to.strips[i];
            if !ts.exists {
                continue;
            }
            let fs = &from.strips[i];
            if mid_fade && fs == ts {
                continue;
            }
            let idx = i as u32;
            let _ = self.set_channel_gain(idx, lerp_gain_db(fs.gain, ts.gain, t));
            let _ = self.set_channel_pan(idx, lerp_lin(fs.pan, ts.pan, t));
            let _ =
                self.set_channel_input_gain(idx, lerp_lin(fs.input_gain_db, ts.input_gain_db, t));
            let _ = self.set_channel_phase(idx, lerp_bool(fs.phase_inverted, ts.phase_inverted, t));
            let law = if t >= 0.5 { ts.pan_law } else { fs.pan_law };
            let _ = self.set_channel_pan_law(idx, pan_law_to_wire(law));
            if idx < 128 {
                self.set_channel_main_assign(idx, lerp_bool(fs.main_assign, ts.main_assign, t));
            }
            let _ = self.set_eq_bypass(idx, lerp_bool(fs.eq_bypass, ts.eq_bypass, t));
            for b in 0..ts.eq_bands.len() {
                let fb = &fs.eq_bands[b];
                let tb = &ts.eq_bands[b];
                let _ = self.set_eq_band_gain(idx, b, lerp_lin(fb.gain_db, tb.gain_db, t));
                let _ = self.set_eq_band_freq(idx, b, lerp_log2(fb.freq_hz, tb.freq_hz, t));
                let _ = self.set_eq_band_q(idx, b, lerp_log2(fb.q, tb.q, t));
            }
            // Dynamics: enable snaps at t >= 0.5; when both sides are
            // enabled the params interpolate (a disabled side
            // contributes nothing — the enabled side's params apply
            // verbatim).
            if lerp_bool(fs.comp.is_some(), ts.comp.is_some(), t) {
                let f = fs
                    .comp
                    .as_ref()
                    .or(ts.comp.as_ref())
                    .expect("one side enabled");
                let e = ts.comp.as_ref().unwrap_or(f);
                let _ = self.enable_compressor(idx);
                let _ = self.set_comp_param(idx, 0, lerp_lin(f.threshold_db, e.threshold_db, t));
                let _ = self.set_comp_param(idx, 1, lerp_lin(f.ratio, e.ratio, t));
                let _ = self.set_comp_param(idx, 2, lerp_lin(f.attack_ms, e.attack_ms, t));
                let _ = self.set_comp_param(idx, 3, lerp_lin(f.release_ms, e.release_ms, t));
                let _ =
                    self.set_comp_param(idx, 4, lerp_lin(f.makeup_gain_db, e.makeup_gain_db, t));
                let _ = self.set_comp_param(idx, 5, lerp_lin(f.knee_db, e.knee_db, t));
            } else {
                self.disable_compressor(idx);
            }
            if lerp_bool(fs.gate.is_some(), ts.gate.is_some(), t) {
                let f = fs
                    .gate
                    .as_ref()
                    .or(ts.gate.as_ref())
                    .expect("one side enabled");
                let e = ts.gate.as_ref().unwrap_or(f);
                let _ = self.enable_gate(idx);
                let _ = self.set_gate_param(idx, 0, lerp_lin(f.threshold_db, e.threshold_db, t));
                let _ = self.set_gate_param(idx, 1, lerp_lin(f.hysteresis_db, e.hysteresis_db, t));
                let _ = self.set_gate_param(idx, 2, lerp_lin(f.attack_ms, e.attack_ms, t));
                let _ = self.set_gate_param(idx, 3, lerp_lin(f.release_ms, e.release_ms, t));
                let _ = self.set_gate_param(idx, 4, lerp_lin(f.hold_ms, e.hold_ms, t));
            } else {
                self.disable_gate(idx);
            }
            if lerp_bool(fs.expander.is_some(), ts.expander.is_some(), t) {
                let f = fs
                    .expander
                    .as_ref()
                    .or(ts.expander.as_ref())
                    .expect("one side enabled");
                let e = ts.expander.as_ref().unwrap_or(f);
                let _ = self.enable_expander(idx);
                let _ =
                    self.set_expander_param(idx, 0, lerp_lin(f.threshold_db, e.threshold_db, t));
                let _ = self.set_expander_param(idx, 1, lerp_lin(f.ratio, e.ratio, t));
                let _ = self.set_expander_param(idx, 2, lerp_lin(f.attack_ms, e.attack_ms, t));
                let _ = self.set_expander_param(idx, 3, lerp_lin(f.release_ms, e.release_ms, t));
            } else {
                self.disable_expander(idx);
            }
        }
        // Foreign-solo guard FIRST (allocation-free fixed-console scan —
        // no collect): a live solo on a strip the scene doesn't cover
        // would leak into every covered strip's derived mute.
        for ch in 0..self.channel_ids.len() as u32 {
            if self.soloed_channels.contains(&ch)
                && !to.strips.get(ch as usize).is_some_and(|s| s.exists)
            {
                let _ = self.set_channel_solo(ch, false);
            }
        }
        // Mutes, then solos (solos last: the solo setter recomputes
        // every strip's derived engine mute). No-op writes are skipped —
        // the value in the set already equals the target.
        for (i, s) in to.strips.iter().enumerate() {
            if s.exists && self.user_muted.contains(&(i as u32)) != s.mute {
                let _ = self.set_channel_mute(i as u32, lerp_bool(from.strips[i].mute, s.mute, t));
            }
        }
        for (i, s) in to.strips.iter().enumerate() {
            if s.exists && self.soloed_channels.contains(&(i as u32)) != s.solo {
                let _ = self.set_channel_solo(i as u32, lerp_bool(from.strips[i].solo, s.solo, t));
            }
        }
        // Bus routing + tail controls: the scene fully owns all 8×16
        // slot assignments (a slot assigned after the save is cleared);
        // slot sources snap at the half-way point, tail params
        // interpolate.
        for b in 0..to.buses.len() {
            let fb = &from.buses[b];
            let tb = &to.buses[b];
            if mid_fade && fb == tb {
                continue;
            }
            for slot in 0..tb.sources.len() {
                let src = if t >= 0.5 {
                    tb.sources[slot]
                } else {
                    fb.sources[slot]
                };
                match src {
                    Some(ch) => {
                        let _ = self.set_bus_source(b as u32, slot as u32, ch);
                    }
                    None => self.clear_bus_source(b as u32, slot as u32),
                }
            }
            self.set_bus_gain(b as u32, lerp_gain_db(fb.gain, tb.gain, t));
            self.set_bus_mute(b as u32, lerp_bool(fb.muted, tb.muted, t));
            self.set_bus_feeds_main(b as u32, lerp_bool(fb.feeds_main, tb.feeds_main, t));
        }
        self.set_master_gain(lerp_gain_db(from.master_gain, to.master_gain, t));
    }

    /// Process one block through the full console (pure Rust, no JS
    /// interop — callable from native tests). Returns the interleaved
    /// stereo output slice (`block_size × 2` frames), valid until the
    /// next call.
    ///
    /// Flow per block (see the struct docs for the full architecture):
    /// FIFO drain → STAGING (dynamics + EQ per strip) → metering + tap →
    /// nine `process_mix_rt` engine calls → bus tail → master gain →
    /// limiter → interleave → master meter.
    ///
    /// Zero-allocation in the steady state: every heap buffer is
    /// pre-allocated and reused, and the engine call lists live on the
    /// STACK (a struct field cannot hold `&[f32]` slices into another
    /// field — self-referential — and raw-pointer tricks are off the
    /// table; fixed arrays of `Copy` tuples cost no heap).
    /// Regression-tested in `tests/alloc_test.rs`.
    pub fn process_block(&mut self, block_size: u32) -> Result<&[f32], JsValue> {
        let bs = self.buffer_size.min(block_size as usize).max(1);
        let buf = self.buffer_size;

        // ── Timed scene cross-fade: advance one block, apply ──
        // Audio-rate, block-granular (no timers/threads): t is the fade
        // position at THIS block's end (zero-order hold), so the fade
        // drives the gain stages the block below actually renders.
        // Allocation-free: the fade state is moved out and back (plain
        // memcpys of fixed-size scenes, no heap ops); the two snapshots
        // were allocated once at recall start. Completing the fade drops
        // them (one-time teardown). `fade_applying` lets the
        // interpolation reuse the public setters without tripping their
        // cancel-on-set.
        if let Some(mut fade) = self.fade.take() {
            fade.pos_ms += bs as f64 * 1000.0 / self.sample_rate as f64;
            let t = (fade.pos_ms / fade.dur_ms).clamp(0.0, 1.0) as f32;
            self.fade_applying = true;
            if t >= 1.0 {
                // Complete: apply the target exactly (lerp endpoints
                // short-circuit to the stored values verbatim).
                self.apply_scene_interp(&fade.to, &fade.to, 1.0);
            } else {
                self.apply_scene_interp(&fade.from, &fade.to, t);
                self.fade = Some(fade);
            }
            self.fade_applying = false;
        }

        // ── Channel direct-out tap: start a fresh block ──
        // Zeroed here so muted / solo-gated / input-less channels surface
        // as silence; the tap pass below skips muted strips (their slots
        // stay zeroed).
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

        // ── STAGING pass: raw source → dynamics → EQ, per existing strip ──
        // Every strip 0-255 that exists (has a channel id) stages its raw
        // source signal into `staged`: input strips their own drained row;
        // bus slots the RAW row of their assigned source channel (parallel
        // raw tap — the source's mute/fader/EQ/dynamics never touch the
        // bus path; pinned by tests/bus_parallel_test.rs). Unassigned
        // slots and missing sources stage silence; rows are re-zeroed
        // first so starvation and clear_bus_source can't replay stale
        // audio. The strip's own dynamics (gate → expander → compressor)
        // then EQ run in place — same order and math as before the
        // engine pivot. Unlike the old pass structure this runs for
        // muted strips too, so meters and compressor GR readback stay
        // live while muted (muted strips are excluded from the engine
        // call lists below and contribute to no mix).
        for idx in 0..self.channel_ids.len() {
            if self.channel_ids[idx].is_none() {
                continue;
            }
            // Which raw input row does this strip stage?
            let src = if idx < 128 {
                Some(idx as u32)
            } else {
                let bus = (idx - 128) / 16;
                let slot = (idx - 128) % 16;
                // Only input channels (0-127) have drained rows.
                self.bus_sources[bus][slot].filter(|&s| s < 128)
            };
            let base = idx * buf;
            let row = &mut self.staged[base..base + buf];
            match src {
                Some(s) => {
                    // The drain pass above writes rows at stride `bs`.
                    let sb = s as usize * bs;
                    row[..bs].copy_from_slice(&self.block_inputs[sb..sb + bs]);
                    if bs < buf {
                        row[bs..].fill(0.0);
                    }
                }
                None => row.fill(0.0),
            }
            let strip = idx as u32;
            if let Some(d) = self.dynamics_chains.get_mut(&strip) {
                d.process(&mut row[..bs]);
            }
            // EQ (always process unless explicitly bypassed)
            if let Some(eq) = self.eq_chains.get_mut(&strip) {
                if !eq.is_bypassed() {
                    eq.process(&mut row[..bs]);
                }
            }

            // Per-channel metering (post-dynamics, post-EQ, pre-fader)
            let peak = row[..bs].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
            let sq_sum: f32 = row[..bs].iter().map(|s| s * s).sum();
            let rms = (sq_sum / bs as f32).sqrt();
            self.channel_peak.insert(
                strip,
                if peak > 1e-10 {
                    20.0 * peak.log10()
                } else {
                    -200.0
                },
            );
            self.channel_rms.insert(
                strip,
                if rms > 1e-10 {
                    20.0 * rms.log10()
                } else {
                    -200.0
                },
            );
        }

        // Snapshot every existing strip's gain params into the reused
        // scratch vec — read from each strip's OWNING instance; covers
        // input strips 0-127 and bus slots 128-255 alike. Drives the
        // direct-out tap and the nine engine call lists.
        self.strip_params.clear();
        for idx in 0..self.channel_ids.len() as u32 {
            if self.channel_ids[idx as usize].is_some() {
                if let Some((id, p)) = self.mix_params_for(idx) {
                    self.strip_params.push((idx, id, p));
                }
            }
        }

        // ── Direct-out tap: capture the pan-stage input ──
        // staged holds the post gate/comp/EQ signal; apply the same
        // input gain × phase × fader × VCA the owning engine applies,
        // but NOT pan → the classic post-fader mono direct out.
        // (Muted/solo-gated strips skip; their slots stay zeroed above.)
        if self.tap_channels > 0 {
            let n = self.tap_channels as usize;
            for &(strip, id, params) in &self.strip_params {
                if strip as usize >= n || params.muted {
                    continue;
                }
                let vca = self
                    .owning_engine(strip)
                    .map(|e| e.engine().vca_gain_for_channel(id))
                    .unwrap_or(1.0);
                let phase: f32 = if params.phase_inverted { -1.0 } else { 1.0 };
                let g = db_to_linear(params.input_gain_db) * phase * params.fader_gain * vca;
                let base = strip as usize * buf;
                for i in 0..bs {
                    self.channel_tap_buf[i * n + strip as usize] = self.staged[base + i] * g;
                }
            }
        }

        // ── Build the nine engine call lists ──
        // Fixed-size STACK arrays of Copy tuples whose input slices point
        // into `staged` — zero heap allocation (a struct field cannot
        // hold references into another field of the same struct, so the
        // reused-Vec pattern used for `strip_params` is not possible
        // here; the arrays cost ~15 KB of stack per block). Main list:
        // strips 0-127 that exist, are not effectively muted, and have
        // main_assign. Bus i list: its 16 slots under the same
        // existence/mute rules.
        let dummy: (ChannelId, ChannelProcessParams, &[f32]) =
            (ChannelId::nil(), NULL_PROCESS_PARAMS, &[]);
        let mut main_list = [dummy; 128];
        let mut n_main = 0usize;
        let mut bus_lists = [[dummy; 16]; 8];
        let mut bus_counts = [0usize; 8];
        for &(strip, id, params) in &self.strip_params {
            if params.muted {
                continue;
            }
            let engine_params = ChannelProcessParams {
                fader_gain: params.fader_gain,
                pan: params.pan,
                muted: params.muted,
                input_gain_db: params.input_gain_db,
                phase_inverted: params.phase_inverted,
                pan_law: pan_law_to_engine(params.pan_law),
            };
            let base = strip as usize * buf;
            if strip < 128 {
                if self.main_assign[strip as usize] {
                    main_list[n_main] = (id, engine_params, &self.staged[base..base + buf]);
                    n_main += 1;
                }
            } else {
                let b = ((strip - 128) / 16) as usize;
                bus_lists[b][bus_counts[b]] = (id, engine_params, &self.staged[base..base + buf]);
                bus_counts[b] += 1;
            }
        }

        // ── Nine engine calls: the engines do ALL summing ──
        // process_mix_rt applies input gain/phase → effects → fader × VCA
        // → pan → PDC → sends → routing per channel and zeroes its output
        // buffers first, so an empty call list correctly produces
        // silence. Main outs → master; bus outs → the per-bus engine
        // outputs (the bus tail below applies gain/mute and feed master).
        self.main.engine_mut().process_mix_rt(
            &main_list[..n_main],
            &mut self.master_left,
            &mut self.master_right,
        );
        for b in 0..8 {
            self.buses[b].engine_mut().process_mix_rt(
                &bus_lists[b][..bus_counts[b]],
                &mut self.bus_left[b],
                &mut self.bus_right[b],
            );
        }

        // ── Bus tail: publish → meter → feed master ──
        // bus_pub is the bus's own stereo output (post-gain, mute →
        // silence): what take_bus_output drains and what feeds_main
        // accumulates into master. A muted bus contributes nowhere.
        for b in 0..8usize {
            let muted = self.bus_muted[b];
            let bg = self.bus_gains[b];
            for i in 0..bs {
                self.bus_pub_l[b][i] = if muted { 0.0 } else { self.bus_left[b][i] * bg };
                self.bus_pub_r[b][i] = if muted {
                    0.0
                } else {
                    self.bus_right[b][i] * bg
                };
            }

            // Bus metering on the published output (post-gain)
            let pl = &self.bus_pub_l[b];
            let pr = &self.bus_pub_r[b];
            let peak = pl[..bs]
                .iter()
                .chain(pr[..bs].iter())
                .map(|s| s.abs())
                .fold(0.0f32, f32::max);
            let sq: f32 = pl[..bs].iter().chain(pr[..bs].iter()).map(|s| s * s).sum();
            let rms = (sq / (bs as f32 * 2.0)).sqrt();
            self.bus_peak.insert(
                b as u32,
                if peak > 1e-10 {
                    20.0 * peak.log10()
                } else {
                    -200.0
                },
            );
            self.bus_rms.insert(
                b as u32,
                if rms > 1e-10 {
                    20.0 * rms.log10()
                } else {
                    -200.0
                },
            );

            if self.feeds_main[b] && !muted {
                for i in 0..bs {
                    self.master_left[i] += pl[i];
                    self.master_right[i] += pr[i];
                }
            }
        }
        self.bus_out_frames = bs;
        self.bus_out_ready = [true; 8];

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
    /// Clear the sticky master clip indicators (both channels).
    pub fn master_clear_clip(&mut self) {
        self.master_meter.clipped = [false; 2];
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
