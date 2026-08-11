/**
 * mixer-worklet-processor.js — CakeMix WASM mixer in an AudioWorklet.
 *
 * Uses dynamic import() to load the wasm-bindgen glue (works in addModule
 * classic scripts). Main thread sends the compiled WebAssembly.Module.
 */

const BLOCK_SIZE = 128;
const SAMPLE_RATE = 48000;
const FREQS = [220.0, 277.18, 329.63, 440.0];

let _wasmModule = null;
let _mixerReady = false;
let _initPromise = null;

// Dynamic import of the wasm-bindgen glue.
// This runs when the worklet script is first loaded.
_initPromise = import("/pkg/mixer_wasm.js").then(mod => {
    self._wasmMod = mod;
    self._wasmLoaded = true;
}).catch(err => {
    console.error("[mixer-worklet] dynamic import failed:", err);
});

class MixerProcessor extends AudioWorkletProcessor {
    constructor(options) {
        super();

        this._phase = new Float32Array(FREQS.length);
        this._running = false;
        this._gain = new Float32Array(FREQS.length).fill(0.5);
        this._pan = new Float32Array(FREQS.length).fill(0.0);
        this._muted = new Array(FREQS.length).fill(false);
        this._frameCount = 0;
        this._mixer = null;
        this._chBuf = new Float32Array(BLOCK_SIZE);

        const opts = (options && options.processorOptions) || {};

        this.port.onmessage = async (e) => {
            const msg = e.data;
            switch (msg.type) {
                case "init-wasm": {
                    // Wait for dynamic import to complete.
                    await _initPromise;
                    if (!self._wasmMod) {
                        this.port.postMessage({ type: "error", msg: "glue import failed" });
                        return;
                    }
                    try {
                        self._wasmMod.initSync(msg.module);
                        this._mixer = new self._wasmMod.MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 32);
                        this.port.postMessage({ type: "wasm-ready" });
                    } catch (err) {
                        this.port.postMessage({ type: "error", msg: String(err) });
                    }
                    break;
                }
                case "start":
                    this._running = true;
                    break;
                case "stop":
                    this._running = false;
                    break;
                case "set-gain":
                    if (msg.ch >= 0 && msg.ch < FREQS.length) {
                        this._gain[msg.ch] = msg.gain;
                        if (this._mixer) {
                            try { this._mixer.set_channel_gain(msg.ch, msg.gain); } catch(e) {}
                        }
                    }
                    break;
                case "set-pan":
                    if (msg.ch >= 0 && msg.ch < FREQS.length) {
                        this._pan[msg.ch] = msg.pan;
                        if (this._mixer) {
                            try { this._mixer.set_channel_pan(msg.ch, msg.pan); } catch(e) {}
                        }
                    }
                    break;
                case "set-mute":
                    if (msg.ch >= 0 && msg.ch < FREQS.length) {
                        this._muted[msg.ch] = msg.muted;
                        if (this._mixer) {
                            try { this._mixer.set_channel_mute(msg.ch, msg.muted); } catch(e) {}
                        }
                    }
                    break;
            }
        };

        this.port.postMessage({ type: "ready" });
    }

    process(inputs, outputs) {
        const out = outputs[0];
        if (!out || out.length < 2) return true;

        const outL = out[0];
        const outR = out[1];
        const n = Math.min(outL.length, BLOCK_SIZE);

        outL.fill(0);
        outR.fill(0);

        if (!this._running) return true;

        if (this._mixer) {
            // === WASM MIXER PATH ===
            // Generate test tones and feed to the WASM mixer.
            for (let ch = 0; ch < FREQS.length; ch++) {
                const freq = FREQS[ch];
                for (let i = 0; i < n; i++) {
                    this._chBuf[i] = 0.2 * Math.sin(2 * Math.PI * freq * this._phase[ch] / SAMPLE_RATE);
                    this._phase[ch] += 1;
                }
                try {
                    this._mixer.set_channel_input(ch, this._chBuf);
                } catch (e) {
                    console.error("[mixer-worklet] set_channel_input:", e);
                }
            }

            let output;
            try {
                output = this._mixer.process(BLOCK_SIZE);
            } catch (e) {
                console.error("[mixer-worklet] process:", e);
                return true;
            }

            // De-interleave stereo output to Web Audio format.
            for (let i = 0; i < n; i++) {
                outL[i] = output[i * 2];
                outR[i] = output[i * 2 + 1];
            }
        } else {
            // === FALLBACK: pure JS mixer (while WASM is loading) ===
            for (let ch = 0; ch < FREQS.length; ch++) {
                if (this._muted[ch]) continue;
                const gain = this._gain[ch];
                const pan = this._pan[ch];
                const freq = FREQS[ch];
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
            for (let i = 0; i < n; i++) {
                outL[i] = Math.tanh(outL[i]);
                outR[i] = Math.tanh(outR[i]);
            }
        }

        this._frameCount++;
        if (this._frameCount % 500 === 0) {
            let peakL = 0, peakR = 0;
            for (let i = 0; i < n; i++) {
                peakL = Math.max(peakL, Math.abs(outL[i]));
                peakR = Math.max(peakR, Math.abs(outR[i]));
            }
            this.port.postMessage({
                type: "status",
                frame: this._frameCount,
                peakL, peakR,
                wasmActive: !!this._mixer,
            });
        }

        return true;
    }
}

registerProcessor("mixer-processor", MixerProcessor);
