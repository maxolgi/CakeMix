use wasm_bindgen::prelude::*;

/// WASM binding for the oximedia-mixer audio engine.
///
/// Construction: `new(sample_rate, block_size, max_channels)`.
/// Set per-channel input via `set_channel_input(ch, data)`.
/// Process via `process(block_size)` → interleaved stereo Float32Array.
#[wasm_bindgen]
pub struct MixerWasm {
    engine: oximedia_mixer::AudioMixer,
    sample_rate: u32,
    block_size: u32,
    channel_inputs: std::collections::HashMap<u32, Vec<f32>>,
}

#[wasm_bindgen]
impl MixerWasm {
    #[wasm_bindgen(constructor)]
    pub fn new(
        sample_rate: u32,
        block_size: u32,
        max_channels: u32,
    ) -> Result<MixerWasm, JsValue> {
        console_error_panic_hook::set_once();

        let config = oximedia_mixer::MixerConfig {
            sample_rate,
            block_size: block_size as usize,
            max_channels: max_channels as usize,
            ..Default::default()
        };
        let engine = oximedia_mixer::AudioMixer::new(config);

        Ok(MixerWasm {
            engine,
            sample_rate,
            block_size,
            channel_inputs: std::collections::HashMap::new(),
        })
    }

    /// Set pending input audio for a channel (planar f32, mono or interleaved).
    pub fn set_channel_input(&mut self, ch: u32, data: &js_sys::Float32Array) {
        let mut buf = vec![0.0f32; data.length() as usize];
        data.copy_to(&mut buf);
        self.channel_inputs.insert(ch, buf);
    }

    /// Process one block. Returns interleaved stereo (L, R, L, R, ...).
    pub fn process(&mut self, block_size: u32) -> Result<js_sys::Float32Array, JsValue> {
        let bs = block_size as usize;
        let mut left = vec![0.0f32; bs];
        let mut right = vec![0.0f32; bs];

        // Sum all active channels into stereo master.
        for (&ch_id, samples) in &self.channel_inputs {
            let n = samples.len().min(bs);
            for i in 0..n {
                left[i] += samples[i];
                right[i] += samples[i];
            }
        }

        // Interleave.
        let mut out = vec![0.0f32; bs * 2];
        for i in 0..bs {
            out[i * 2] = left[i];
            out[i * 2 + 1] = right[i];
        }

        Ok(js_sys::Float32Array::from(&out[..]))
    }

    /// Set channel gain (linear 0.0–1.0+).
    pub fn set_channel_gain(&mut self, ch: u32, gain: f32) {
        if let Some(channel) = self.engine.channels_mut().get_mut(&ch) {
            channel.set_gain(gain);
        }
    }

    /// Set channel pan (-1.0 left, 0.0 center, 1.0 right).
    pub fn set_channel_pan(&mut self, ch: u32, pan: f32) {
        if let Some(channel) = self.engine.channels_mut().get_mut(&ch) {
            channel.set_pan(pan);
        }
    }

    /// Mute a channel.
    pub fn set_channel_mute(&mut self, ch: u32, muted: bool) {
        if let Some(channel) = self.engine.channels_mut().get_mut(&ch) {
            channel.set_muted(muted);
        }
    }
}
