// Crypto polyfill for AudioWorkletGlobalScope (no crypto API)
if (typeof globalThis.crypto === 'undefined') {
    globalThis.crypto = {
        getRandomValues: function(arr) {
            for (var i = 0; i < arr.length; i++) {
                arr[i] = Math.floor(Math.random() * 256);
            }
            return arr;
        }
    };
}

// AudioWorklet processor for CakeMix mixer
var BLOCK_SIZE = 128;
var SAMPLE_RATE = 48000;
var FREQS = [220.0, 277.18, 329.63, 440.0];

class MixerProcessor extends AudioWorkletProcessor {
    constructor(options) {
        super();
        this._phase = new Float32Array(FREQS.length);
        this._running = false;
        this._mixer = null;
        this._chBuf = new Float32Array(BLOCK_SIZE);

        this.port.onmessage = (e) => {
            var msg = e.data;
            if (msg.type === "init-wasm") {
                try {
                    var module = msg.wasmBytes ? new WebAssembly.Module(msg.wasmBytes) : msg.module;
                    initSync(module);
                    this._mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 32);
                    this.port.postMessage({ type: "wasm-ready" });
                } catch(err) {
                    this.port.postMessage({ type: "error", msg: String(err) });
                }
            } else if (msg.type === "start") {
                this._running = true;
            } else if (msg.type === "stop") {
                this._running = false;
            } else if (msg.type === "set-gain") {
                if (this._mixer) try { this._mixer.set_channel_gain(msg.ch, msg.gain); } catch(e) {}
            } else if (msg.type === "set-pan") {
                if (this._mixer) try { this._mixer.set_channel_pan(msg.ch, msg.pan); } catch(e) {}
            } else if (msg.type === "set-mute") {
                if (this._mixer) try { this._mixer.set_channel_mute(msg.ch, msg.muted); } catch(e) {}
            }
        };
        this.port.postMessage({ type: "ready" });
    }

    process(inputs, outputs) {
        var out = outputs[0];
        if (!out || out.length < 2) return true;
        var outL = out[0], outR = out[1], n = Math.min(outL.length, BLOCK_SIZE);
        outL.fill(0); outR.fill(0);
        if (!this._running) return true;

        if (this._mixer) {
            for (var ch = 0; ch < FREQS.length; ch++) {
                var freq = FREQS[ch];
                for (var i = 0; i < n; i++) {
                    this._chBuf[i] = 0.2 * Math.sin(2 * Math.PI * freq * this._phase[ch] / SAMPLE_RATE);
                    this._phase[ch] += 1;
                }
                try { this._mixer.set_channel_input(ch, this._chBuf); } catch(e) {}
            }
            var output;
            try { output = this._mixer.process(BLOCK_SIZE); } catch(e) { return true; }
            for (var i = 0; i < n; i++) {
                outL[i] = output[i*2];
                outR[i] = output[i*2+1];
            }
        } else {
            for (var ch = 0; ch < FREQS.length; ch++) {
                var freq = FREQS[ch];
                for (var i = 0; i < n; i++) {
                    var s = 0.1 * Math.sin(2 * Math.PI * freq * this._phase[ch] / SAMPLE_RATE);
                    outL[i] += s; outR[i] += s;
                    this._phase[ch] += 1;
                }
            }
        }
        return true;
    }
}

registerProcessor("mixer-processor", MixerProcessor);
