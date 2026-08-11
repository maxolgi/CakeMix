/**
 * mixer-worklet-processor.js — AudioWorkletProcessor that wraps the
 * CakeMix WASM mixer for real-time audio processing.
 *
 * Receives a pre-compiled WebAssembly.Module via processor options,
 * instantiates the mixer, and processes 128-sample blocks.
 *
 * In M1 demo mode, generates 4 sine wave test tones (one per channel)
 * inside the worklet, feeds them to the mixer, and outputs stereo.
 */

import { MixerWasm, initSync } from '/pkg/mixer_wasm.js';

const BLOCK_SIZE = 128;
const SAMPLE_RATE = 48000;

// Test tone frequencies (A major chord).
const FREQS = [220.0, 277.18, 329.63, 440.0];

let _wasmReady = false;

class MixerProcessor extends AudioWorkletProcessor {
    constructor(options) {
        super();

        const opts = (options && options.processorOptions) || {};

        // Initialize WASM with the pre-compiled module.
        if (opts.module && !_wasmReady) {
            try {
                initSync(opts.module);
                _wasmReady = true;
            } catch (e) {
                console.error('[mixer-worklet] WASM init failed:', e);
            }
        }

        this._mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 32);

        // Per-channel phase accumulators for test tones.
        this._phase = new Float32Array(FREQS.length);
        this._running = true;
        this._frameCount = 0;

        // Receive control messages from the main thread.
        this.port.onmessage = (e) => {
            const msg = e.data;
            switch (msg.type) {
                case 'set-gain':
                    try { this._mixer.set_channel_gain(msg.ch, msg.gain); }
                    catch (err) { /* channel not yet created */ }
                    break;
                case 'set-pan':
                    try { this._mixer.set_channel_pan(msg.ch, msg.pan); }
                    catch (err) { /* channel not yet created */ }
                    break;
                case 'set-mute':
                    try { this._mixer.set_channel_mute(msg.ch, msg.muted); }
                    catch (err) { /* channel not yet created */ }
                    break;
            }
        };

        this.port.postMessage({ type: 'ready' });
    }

    process(inputs, outputs) {
        if (!this._running) return false;

        // Generate test tones and feed to mixer.
        const buf = new Float32Array(BLOCK_SIZE);
        for (let ch = 0; ch < FREQS.length; ch++) {
            const freq = FREQS[ch];
            for (let i = 0; i < BLOCK_SIZE; i++) {
                buf[i] = 0.2 * Math.sin(2 * Math.PI * freq * this._phase[ch] / SAMPLE_RATE);
                this._phase[ch] += 1;
            }
            try {
                this._mixer.set_channel_input(ch, buf);
            } catch (e) {
                console.error('[mixer-worklet] set_channel_input:', e);
            }
        }

        // Process one block.
        let output;
        try {
            output = this._mixer.process(BLOCK_SIZE);
        } catch (e) {
            console.error('[mixer-worklet] process:', e);
            return true;
        }

        // Write interleaved stereo to de-interleaved Web Audio output.
        const out = outputs[0];
        if (out && out.length >= 2) {
            const outL = out[0];
            const outR = out[1];
            const n = Math.min(outL.length, BLOCK_SIZE);
            for (let i = 0; i < n; i++) {
                outL[i] = output[i * 2];
                outR[i] = output[i * 2 + 1];
            }
        }

        this._frameCount++;
        if (this._frameCount === 1 || this._frameCount === 500) {
            this.port.postMessage({
                type: 'status',
                frame: this._frameCount,
                sample: output[0],
            });
        }

        return true;
    }
}

registerProcessor('mixer-processor', MixerProcessor);
