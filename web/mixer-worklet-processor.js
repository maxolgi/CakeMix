/**
 * mixer-worklet-processor.js — AudioWorkletProcessor for CakeMix demo.
 *
 * Self-contained: no ES module imports (AudioWorklet has limited import support).
 * Generates 4 test tones and applies gain/pan/mute in pure JS.
 *
 * The WASM mixer integration will be added later via a bundler step that
 * inlines the wasm-bindgen glue into this file.
 *
 * Messages from main thread:
 *   { type: 'set-gain', ch, gain }   — per-channel gain (0.0–1.5)
 *   { type: 'set-pan', ch, pan }     — per-channel pan (-1.0–1.0)
 *   { type: 'set-mute', ch, muted }  — mute/unmute
 *   { type: 'start' }                — start generating audio
 *   { type: 'stop' }                 — stop
 *
 * Messages to main thread:
 *   { type: 'ready' }                — worklet initialized
 *   { type: 'status', frame, peakL, peakR }  — periodic status
 */

const BLOCK_SIZE = 128;
const SAMPLE_RATE = 48000;
const NUM_CHANNELS = 4;
const FREQS = [220.0, 277.18, 329.63, 440.0];

class MixerProcessor extends AudioWorkletProcessor {
    constructor(options) {
        super();

        this._phase = new Float32Array(NUM_CHANNELS);
        this._gain = new Float32Array(NUM_CHANNELS).fill(0.5);
        this._pan = new Float32Array(NUM_CHANNELS).fill(0.0);
        this._muted = new Array(NUM_CHANNELS).fill(false);
        this._running = false;
        this._frameCount = 0;

        this.port.onmessage = (e) => {
            const msg = e.data;
            switch (msg.type) {
                case 'start':
                    this._running = true;
                    break;
                case 'stop':
                    this._running = false;
                    break;
                case 'set-gain':
                    if (msg.ch >= 0 && msg.ch < NUM_CHANNELS) {
                        this._gain[msg.ch] = msg.gain;
                    }
                    break;
                case 'set-pan':
                    if (msg.ch >= 0 && msg.ch < NUM_CHANNELS) {
                        this._pan[msg.ch] = msg.pan;
                    }
                    break;
                case 'set-mute':
                    if (msg.ch >= 0 && msg.ch < NUM_CHANNELS) {
                        this._muted[msg.ch] = msg.muted;
                    }
                    break;
            }
        };

        this.port.postMessage({ type: 'ready' });
    }

    process(inputs, outputs) {
        const out = outputs[0];
        if (!out || out.length < 2) return true;

        const outL = out[0];
        const outR = out[1];
        const n = outL.length;

        // Clear output.
        outL.fill(0);
        outR.fill(0);

        if (!this._running) return true;

        for (let ch = 0; ch < NUM_CHANNELS; ch++) {
            if (this._muted[ch]) continue;

            const gain = this._gain[ch];
            const pan = this._pan[ch];
            const freq = FREQS[ch];

            // Linear pan law: pan_norm = (pan + 1) * 0.5
            const panNorm = (pan + 1.0) * 0.5;
            const leftGain = (1.0 - panNorm) * gain;
            const rightGain = panNorm * gain;

            for (let i = 0; i < n; i++) {
                const sample = 0.2 * Math.sin(2 * Math.PI * freq * this._phase[ch] / SAMPLE_RATE);
                outL[i] += sample * leftGain;
                outR[i] += sample * rightGain;
                this._phase[ch] += 1;
            }
        }

        // Soft clip to prevent harsh clipping.
        for (let i = 0; i < n; i++) {
            outL[i] = Math.tanh(outL[i]);
            outR[i] = Math.tanh(outR[i]);
        }

        this._frameCount++;
        if (this._frameCount % 500 === 0) {
            let peakL = 0, peakR = 0;
            for (let i = 0; i < n; i++) {
                peakL = Math.max(peakL, Math.abs(outL[i]));
                peakR = Math.max(peakR, Math.abs(outR[i]));
            }
            this.port.postMessage({
                type: 'status',
                frame: this._frameCount,
                peakL: peakL,
                peakR: peakR,
            });
        }

        return true;
    }
}

registerProcessor('mixer-processor', MixerProcessor);
