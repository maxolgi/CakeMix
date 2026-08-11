use std::collections::HashMap;
use wasm_bindgen::prelude::*;

use oximedia_mixer::{
    channel::{ChannelType, PanLaw},
    ChannelId, ChannelProcessParams, MixerConfig, PanLawType,
};
use oximedia_audio::ChannelLayout;

pub mod effects;

/// WASM binding for the oximedia-mixer audio engine.
///
/// Construction: `new(sample_rate, buffer_size, max_channels)`.
/// Set per-channel input via `set_channel_input(ch, data)`.
/// Process via `process(block_size)` → interleaved stereo Float32Array.
///
/// # Per-channel input architecture
///
/// The engine's `process()` feeds the SAME input to every channel.
/// We resolve this at the binding layer by calling `engine.process_mix()`
/// once per active channel with that channel's own input, then summing
/// the master outputs. This is correct when all channels route to master
/// (the M0 default). Bus-effect sharing across per-channel calls is
/// deferred to M5.
#[wasm_bindgen]
pub struct MixerWasm {
    engine: oximedia_mixer::AudioMixer,
    buffer_size: usize,
    /// Maps JS-visible channel index → engine ChannelId (UUID).
    channel_ids: Vec<Option<ChannelId>>,
    /// Pending per-channel input audio (planar f32, mono).
    channel_inputs: HashMap<u32, Vec<f32>>,
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

    /// Set pending input audio for a channel (planar f32, mono).
    pub fn set_channel_input(&mut self, ch: u32, data: &js_sys::Float32Array) -> Result<(), JsValue> {
        self.ensure_channel(ch)?;
        let mut buf = vec![0.0f32; data.length() as usize];
        data.copy_to(&mut buf);
        self.channel_inputs.insert(ch, buf);
        Ok(())
    }

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

    /// Process one block. Calls `engine.process_mix()` per active channel
    /// with that channel's own input, sums to master stereo.
    /// Returns interleaved stereo (L, R, L, R, ...) Float32Array.
    pub fn process(&mut self, _block_size: u32) -> Result<js_sys::Float32Array, JsValue> {
        let bs = self.buffer_size;
        let mut master_left = vec![0.0f32; bs];
        let mut master_right = vec![0.0f32; bs];

        // Process each active channel through the real engine DSP.
        for (&ch_idx, samples) in &self.channel_inputs {
            let Some(&Some(id)) = self.channel_ids.get(ch_idx as usize) else {
                continue;
            };

            // Build per-channel params from current channel state.
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

            // Call the real engine DSP with this channel's own input.
            let (ch_left, ch_right) =
                self.engine.engine_mut().process_mix(&[(id, params)], samples);

            for i in 0..bs {
                master_left[i] += ch_left[i];
                master_right[i] += ch_right[i];
            }
        }

        // Interleave stereo.
        let mut out = vec![0.0f32; bs * 2];
        for i in 0..bs {
            out[i * 2] = master_left[i];
            out[i * 2 + 1] = master_right[i];
        }

        Ok(js_sys::Float32Array::from(&out[..]))
    }
}
