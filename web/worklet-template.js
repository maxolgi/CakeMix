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
var FREQS = [220.0, 277.18, 329.63, 440.0]; // A major chord for demo

class MixerProcessor extends AudioWorkletProcessor {
    constructor(options) {
        super();
        this._phase = new Float32Array(FREQS.length);
        this._running = false;
        this._mixer = null;
        this._chBuf = new Float32Array(BLOCK_SIZE);
        this._mode = "demo"; // "demo" or "live"
        this._pendingPcm = []; // queued PCM packets for live mode
        this._meterInterval = 0;

        this.port.onmessage = (e) => {
            var msg = e.data;
            if (msg.type === "init-wasm") {
                try {
                    var module = msg.wasmBytes ? new WebAssembly.Module(msg.wasmBytes) : msg.module;
                    initSync({ module: module });
                    this._mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 128);
                    this.port.postMessage({ type: "wasm-ready" });
                } catch(err) {
                    this.port.postMessage({ type: "error", msg: String(err) });
                }
            } else if (msg.type === "start") {
                this._running = true;
            } else if (msg.type === "stop") {
                this._running = false;
            } else if (msg.type === "set-mode") {
                this._mode = msg.mode; // "demo" or "live"
            } else if (msg.type === "set-gain") {
                if (this._mixer) try { this._mixer.set_channel_gain(msg.ch, msg.gain); } catch(e) {}
            } else if (msg.type === "set-pan") {
                if (this._mixer) try { this._mixer.set_channel_pan(msg.ch, msg.pan); } catch(e) {}
            } else if (msg.type === "set-mute") {
                if (this._mixer) try { this._mixer.set_channel_mute(msg.ch, msg.muted); } catch(e) {}
            } else if (msg.type === "set-solo") {
                if (this._mixer) try { this._mixer.set_channel_solo(msg.ch, msg.soloed); } catch(e) {}
            } else if (msg.type === "set-eq-gain") {
                if (this._mixer) try { this._mixer.set_eq_band_gain(msg.ch, msg.band, msg.gainDb); } catch(e) {}
            } else if (msg.type === "set-eq-freq") {
                if (this._mixer) try { this._mixer.set_eq_band_freq(msg.ch, msg.band, msg.freqHz); } catch(e) {}
            } else if (msg.type === "set-eq-q") {
                if (this._mixer) try { this._mixer.set_eq_band_q(msg.ch, msg.band, msg.q); } catch(e) {}
            } else if (msg.type === "set-eq-bypass") {
                if (this._mixer) try { this._mixer.set_eq_bypass(msg.ch, msg.bypassed); } catch(e) {}
            } else if (msg.type === "map-pid") {
                if (this._mixer) try { this._mixer.map_pid(msg.pid, msg.chStart, msg.channelCount); } catch(e) {}
            } else if (msg.type === "unmap-pid") {
                if (this._mixer) try { this._mixer.unmap_pid(msg.pid); } catch(e) {}
            } else if (msg.type === "set-input-gain") {
                if (this._mixer) try { this._mixer.set_channel_input_gain(msg.ch, msg.gainDb); } catch(e) {}
            } else if (msg.type === "set-phase") {
                if (this._mixer) try { this._mixer.set_channel_phase(msg.ch, msg.inverted); } catch(e) {}
            } else if (msg.type === "set-pan-law") {
                if (this._mixer) try { this._mixer.set_channel_pan_law(msg.ch, msg.law); } catch(e) {}
            } else if (msg.type === "set-name") {
                if (this._mixer) try { this._mixer.set_channel_name(msg.ch, msg.name); } catch(e) {}
            } else if (msg.type === "enable-compressor") {
                if (this._mixer) try { this._mixer.enable_compressor(msg.ch); } catch(e) {}
            } else if (msg.type === "disable-compressor") {
                if (this._mixer) try { this._mixer.disable_compressor(msg.ch); } catch(e) {}
            } else if (msg.type === "set-comp-param") {
                if (this._mixer) try { this._mixer.set_comp_param(msg.ch, msg.param, msg.value); } catch(e) {}
            } else if (msg.type === "enable-gate") {
                if (this._mixer) try { this._mixer.enable_gate(msg.ch); } catch(e) {}
            } else if (msg.type === "disable-gate") {
                if (this._mixer) try { this._mixer.disable_gate(msg.ch); } catch(e) {}
            } else if (msg.type === "set-gate-param") {
                if (this._mixer) try { this._mixer.set_gate_param(msg.ch, msg.param, msg.value); } catch(e) {}
            } else if (msg.type === "enable-expander") {
                if (this._mixer) try { this._mixer.enable_expander(msg.ch); } catch(e) {}
            } else if (msg.type === "disable-expander") {
                if (this._mixer) try { this._mixer.disable_expander(msg.ch); } catch(e) {}
            } else if (msg.type === "set-exp-param") {
                if (this._mixer) try { this._mixer.set_expander_param(msg.ch, msg.param, msg.value); } catch(e) {}
            } else if (msg.type === "set-master-gain") {
                if (this._mixer) try { this._mixer.set_master_gain(msg.gain); } catch(e) {}
            } else if (msg.type === "set-limiter") {
                if (this._mixer) try { this._mixer.set_limiter_enabled(msg.enabled); } catch(e) {}
            } else if (msg.type === "set-limiter-ceiling") {
                if (this._mixer) try { this._mixer.set_limiter_ceiling(msg.ceilingDb); } catch(e) {}
            } else if (msg.type === "set-limiter-release") {
                if (this._mixer) try { this._mixer.set_limiter_release(msg.releaseMs); } catch(e) {}
            } else if (msg.type === "add-bus") {
                if (this._mixer) try { var busId = this._mixer.add_bus(msg.name, msg.busType); this.port.postMessage({type:"bus-added", busId: busId}); } catch(e) {}
            } else if (msg.type === "route-to-bus") {
                if (this._mixer) try { this._mixer.route_channel_to_bus(msg.ch, msg.busId); } catch(e) {}
            } else if (msg.type === "route-to-master") {
                if (this._mixer) try { this._mixer.route_channel_to_master(msg.ch); } catch(e) {}
            } else if (msg.type === "set-aux-send") {
                if (this._mixer) try { this._mixer.set_aux_send(msg.ch, msg.sendIdx, msg.busId, msg.level, msg.preFader); } catch(e) {}
            } else if (msg.type === "remove-aux-send") {
                if (this._mixer) try { this._mixer.remove_aux_send(msg.ch, msg.sendIdx); } catch(e) {}
            } else if (msg.type === "pcm") {
                // External PCM from WebSRT worker (relayed via main thread).
                // msg.samples is a Float32Array, msg.pid identifies the stream.
                if (this._mixer && this._mode === "live") {
                    try { this._mixer.feed_pcm(msg.pid, msg.samples); } catch(e) {}
                }
            }
        };
        this.port.postMessage({ type: "ready" });
    }

    process(inputs, outputs) {
        var out = outputs[0];
        if (!out || out.length < 2) return true;
        var outL = out[0], outR = out[1], n = Math.min(outL.length, BLOCK_SIZE);
        outL.fill(0); outR.fill(0);
        if (!this._running || !this._mixer) {
            // Send zeroed meters when stopped so UI clears.
            this._meterInterval++;
            if (this._meterInterval >= 10) {
                this._meterInterval = 0;
                this.port.postMessage({
                    type: "meter",
                    peakL: -Infinity, peakR: -Infinity,
                    rmsL: -Infinity, rmsR: -Infinity,
                    clip: false,
                });
            }
            return true;
        }

        if (this._mode === "demo") {
            // Generate test tones, feed to mixer
            for (var ch = 0; ch < FREQS.length; ch++) {
                var freq = FREQS[ch];
                for (var i = 0; i < n; i++) {
                    this._chBuf[i] = 0.2 * Math.sin(2 * Math.PI * freq * this._phase[ch] / SAMPLE_RATE);
                    this._phase[ch] += 1;
                }
                try { this._mixer.set_channel_input(ch, this._chBuf); } catch(e) {}
            }
        }
        // In "live" mode, PCM is already fed via feed_pcm messages —
        // just call process().

        var output;
        try { output = this._mixer.process(BLOCK_SIZE); } catch(e) { return true; }
        for (var i = 0; i < n; i++) {
            outL[i] = output[i*2];
            outR[i] = output[i*2+1];
        }

        // Report meters every ~10 blocks (every ~2ms at 128/48k)
        this._meterInterval++;
        if (this._meterInterval >= 10) {
            this._meterInterval = 0;
            try {
                this.port.postMessage({
                    type: "meter",
                    peakL: this._mixer.master_peak_db_l(),
                    peakR: this._mixer.master_peak_db_r(),
                    rmsL: this._mixer.master_rms_db_l(),
                    rmsR: this._mixer.master_rms_db_r(),
                    clip: this._mixer.master_clipping(),
                    limiterGr: this._mixer.limiter_gain_reduction_db(),
                    channels: JSON.parse(this._mixer.channel_meters_json()),
                });
            } catch(e) {}
        }

        return true;
    }
}

registerProcessor("mixer-processor", MixerProcessor);
