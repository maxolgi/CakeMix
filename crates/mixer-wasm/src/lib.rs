use std::collections::HashMap;
use wasm_bindgen::prelude::*;

use oximedia_mixer::{
    channel::{ChannelType, PanLaw},
    ChannelId, ChannelProcessParams, MixerConfig, PanLawType,
};
use oximedia_audio::ChannelLayout;
use oximedia_mixer::metering::{Meter, MeterBallistics};

pub mod effects;

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
/// - `map_pid(pid, ch_start)` — route a TS PID's audio to mixer channels.
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
}

/// Per-PID mapping metadata (from MPEG-2 component descriptors).
#[derive(Clone, Copy, Debug)]
struct PidMapping {
    /// Starting channel index in the mixer.
    ch_start: u32,
    /// Number of audio channels in this PID (1, 2, 6, 8).
    channel_count: u32,
    /// Whether this PID is subscribed (audio is active).
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

        let config = MixerConfig {
            sample_rate,
            buffer_size: buffer_size as usize,
            max_channels: max_channels as usize,
            ..Default::default()
        };
        let engine = oximedia_mixer::AudioMixer::new(config);

        Ok(MixerWasm {
            engine,
            buffer_size: buffer_size as usize,
            channel_ids: vec![None; max_channels as usize],
            channel_inputs: HashMap::new(),
            pid_map: HashMap::new(),
            master_meter: Meter::new(2, sample_rate, MeterBallistics::Fast),
        })
    }

    /// Ensure a channel exists at the given index. Returns its engine ChannelId.
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
            // Use Linear pan law so center pan = 1:1 throughput.
            if let Ok(ch) = self.engine.get_channel_mut(id) {
                ch.set_pan_law(PanLaw::Linear);
            }
            self.channel_ids[i] = Some(id);
        }
        Ok(self.channel_ids[i].unwrap())
    }

    // ------------------------------------------------------------------
    // Input
    // ------------------------------------------------------------------

    /// Set pending input audio for a channel (planar f32, mono).
    pub fn set_channel_input(&mut self, ch: u32, data: &js_sys::Float32Array) -> Result<(), JsValue> {
        self.ensure_channel(ch)?;
        let mut buf = vec![0.0f32; data.length() as usize];
        data.copy_to(&mut buf);
        self.channel_inputs.insert(ch, buf);
        Ok(())
    }

    /// Set pending input audio from an interleaved Float32 buffer.
    ///
    /// WebSRT delivers PCM as interleaved Float32 per PID (s302m).
    /// This de-interleaves into consecutive mixer channels starting at `ch_start`.
    ///
    /// For stereo: L,R,L,R,... → ch_start gets L stream, ch_start+1 gets R stream.
    /// For mono: passes through as-is to ch_start.
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

        // De-interleave into per-channel buffers.
        let mut deinterleaved: Vec<Vec<f32>> = vec![Vec::with_capacity(frames); nc];
        let mut raw = vec![0.0f32; total];
        data.copy_to(&mut raw);

        for f in 0..frames {
            for c in 0..nc {
                deinterleaved[c].push(raw[f * nc + c]);
            }
        }

        // Assign to consecutive mixer channels.
        for (c, buf) in deinterleaved.into_iter().enumerate() {
            let ch = ch_start + c as u32;
            self.ensure_channel(ch)?;
            self.channel_inputs.insert(ch, buf);
        }

        Ok(())
    }

    // ------------------------------------------------------------------
    // PID mapping (idempotent — safe for mid-stream reconfiguration)
    // ------------------------------------------------------------------

    /// Map a TS PID to starting channel index with metadata.
    ///
    /// Aligns with the PidMap handoff contract from audioplan.md:
    /// each PID carries channelCount (1/2/6/8) and is subscribed by default.
    ///
    /// Idempotent: calling twice with the same PID updates the mapping.
    /// Safe for mid-stream reconfiguration.
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

    /// Remove a PID mapping. Idempotent — safe to call on an unmapped PID.
    pub fn unmap_pid(&mut self, pid: u16) {
        self.pid_map.remove(&pid);
    }

    /// Get the starting channel index a PID is mapped to, or -1 if unmapped.
    pub fn pid_channel(&self, pid: u16) -> i32 {
        self.pid_map
            .get(&pid)
            .map(|m| m.ch_start as i32)
            .unwrap_or(-1)
    }

    /// Get the channel count for a PID, or 0 if unmapped.
    pub fn pid_channel_count(&self, pid: u16) -> u32 {
        self.pid_map
            .get(&pid)
            .map(|m| m.channel_count)
            .unwrap_or(0)
    }

    /// Subscribe to a PID (enable audio output). Default is subscribed.
    pub fn subscribe_pid(&mut self, pid: u16) {
        if let Some(m) = self.pid_map.get_mut(&pid) {
            m.subscribed = true;
            // Unmute all channels for this PID.
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

    /// Unsubscribe from a PID (mute its channels).
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

    /// Convenience: feed PCM data for a specific PID directly.
    /// Looks up the PID mapping and calls set_channel_input_interleaved.
    /// This matches the PcmPacket handoff from the WebSRT worker.
    pub fn feed_pcm(
        &mut self,
        pid: u16,
        data: &js_sys::Float32Array,
    ) -> Result<(), JsValue> {
        let Some(mapping) = self.pid_map.get(&pid).copied() else {
            return Ok(()); // unmapped PID — ignore
        };
        if !mapping.subscribed {
            return Ok(()); // unsubscribed — drop
        }
        self.set_channel_input_interleaved(mapping.ch_start, data, mapping.channel_count)
    }

    // ------------------------------------------------------------------
    // Channel controls
    // ------------------------------------------------------------------

    /// Set channel gain (linear 0.0–2.0).
    pub fn set_channel_gain(&mut self, ch: u32, gain: f32) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        self.engine
            .set_channel_gain(id, gain)
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    /// Set channel pan (-1.0 left, 0.0 center, 1.0 right).
    pub fn set_channel_pan(&mut self, ch: u32, pan: f32) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        self.engine
            .set_channel_pan(id, pan)
            .map_err(|e| JsValue::from_str(&format!("{e:?}")))
    }

    /// Mute a channel.
    pub fn set_channel_mute(&mut self, ch: u32, muted: bool) -> Result<(), JsValue> {
        let id = self.ensure_channel(ch)?;
        if let Ok(channel) = self.engine.get_channel_mut(id) {
            channel.set_muted(muted);
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Processing
    // ------------------------------------------------------------------

    /// Process one block. Returns interleaved stereo (L, R, L, R, ...).
    pub fn process(&mut self, _block_size: u32) -> Result<js_sys::Float32Array, JsValue> {
        let bs = self.buffer_size;
        let mut master_left = vec![0.0f32; bs];
        let mut master_right = vec![0.0f32; bs];

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

            let (ch_left, ch_right) =
                self.engine.engine_mut().process_mix(&[(id, params)], samples);

            for i in 0..bs {
                master_left[i] += ch_left[i];
                master_right[i] += ch_right[i];
            }
        }

        let mut out = vec![0.0f32; bs * 2];
        for i in 0..bs {
            out[i * 2] = master_left[i];
            out[i * 2 + 1] = master_right[i];
        }

        // Update master meter.
        self.master_meter.process(&out);

        Ok(js_sys::Float32Array::from(&out[..]))
    }

    // ------------------------------------------------------------------
    // Metering
    // ------------------------------------------------------------------

    /// Get master peak level in dB for left channel.
    pub fn master_peak_db_l(&self) -> f32 {
        self.master_meter.data().peak.first()
            .map(|p| p.current_db)
            .unwrap_or(-f32::INFINITY)
    }

    /// Get master peak level in dB for right channel.
    pub fn master_peak_db_r(&self) -> f32 {
        self.master_meter.data().peak.get(1)
            .map(|p| p.current_db)
            .unwrap_or(-f32::INFINITY)
    }

    /// Get master RMS level in dB for left channel.
    pub fn master_rms_db_l(&self) -> f32 {
        self.master_meter.data().rms.first()
            .map(|r| r.current_db)
            .unwrap_or(-f32::INFINITY)
    }

    /// Get master RMS level in dB for right channel.
    pub fn master_rms_db_r(&self) -> f32 {
        self.master_meter.data().rms.get(1)
            .map(|r| r.current_db)
            .unwrap_or(-f32::INFINITY)
    }

    /// Check if master output is clipping (peak ≥ 0 dBFS).
    pub fn master_clipping(&self) -> bool {
        self.master_meter.data().peak.iter().any(|p| p.clipped)
    }
}
