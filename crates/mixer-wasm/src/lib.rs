use std::collections::HashMap;
use wasm_bindgen::prelude::*;

use oximedia_mixer::{
    channel::{ChannelType, PanLaw},
    ChannelId, ChannelProcessParams, MixerConfig, PanLawType,
};
use oximedia_audio::ChannelLayout;
use oximedia_mixer::metering::{Meter, MeterBallistics};

pub mod effects;
use effects::EqEffect;

use oximedia_mixer::effects_chain::AudioEffect;

/// WASM binding for the oximedia-mixer audio engine.
///
/// Construction: `new(sample_rate, block_size, max_channels)`.
///
/// # PCM transport contract
///
/// Audio arrives from the WebSRT demuxer as **Float32 interleaved** per PID
/// (i32→f32 conversion done in the demuxer, not JS). 48 kHz is fixed.
/// PTS comes from PES PTS (s302m, ffmpeg-populated).
///
/// Two input modes:
/// - `set_channel_input(ch, data)` — mono planar Float32 (one channel).
/// - `set_channel_input_interleaved(ch_start, data, num_channels)` —
///   interleaved stereo/multichannel, de-interleaved into consecutive
///   mixer channels starting at `ch_start`.
///
/// PID mapping:
/// - `map_pid(pid, ch_start, channel_count)` — route a TS PID to mixer channels.
/// - `unmap_pid(pid)` — remove mapping (idempotent, for mid-stream reconfig).
///
/// Process via `process(block_size)` → interleaved stereo Float32Array.
///
/// # Per-channel input architecture
///
/// The engine's `process()` feeds the SAME input to every channel.
/// We resolve this at the binding layer by calling `engine.process_mix()`
/// once per active channel with that channel's own input, then summing
/// the master outputs.
#[wasm_bindgen]
pub struct MixerWasm {
    engine: oximedia_mixer::AudioMixer,
    buffer_size: usize,
    /// Maps JS-visible channel index → engine ChannelId (UUID).
    channel_ids: Vec<Option<ChannelId>>,
    /// Pending per-channel input audio (mono f32).
    channel_inputs: HashMap<u32, Vec<f32>>,
    /// Maps TS PID → PID mapping info (for pidmap events).
    pid_map: HashMap<u16, PidMapping>,
    /// Master output meter (stereo peak/RMS).
    master_meter: Meter,
    // ── Pre-allocated scratch buffers (zero per-call allocation) ──
    master_left: Vec<f32>,
    master_right: Vec<f32>,
    stereo_out: Vec<f32>,
    /// Reusable de-interleave scratch.
    deinterleave_scratch: Vec<Vec<f32>>,
    /// Reusable raw input copy buffer.
    raw_input: Vec<f32>,
    /// Per-channel solo state.
    soloed_channels: std::collections::HashSet<u32>,
    /// Per-channel EQ instances (for parameter control).
    eq_chains: HashMap<u32, EqEffect>,
    /// Sample rate (for creating EQ effects).
    sample_rate: u32,
}

/// Per-PID mapping metadata (from MPEG-2 component descriptors).
#[derive(Clone, Copy, Debug)]
struct PidMapping {
    ch_start: u32,
    channel_count: u32,
    subscribed: bool,
}

#[wasm_bindgen]
impl MixerWasm {
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        sample_rate: u32,
        buffer_size: u32,
        max_channels: u32,
    ) -> Result<MixerWasm, JsValue> {
        console_error_panic_hook::set_once();

        let bs = buffer_size as usize;
        let config = MixerConfig {
            sample_rate,
            buffer_size: bs,
            max_channels: max_channels as usize,
            ..Default::default()
        };
        let engine = oximedia_mixer::AudioMixer::new(config);

        Ok(MixerWasm {
            engine,
            buffer_size: bs,
            channel_ids: vec![None; max_channels as usize],
            channel_inputs: HashMap::new(),
            pid_map: HashMap::new(),
            master_meter: Meter::new(2, sample_rate, MeterBallistics::Fast),
            master_left: vec![0.0; bs],
            master_right: vec![0.0; bs],
            stereo_out: vec![0.0; bs * 2],
            deinterleave_scratch: Vec::new(),
            raw_input: Vec::new(),
            soloed_channels: std::collections::HashSet::new(),
            eq_chains: HashMap::new(),
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
                .add_channel(
                    format!("ch{idx}"),
                    ChannelType::Mono,
                    ChannelLayout::Mono,
                )
                .map_err(|e| JsValue::from_str(&format!("{e:?}")))?;
            if let Ok(ch) = self.engine.get_channel_mut(id) {
                ch.set_pan_law(PanLaw::Linear);
            }
            self.channel_ids[i] = Some(id);
            // Initialize 6-band EQ for this channel.
            if !self.eq_chains.contains_key(&idx) {
                let eq = EqEffect::six_band(self.sample_rate);
                self.eq_chains.insert(idx, eq);
            }
        }
        Ok(self.channel_ids[i].unwrap())
    }

    // ------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------

    pub fn set_channel_input(&mut self, ch: u32, data: &js_sys::Float32Array) -> Result<(), JsValue> {
        self.ensure_channel(ch)?;
        let len = data.length() as usize;
        self.raw_input.resize(len, 0.0);
        data.copy_to(&mut self.raw_input);
        // Move into channel_inputs by swap (avoids Vec alloc if slot exists).
        let slot = self.channel_inputs.entry(ch).or_insert_with(|| Vec::with_capacity(self.buffer_size));
        slot.clear();
        slot.extend_from_slice(&self.raw_input);
        Ok(())
    }

    pub fn set_channel_input_interleaved(
        &mut self,
        ch_start: u32,
        data: &js_sys::Float32Array,
        num_channels: u32,
    ) -> Result<(), JsValue> {
        let nc = num_channels as usize;
        if nc == 0 {
            return Err(JsValue::from_str("num_channels must be > 0"));
        }

        let total = data.length() as usize;
        if total % nc != 0 {
            return Err(JsValue::from_str(&format!(
                "interleaved data length {total} not divisible by num_channels {nc}"
            )));
        }

        let frames = total / nc;

        // Ensure scratch has nc channels, each with capacity frames.
        if self.deinterleave_scratch.len() < nc {
            self.deinterleave_scratch.resize(nc, Vec::new());
        }
        for c in 0..nc {
            self.deinterleave_scratch[c].clear();
            self.deinterleave_scratch[c].reserve(frames);
        }

        // Copy raw input.
        self.raw_input.resize(total, 0.0);
        data.copy_to(&mut self.raw_input);

        // De-interleave into scratch.
        for f in 0..frames {
            for c in 0..nc {
                self.deinterleave_scratch[c].push(self.raw_input[f * nc + c]);
            }
        }

        // Assign to consecutive mixer channels.
        for c in 0..nc {
            let ch = ch_start + c as u32;
            self.ensure_channel(ch)?;
            let slot = self.channel_inputs.entry(ch).or_insert_with(|| Vec::with_capacity(self.buffer_size));
            std::mem::swap(slot, &mut self.deinterleave_scratch[c]);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // PID mapping
    // ------------------------------------------------------------------

    pub fn map_pid(
        &mut self,
        pid: u16,
        ch_start: u32,
        channel_count: u32,
    ) -> Result<(), JsValue> {
        for i in 0..channel_count {
            self.ensure_channel(ch_start + i)?;
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
        self.pid_map
            .get(&pid)
            .map(|m| m.channel_count)
            .unwrap_or(0)
    }

    pub fn subscribe_pid(&mut self, pid: u16) {
        if let Some(m) = self.pid_map.get_mut(&pid) {
            m.subscribed = true;
            let ch_start = m.ch_start;
            let count = m.channel_count;
            for i in 0..count {
                if let Some(&Some(id)) = self.channel_ids.get((ch_start + i) as usize) {
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
            let ch_start = m.ch_start;
            let count = m.channel_count;
            for i in 0..count {
                if let Some(&Some(id)) = self.channel_ids.get((ch_start + i) as usize) {
                    if let Ok(ch) = self.engine.get_channel_mut(id) {
                        ch.set_muted(true);
                    }
                }
            }
        }
    }

    pub fn feed_pcm(
        &mut self,
        pid: u16,
        data: &js_sys::Float32Array,
    ) -> Result<(), JsValue> {
        let Some(mapping) = self.pid_map.get(&pid).copied() else {
            return Ok(());
        };
        if !mapping.subscribed {
            return Ok(());
        }
        self.set_channel_input_interleaved(mapping.ch_start, data, mapping.channel_count)
    }

    // ------------------------------------------------------------------
    // Channel controls
    // ------------------------------------------------------------------

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
        let id = self.ensure_channel(ch)?;
        if let Ok(channel) = self.engine.get_channel_mut(id) {
            channel.set_muted(muted);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // EQ controls
    // ------------------------------------------------------------------

    /// Set EQ band gain (dB) for a channel's 6-band EQ.
    /// Band 0=HPF, 1=Low, 2=Lo-Mid, 3=Mid, 4=Hi-Mid, 5=High.
    pub fn set_eq_band_gain(&mut self, ch: u32, band: usize, gain_db: f32) -> Result<(), JsValue> {
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            if band < eq.inner().bands.len() {
                eq.inner_mut().bands[band].set_gain_db(gain_db as f64);
                eq.inner_mut().bands[band].update_coefficients();
            }
        }
        Ok(())
    }

    /// Set EQ band frequency (Hz) for a channel.
    pub fn set_eq_band_freq(&mut self, ch: u32, band: usize, freq_hz: f32) -> Result<(), JsValue> {
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            if band < eq.inner().bands.len() {
                eq.inner_mut().bands[band].set_frequency(freq_hz as f64);
                eq.inner_mut().bands[band].update_coefficients();
            }
        }
        Ok(())
    }

    /// Set EQ band Q for a channel.
    pub fn set_eq_band_q(&mut self, ch: u32, band: usize, q: f32) -> Result<(), JsValue> {
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            if band < eq.inner().bands.len() {
                eq.inner_mut().bands[band].set_q(q as f64);
                eq.inner_mut().bands[band].update_coefficients();
            }
        }
        Ok(())
    }

    /// Bypass/unbypass EQ for a channel.
    pub fn set_eq_bypass(&mut self, ch: u32, bypassed: bool) -> Result<(), JsValue> {
        if let Some(eq) = self.eq_chains.get_mut(&ch) {
            eq.set_bypassed(bypassed);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Solo
    // ------------------------------------------------------------------

    /// Solo a channel (mutes all others).
    pub fn set_channel_solo(&mut self, ch: u32, soloed: bool) -> Result<(), JsValue> {
        if soloed {
            self.soloed_channels.insert(ch);
        } else {
            self.soloed_channels.remove(&ch);
        }
        // Apply: if any channel is soloed, mute all non-soloed channels.
        let any_soloed = !self.soloed_channels.is_empty();
        let ids_to_update: Vec<(usize, ChannelId, bool)> = self.channel_ids.iter().enumerate()
            .filter_map(|(i, id_opt)| {
                id_opt.map(|id| {
                    let should_mute = any_soloed && !self.soloed_channels.contains(&(i as u32));
                    (i, id, should_mute)
                })
            })
            .collect();
        for (_, id, should_mute) in ids_to_update {
            if let Ok(channel) = self.engine.get_channel_mut(id) {
                if should_mute {
                    channel.set_muted(true);
                } else {
                    channel.set_muted(false);
                }
            }
        }
        Ok(())
    }

    /// Check if a channel is user-muted (not solo-muted).
    fn user_muted(&self, ch: u32) -> bool {
        // Check if the channel's mute state was set by the user.
        // For now, track via a simple heuristic: if not soloed and not in
        // soloed set, check PID subscription. This is simplified.
        false
    }

    // ------------------------------------------------------------------
    // Processing (allocation-free hot path)
    // ------------------------------------------------------------------

    pub fn process(&mut self, _block_size: u32) -> Result<js_sys::Float32Array, JsValue> {
        let bs = self.buffer_size;

        // Clear master bus (reuse pre-allocated buffers).
        for i in 0..bs {
            self.master_left[i] = 0.0;
            self.master_right[i] = 0.0;
        }

        for (&ch_idx, samples) in &self.channel_inputs {
            let Some(&Some(id)) = self.channel_ids.get(ch_idx as usize) else {
                continue;
            };



            let params = if let Ok(ch) = self.engine.get_channel(id) {
                let pan_law = match ch.pan_law() {
                    PanLaw::Linear => PanLawType::Linear,
                    PanLaw::Minus3dB => PanLawType::Minus3dB,
                    PanLaw::Minus4Dot5dB => PanLawType::Minus4Dot5dB,
                    PanLaw::Minus6dB => PanLawType::Minus6dB,
                };
                ChannelProcessParams {
                    fader_gain: ch.gain(),
                    pan: ch.pan(),
                    muted: ch.is_muted(),
                    input_gain_db: ch.input().gain_db,
                    phase_inverted: ch.is_phase_inverted(),
                    pan_law,
                }
            } else {
                continue;
            };

            if params.muted {
                continue;
            }

            // Apply EQ on a copy of the input before process_mix.
            let mut eq_samples: Vec<f32> = samples.to_vec();
            if let Some(eq) = self.eq_chains.get_mut(&ch_idx) {
                eq.process(&mut eq_samples);
            }

            let (ch_left, ch_right) =
                self.engine.engine_mut().process_mix(&[(id, params)], &eq_samples);

            for i in 0..bs {
                self.master_left[i] += ch_left[i];
                self.master_right[i] += ch_right[i];
            }
        }

        // Interleave master into stereo output.
        for i in 0..bs {
            self.stereo_out[i * 2] = self.master_left[i];
            self.stereo_out[i * 2 + 1] = self.master_right[i];
        }

        self.master_meter.process(&self.stereo_out);

        // Copy to JS-owned Float32Array (one alloc — unavoidable for the FFI).
        let out = js_sys::Float32Array::new_with_length((bs * 2) as u32);
        out.copy_from(&self.stereo_out);

        Ok(out)
    }

    // ------------------------------------------------------------------
    // Metering
    // ------------------------------------------------------------------

    pub fn master_peak_db_l(&self) -> f32 {
        self.master_meter.data().peak.first()
            .map(|p| p.current_db)
            .unwrap_or(-f32::INFINITY)
    }

    pub fn master_peak_db_r(&self) -> f32 {
        self.master_meter.data().peak.get(1)
            .map(|p| p.current_db)
            .unwrap_or(-f32::INFINITY)
    }

    pub fn master_rms_db_l(&self) -> f32 {
        self.master_meter.data().rms.first()
            .map(|r| r.current_db)
            .unwrap_or(-f32::INFINITY)
    }

    pub fn master_rms_db_r(&self) -> f32 {
        self.master_meter.data().rms.get(1)
            .map(|r| r.current_db)
            .unwrap_or(-f32::INFINITY)
    }

    pub fn master_clipping(&self) -> bool {
        self.master_meter.data().peak.iter().any(|p| p.clipped)
    }
}
