use std::collections::{HashMap, HashSet};
use wasm_bindgen::prelude::*;

use oximedia_mixer::{
    bus::{BusId, BusType},
    channel::{ChannelType, PanLaw},
    ChannelId, ChannelProcessParams, MixerConfig, PanLawType,
};
use oximedia_audio::ChannelLayout;
use oximedia_mixer::metering::{Meter, MeterBallistics};
use oximedia_mixer::oversampled_limiter::OversampledLimiter;
use oximedia_mixer::effects_chain::AudioEffect;

pub mod effects;
use effects::{CompressorEffect, EqEffect, ExpanderEffect, GateEffect};

struct ChannelDynamics {
    compressor: Option<CompressorEffect>,
    gate: Option<GateEffect>,
    expander: Option<ExpanderEffect>,
}

impl Default for ChannelDynamics {
    fn default() -> Self { Self::new() }
}

impl ChannelDynamics {
    fn new() -> Self {
        Self { compressor: None, gate: None, expander: None }
    }

    fn process(&mut self, samples: &mut [f32]) {
        if let Some(g) = &mut self.gate { g.process(samples); }
        if let Some(e) = &mut self.expander { e.process(samples); }
        if let Some(c) = &mut self.compressor { c.process(samples); }
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
#[wasm_bindgen]
pub struct MixerWasm {
    engine: oximedia_mixer::AudioMixer,
    buffer_size: usize,
    channel_ids: Vec<Option<ChannelId>>,
    channel_inputs: HashMap<u32, Vec<f32>>,
    pid_map: HashMap<u16, PidMapping>,
    master_meter: Meter,
    // ── Pre-allocated scratch buffers ──
    master_left: Vec<f32>,
    master_right: Vec<f32>,
    stereo_out: Vec<f32>,
    eq_scratch: Vec<f32>,
    eq_scratch2: Vec<f32>,
    deinterleave_scratch: Vec<Vec<f32>>,
    raw_input: Vec<f32>,
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
    // Slot stereo input buffers (128 slots; written by pass 1, consumed by pass 2)
    slot_in_l: Vec<Vec<f32>>,
    slot_in_r: Vec<Vec<f32>>,
    bus_peak: HashMap<u32, f32>,
    bus_rms: HashMap<u32, f32>,
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
    pub fn new(sample_rate: u32, buffer_size: u32, max_channels: u32) -> Result<MixerWasm, JsValue> {
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
            pid_map: HashMap::new(),
            master_meter: Meter::new(2, sample_rate, MeterBallistics::Fast),
            master_left: vec![0.0; bs],
            master_right: vec![0.0; bs],
            stereo_out: vec![0.0; bs * 2],
            eq_scratch: vec![0.0; bs],
            eq_scratch2: vec![0.0; bs],
            deinterleave_scratch: Vec::new(),
            raw_input: Vec::new(),
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
            slot_in_l: (0..128).map(|_| vec![0.0; bs]).collect(),
            slot_in_r: (0..128).map(|_| vec![0.0; bs]).collect(),
            bus_peak: HashMap::new(),
            bus_rms: HashMap::new(),
            unmapped_pid_drops: 0,
            sample_rate,
        })
    }

    fn ensure_channel(&mut self, idx: u32) -> Result<ChannelId, JsValue> {
        let i = idx as usize;
        if i >= self.channel_ids.len() {
            return Err(JsValue::from_str(&format!(
                "channel index {idx} out of range (max {})", self.channel_ids.len()
            )));
        }
        if self.channel_ids[i].is_none() {
            let id = self.engine.add_channel(
                format!("ch{idx}"), ChannelType::Mono, ChannelLayout::Mono,
            ).map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
            if let Ok(ch) = self.engine.get_channel_mut(id) {
                ch.set_pan_law(PanLaw::Linear);
            }
            self.channel_ids[i] = Some(id);
            self.eq_chains.insert(idx, EqEffect::six_band(self.sample_rate));
            self.dynamics_chains.insert(idx, ChannelDynamics::new());
        }
        Ok(self.channel_ids[i].unwrap())
    }

    /// Build engine process params for a channel index (input or slot).
    fn params_for(&self, ch_idx: u32) -> Option<(ChannelId, ChannelProcessParams)> {
        let id = self.channel_ids.get(ch_idx as usize)?.as_ref().copied()?;
        let ch = self.engine.get_channel(id).ok()?;
        let pan_law = match ch.pan_law() {
            PanLaw::Linear => PanLawType::Linear,
            PanLaw::Minus3dB => PanLawType::Minus3dB,
            PanLaw::Minus4Dot5dB => PanLawType::Minus4Dot5dB,
            PanLaw::Minus6dB => PanLawType::Minus6dB,
        };
        let params = ChannelProcessParams {
            fader_gain: ch.gain(), pan: ch.pan(), muted: ch.is_muted(),
            input_gain_db: ch.input().gain_db, phase_inverted: ch.is_phase_inverted(),
            pan_law,
        };
        Some((id, params))
    }

    // ── Input ──────────────────────────────────────────

    pub fn set_channel_input(&mut self, ch: u32, data: &js_sys::Float32Array) -> Result<(), JsValue> {
        self.ensure_channel(ch)?;
        let len = data.length() as usize;
        self.raw_input.resize(len, 0.0);
        data.copy_to(&mut self.raw_input);
        let slot = self.channel_inputs.entry(ch).or_insert_with(|| Vec::with_capacity(self.buffer_size));
        slot.clear();
        slot.extend_from_slice(&self.raw_input);
        Ok(())
    }

    pub fn set_channel_input_interleaved(
        &mut self, ch_start: u32, data: &js_sys::Float32Array, num_channels: u32,
    ) -> Result<(), JsValue> {
        let nc = num_channels as usize;
        if nc == 0 { return Err(JsValue::from_str("num_channels must be > 0")); }
        let total = data.length() as usize;
        if !total.is_multiple_of(nc) {
            return Err(JsValue::from_str(&format!("length {total} not divisible by {nc}")));
        }
        let frames = total / nc;
        if self.deinterleave_scratch.len() < nc {
            self.deinterleave_scratch.resize(nc, Vec::new());
        }
        for c in 0..nc { self.deinterleave_scratch[c].clear(); self.deinterleave_scratch[c].reserve(frames); }
        self.raw_input.resize(total, 0.0);
        data.copy_to(&mut self.raw_input);
        for f in 0..frames {
            for c in 0..nc { self.deinterleave_scratch[c].push(self.raw_input[f * nc + c]); }
        }
        for c in 0..nc {
            let ch = ch_start + c as u32;
            self.ensure_channel(ch)?;
            let slot = self.channel_inputs.entry(ch).or_insert_with(|| Vec::with_capacity(self.buffer_size));
            std::mem::swap(slot, &mut self.deinterleave_scratch[c]);
        }
        Ok(())
    }

    // ── PID mapping ────────────────────────────────────

    pub fn map_pid(&mut self, pid: u16, ch_start: u32, channel_count: u32) -> Result<(), JsValue> {
        for i in 0..channel_count { self.ensure_channel(ch_start + i)?; }
        self.pid_map.insert(pid, PidMapping { ch_start, channel_count, subscribed: true });
        Ok(())
    }
    pub fn unmap_pid(&mut self, pid: u16) { self.pid_map.remove(&pid); }
    pub fn pid_channel(&self, pid: u16) -> i32 { self.pid_map.get(&pid).map(|m| m.ch_start as i32).unwrap_or(-1) }
    pub fn pid_channel_count(&self, pid: u16) -> u32 { self.pid_map.get(&pid).map(|m| m.channel_count).unwrap_or(0) }
    pub fn subscribe_pid(&mut self, pid: u16) {
        if let Some(m) = self.pid_map.get_mut(&pid) {
            m.subscribed = true;
            for i in 0..m.channel_count {
                if let Some(&Some(id)) = self.channel_ids.get((m.ch_start + i) as usize) {
                    if let Ok(ch) = self.engine.get_channel_mut(id) { ch.set_muted(false); }
                }
            }
        }
    }
    pub fn unsubscribe_pid(&mut self, pid: u16) {
        if let Some(m) = self.pid_map.get_mut(&pid) {
            m.subscribed = false;
            for i in 0..m.channel_count {
                if let Some(&Some(id)) = self.channel_ids.get((m.ch_start + i) as usize) {
                    if let Ok(ch) = self.engine.get_channel_mut(id) { ch.set_muted(true); }
                }
            }
        }
    }

    pub fn feed_pcm(&mut self, pid: u16, data: &js_sys::Float32Array) -> Result<(), JsValue> {
        let Some(mapping) = self.pid_map.get(&pid).copied() else {
            self.unmapped_pid_drops += 1;
            return Ok(());
        };
        if !mapping.subscribed { return Ok(()); }
        self.set_channel_input_interleaved(mapping.ch_start, data, mapping.channel_count)
    }

    /// Count of PCM packets dropped due to unmapped PID (for diagnostics).
    pub fn unmapped_pid_count(&self) -> u64 { self.unmapped_pid_drops }

    // ── Channel controls ───────────────────────────────

    pub fn set_channel_gain(&mut self, ch: u32, gain: f32) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        self.engine.set_channel_gain(id, gain).map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }
    pub fn set_channel_pan(&mut self, ch: u32, pan: f32) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        self.engine.set_channel_pan(id, pan).map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }
    pub fn set_channel_mute(&mut self, ch: u32, muted: bool) -> Result<(), JsValue> {
        let _ = self.ensure_channel(ch)?;
        if muted { self.user_muted.insert(ch); } else { self.user_muted.remove(&ch); }
        // Apply: muted if user-muted OR (something is soloed and this isn't)
        let effective = muted || (self.solo_active() && !self.soloed_channels.contains(&ch));
        self.set_engine_mute(ch, effective);
        Ok(())
    }
    pub fn set_channel_solo(&mut self, ch: u32, soloed: bool) -> Result<(), JsValue> {
        let _ = self.ensure_channel(ch)?;
        if soloed { self.soloed_channels.insert(ch); } else { self.soloed_channels.remove(&ch); }
        // Re-evaluate all channels' effective mute state
        let ids: Vec<(u32, ChannelId)> = self.channel_ids.iter().enumerate()
            .filter_map(|(i, id)| id.map(|id| (i as u32, id))).collect();
        for (i, id) in ids {
            let effective = self.user_muted.contains(&i) || (self.solo_active() && !self.soloed_channels.contains(&i));
            if let Ok(ch) = self.engine.get_channel_mut(id) { ch.set_muted(effective); }
        }
        Ok(())
    }

    fn solo_active(&self) -> bool { !self.soloed_channels.is_empty() }

    fn set_engine_mute(&mut self, ch: u32, muted: bool) {
        if let Some(&Some(id)) = self.channel_ids.get(ch as usize) {
            if let Ok(channel) = self.engine.get_channel_mut(id) { channel.set_muted(muted); }
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
        if let Some(eq) = self.eq_chains.get_mut(&ch) { eq.set_bypassed(bypassed); }
        Ok(())
    }

    // ── Dynamics controls ──────────────────────────────

    /// Enable compressor on a channel with broadcast defaults (-12 dB threshold, 3:1 ratio).
    pub fn enable_compressor(&mut self, ch: u32) -> Result<(), JsValue> {
        let _ = self.ensure_channel(ch)?;
        self.dynamics_chains.entry(ch).or_default().compressor = Some(CompressorEffect::broadcast(self.sample_rate));
        Ok(())
    }
    pub fn disable_compressor(&mut self, ch: u32) {
        if let Some(d) = self.dynamics_chains.get_mut(&ch) { d.compressor = None; }
    }
    pub fn enable_gate(&mut self, ch: u32) -> Result<(), JsValue> {
        let _ = self.ensure_channel(ch)?;
        self.dynamics_chains.entry(ch).or_default().gate = Some(GateEffect::denoise(self.sample_rate));
        Ok(())
    }
    pub fn disable_gate(&mut self, ch: u32) {
        if let Some(d) = self.dynamics_chains.get_mut(&ch) { d.gate = None; }
    }

    pub fn enable_expander(&mut self, ch: u32) -> Result<(), JsValue> {
        let _ = self.ensure_channel(ch)?;
        self.dynamics_chains.entry(ch).or_default().expander = Some(ExpanderEffect::gentle(self.sample_rate));
        Ok(())
    }
    pub fn disable_expander(&mut self, ch: u32) {
        if let Some(d) = self.dynamics_chains.get_mut(&ch) { d.expander = None; }
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
        if let Ok(c) = self.engine.get_channel_mut(id) { c.input_mut().gain_db = gain_db; }
        Ok(())
    }

    pub fn set_channel_phase(&mut self, ch: u32, inverted: bool) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        if let Ok(c) = self.engine.get_channel_mut(id) { c.set_phase_inverted(inverted); }
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
        if let Ok(c) = self.engine.get_channel_mut(id) { c.set_pan_law(pan_law); }
        Ok(())
    }

    pub fn set_channel_name(&mut self, ch: u32, name: String) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        if let Ok(c) = self.engine.get_channel_mut(id) { c.set_name(name); }
        Ok(())
    }

    // ── Per-channel metering ───────────────────────────

    pub fn channel_peak_db(&self, ch: u32) -> f32 {
        self.channel_peak.get(&ch).copied().unwrap_or(-f32::INFINITY)
    }
    pub fn channel_rms_db(&self, ch: u32) -> f32 {
        self.channel_rms.get(&ch).copied().unwrap_or(-f32::INFINITY)
    }
    pub fn channel_meters_json(&self) -> String {
        let mut s = String::from("[");
        let mut first = true;
        for (&ch, &peak) in &self.channel_peak {
            if !first { s.push(','); }
            first = false;
            let rms = self.channel_rms.get(&ch).copied().unwrap_or(-200.0);
            s.push_str(&format!("{{\"ch\":{ch},\"peak\":{peak:.1},\"rms\":{rms:.1}}}"));
        }
        s.push(']');
        s
    }

    // ── Master controls ────────────────────────────────

    pub fn set_master_gain(&mut self, gain: f32) { self.master_gain = gain.clamp(0.0, 2.0); }

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
        let bus_id = self.engine.add_bus(name, bt, ChannelLayout::Stereo)
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
        let js_id = self.bus_counter;
        self.bus_counter += 1;
        self.bus_map.insert(js_id, bus_id);
        Ok(js_id)
    }

    pub fn route_channel_to_bus(&mut self, ch: u32, bus_id: u32) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        let Some(&bid) = self.bus_map.get(&bus_id) else {
            return Err(JsValue::from_str("unknown bus"));
        };
        self.engine.route_channel_to_bus(id, bid).map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    pub fn route_channel_to_master(&mut self, ch: u32) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        self.engine.route_channel_to_master(id).map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    pub fn set_aux_send(&mut self, ch: u32, _send_idx: u32, bus_id: u32, level: f32, pre_fader: bool) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        let Some(&bid) = self.bus_map.get(&bus_id) else {
            return Err(JsValue::from_str("unknown bus"));
        };
        self.engine.add_aux_send(id, bid, level, pre_fader).map_err(|e| JsValue::from_str(&format!("{e:?}")))
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
        if bus >= 8 { return Err(JsValue::from_str("bus index out of range (max 8)")); }
        if slot >= 16 { return Err(JsValue::from_str("slot out of range (max 16)")); }
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
        if (bus as usize) < 8 { self.bus_gains[bus as usize] = gain.clamp(0.0, 2.0); }
    }
    pub fn set_bus_mute(&mut self, bus: u32, muted: bool) {
        if (bus as usize) < 8 { self.bus_muted[bus as usize] = muted; }
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
            if i > 0 { s.push(','); }
            let peak = self.bus_peak.get(&i).copied().unwrap_or(-200.0);
            let rms = self.bus_rms.get(&i).copied().unwrap_or(-200.0);
            s.push_str(&format!("{{\"bus\":{i},\"peak\":{peak:.1},\"rms\":{rms:.1}}}"));
        }
        s.push(']');
        s
    }

    // ── Master limiter ─────────────────────────────────

    pub fn set_limiter_enabled(&mut self, enabled: bool) { self.limiter_enabled = enabled; }
    pub fn limiter_gain_reduction_db(&self) -> f32 { self.limiter_l.gain_reduction_db() }

    // ── Processing ─────────────────────────────────────

    pub fn process(&mut self, block_size: u32) -> Result<js_sys::Float32Array, JsValue> {
        let bs = self.buffer_size.min(block_size as usize).max(1);

        // Clear master bus
        for i in 0..self.buffer_size { self.master_left[i] = 0.0; self.master_right[i] = 0.0; }

        // Clear bus accumulators and slot input buffers
        for b in 0..8 {
            for s in 0..self.buffer_size {
                self.bus_left[b][s] = 0.0;
                self.bus_right[b][s] = 0.0;
            }
        }
        for s in 0..128 {
            for i in 0..self.buffer_size {
                self.slot_in_l[s][i] = 0.0;
                self.slot_in_r[s][i] = 0.0;
            }
        }

        // Build reverse mapping: input channel → slot index (if assigned to a bus slot)
        let mut ch_to_slot: HashMap<u32, u32> = HashMap::new();
        for (bus_idx, slots) in self.bus_sources.iter().enumerate() {
            for (slot_idx, ch) in slots.iter().enumerate() {
                if let Some(&ch) = ch.as_ref() {
                    ch_to_slot.insert(ch, 128 + (bus_idx as u32) * 16 + slot_idx as u32);
                }
            }
        }

        // Collect channel data upfront (avoid borrow conflicts).
        // Pass 1 covers input channels only (indices 0-127); slot channels
        // (128-255) are handled in pass 2 below.
        let ch_data: Vec<(u32, ChannelId, ChannelProcessParams)> = self.channel_inputs.keys()
            .filter(|&&ch_idx| ch_idx < 128)
            .filter_map(|&ch_idx| self.params_for(ch_idx).map(|(id, p)| (ch_idx, id, p)))
            .collect();

        // ── Pass 1: input channels ──
        for (ch_idx, id, params) in ch_data {
            if params.muted { continue; }

            // Get this channel's input samples
            let samples = match self.channel_inputs.get(&ch_idx) {
                Some(s) => s.as_slice(),
                None => continue,
            };

            // ── Gate → Compressor → EQ (in-place on scratch) ──
            // Copy to pre-allocated scratch (no per-channel Vec alloc)
            self.eq_scratch[..bs].copy_from_slice(&samples[..bs]);

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
            let peak = self.eq_scratch[..bs].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
            let sq_sum: f32 = self.eq_scratch[..bs].iter().map(|s| s * s).sum();
            let rms = (sq_sum / bs as f32).sqrt();
            self.channel_peak.insert(ch_idx, if peak > 1e-10 { 20.0 * peak.log10() } else { -200.0 });
            self.channel_rms.insert(ch_idx, if rms > 1e-10 { 20.0 * rms.log10() } else { -200.0 });

            // Engine: input gain, fader, pan → stereo pair
            let (ch_left, ch_right) = self.engine.engine_mut()
                .process_mix(&[(id, params)], &self.eq_scratch[..bs]);

            // Route: if this channel feeds a bus slot, write to the slot's
            // stereo input; else go directly to master.
            if let Some(&slot_idx) = ch_to_slot.get(&ch_idx) {
                let si = (slot_idx - 128) as usize;
                for i in 0..bs {
                    self.slot_in_l[si][i] += ch_left[i];
                    self.slot_in_r[si][i] += ch_right[i];
                }
            } else {
                for i in 0..bs {
                    self.master_left[i] += ch_left[i];
                    self.master_right[i] += ch_right[i];
                }
            }
        }

        // ── Pass 2: bus slots (indices 128-255) are full channel strips ──
        // Each slot receives a stereo signal (its assigned inputs' processed
        // output), runs dynamics → EQ, then fader/pan via the engine, and
        // feeds its bus accumulator.
        for bus_idx in 0..8u32 {
            for slot in 0..16u32 {
                let slot_idx = 128 + bus_idx * 16 + slot;
                let si = slot_idx as usize - 128;

                let Some((id, params)) = self.params_for(slot_idx) else { continue; };
                if params.muted { continue; }

                // Process L through the slot's dynamics → EQ into eq_scratch2
                self.eq_scratch2[..bs].copy_from_slice(&self.slot_in_l[si][..bs]);
                if let Some(d) = self.dynamics_chains.get_mut(&slot_idx) {
                    d.process(&mut self.eq_scratch2[..bs]);
                }
                if let Some(eq) = self.eq_chains.get_mut(&slot_idx) {
                    if !eq.is_bypassed() {
                        eq.process(&mut self.eq_scratch2[..bs]);
                    }
                }

                // Process R through the same dynamics → EQ into eq_scratch
                self.eq_scratch[..bs].copy_from_slice(&self.slot_in_r[si][..bs]);
                if let Some(d) = self.dynamics_chains.get_mut(&slot_idx) {
                    d.process(&mut self.eq_scratch[..bs]);
                }
                if let Some(eq) = self.eq_chains.get_mut(&slot_idx) {
                    if !eq.is_bypassed() {
                        eq.process(&mut self.eq_scratch[..bs]);
                    }
                }

                // Slot metering (post-dynamics, post-EQ, pre-fader, over L+R)
                let peak = self.eq_scratch2[..bs].iter().chain(self.eq_scratch[..bs].iter())
                    .map(|s| s.abs()).fold(0.0f32, f32::max);
                let sq_sum: f32 = self.eq_scratch2[..bs].iter().chain(self.eq_scratch[..bs].iter())
                    .map(|s| s * s).sum();
                let rms = (sq_sum / (bs as f32 * 2.0)).sqrt();
                self.channel_peak.insert(slot_idx, if peak > 1e-10 { 20.0 * peak.log10() } else { -200.0 });
                self.channel_rms.insert(slot_idx, if rms > 1e-10 { 20.0 * rms.log10() } else { -200.0 });

                // Engine: input gain, fader, pan on L and R separately, then
                // sum. process_mix is linear, so the result is the pan-
                // weighted sum of both halves of the stereo input.
                let bi = bus_idx as usize;
                let (l1, r1) = self.engine.engine_mut()
                    .process_mix(&[(id, params.clone())], &self.eq_scratch2[..bs]);
                for i in 0..bs {
                    self.bus_left[bi][i] += l1[i];
                    self.bus_right[bi][i] += r1[i];
                }
                let (l2, r2) = self.engine.engine_mut()
                    .process_mix(&[(id, params)], &self.eq_scratch[..bs]);
                for i in 0..bs {
                    self.bus_left[bi][i] += l2[i];
                    self.bus_right[bi][i] += r2[i];
                }
            }
        }

        // ── Pass 3: buses to master ──
        for bus_idx in 0..8u32 {
            if self.bus_muted[bus_idx as usize] { continue; }

            // Bus metering on the accumulator (sum of slot outputs, pre-gain)
            let bl = &self.bus_left[bus_idx as usize];
            let br = &self.bus_right[bus_idx as usize];
            let peak = bl[..bs].iter().chain(br[..bs].iter())
                .map(|s| s.abs()).fold(0.0f32, f32::max);
            let sq: f32 = bl[..bs].iter().chain(br[..bs].iter()).map(|s| s * s).sum();
            let rms = (sq / (bs as f32 * 2.0)).sqrt();
            self.bus_peak.insert(bus_idx, if peak > 1e-10 { 20.0 * peak.log10() } else { -200.0 });
            self.bus_rms.insert(bus_idx, if rms > 1e-10 { 20.0 * rms.log10() } else { -200.0 });

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

        self.master_meter.process(&self.stereo_out[..bs * 2]);

        let out = js_sys::Float32Array::new_with_length((bs * 2) as u32);
        out.copy_from(&self.stereo_out[..bs * 2]);
        Ok(out)
    }

    // ── Metering ───────────────────────────────────────

    pub fn master_peak_db_l(&self) -> f32 {
        self.master_meter.data().peak.first().map(|p| p.current_db).unwrap_or(-f32::INFINITY)
    }
    pub fn master_peak_db_r(&self) -> f32 {
        self.master_meter.data().peak.get(1).map(|p| p.current_db).unwrap_or(-f32::INFINITY)
    }
    pub fn master_rms_db_l(&self) -> f32 {
        self.master_meter.data().rms.first().map(|r| r.current_db).unwrap_or(-f32::INFINITY)
    }
    pub fn master_rms_db_r(&self) -> f32 {
        self.master_meter.data().rms.get(1).map(|r| r.current_db).unwrap_or(-f32::INFINITY)
    }
    pub fn master_clipping(&self) -> bool {
        self.master_meter.data().peak.iter().any(|p| p.clipped)
    }
}
