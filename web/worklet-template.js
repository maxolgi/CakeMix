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
var TONE_FREQS = [220.0, 277.18, 329.63, 440.0]; // A major chord for testing
var PUB_BATCH_FRAMES = 1024; // publish tap batch size: 8 blocks of 128 frames

// Publish-tap batching helpers — pure (no `this`) so the math is inspectable.

// Interleave one block (n frames) of stereo into acc starting at
// acc[fillFloats]. Returns the new fill (in floats); acc must have room.
function pubInterleaveBlock(acc, fillFloats, blockL, blockR, n) {
    for (var i = 0; i < n; i++) {
        acc[fillFloats + i * 2] = blockL[i];
        acc[fillFloats + i * 2 + 1] = blockR[i];
    }
    return fillFloats + n * 2;
}

// Interleave one block (n frames) of N-channel tap data into acc starting
// at acc[fillFloats]. tap is one mixer take_channel_tap() block — already
// frame-major interleaved [ch0[i], ch1[i], …, chN-1[i]] — so this is a
// defensive copy: null (tap unavailable/failed) or short taps contribute
// silence for the missing samples. Returns the new fill (in floats); acc
// must have room for n*channels floats.
function pubInterleaveNch(acc, fillFloats, tap, n, channels) {
    for (var i = 0; i < n; i++) {
        var dst = fillFloats + i * channels;
        var src = i * channels;
        for (var c = 0; c < channels; c++) {
            acc[dst + c] = tap && src + c < tap.length ? tap[src + c] : 0;
        }
    }
    return fillFloats + n * channels;
}

// pts (µs) of the first frame of a batch, given the count of frames from
// pub-start up to that first frame (48 kHz). Batch k pts = k * 1024/48000*1e6
// ≈ k*21333.33 µs (rounded to whole µs; 1 µs ≪ 1 sample = 20.83 µs).
function pubBatchPtsUs(framesBeforeBatch) {
    return Math.round(framesBeforeBatch / SAMPLE_RATE * 1e6);
}

class MixerProcessor extends AudioWorkletProcessor {
    constructor(options) {
        super();
        this._tones = new Float32Array(TONE_FREQS.length);
        this._tonesOn = false;
        this._running = false;
        this._mixer = null;
        this._toneBuf = new Float32Array(BLOCK_SIZE);
        this._meterInterval = 0;
        // Publish tap state (output PCM relay; see _pubTap).
        this._pubActive = false;
        this._pubBuf = null;   // interleaved batch accumulator, alloc'd on pub-start
        this._pubFill = 0;     // floats currently in the accumulator
        this._pubFrames = 0;   // frames appended since pub-start (drives pts)
        this._pubChannels = 2; // tap width: 2 = master pair, >2 = channel direct outs
        this._tapMissingWarned = false; // log-once guards for the channel tap
        this._tapErrorWarned = false;

        this.port.onmessage = (e) => {
            var msg = e.data;
            if (msg.type === "init-wasm") {
                try {
                    var module = msg.wasmBytes ? new WebAssembly.Module(msg.wasmBytes) : msg.module;
                    initSync({ module: module });
                    this._mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 256);
                    // Defensive: a >2ch pub may have started before wasm was
                    // ready — (re)arm the channel tap on the fresh mixer.
                    if (this._pubActive && this._pubChannels > 2) this._setChannelTap(this._pubChannels);
                    this.port.postMessage({ type: "wasm-ready" });
                } catch(err) {
                    this.port.postMessage({ type: "error", msg: String(err) });
                }
            } else if (msg.type === "start") {
                this._running = true;
            } else if (msg.type === "stop") {
                this._running = false;
            } else if (msg.type === "tones") {
                // Test tones feed mixer inputs 0..3 whenever the engine runs.
                this._tonesOn = !!msg.on;
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
            } else if (msg.type === "set-aux-send") {
                if (this._mixer) try { this._mixer.set_aux_send(msg.ch, msg.sendIdx, msg.busId, msg.level, msg.preFader); } catch(e) {}
            } else if (msg.type === "remove-aux-send") {
                if (this._mixer) try { this._mixer.remove_aux_send(msg.ch, msg.sendIdx); } catch(e) {}
            } else if (msg.type === "set-bus-source") {
                if (this._mixer) try { this._mixer.set_bus_source(msg.bus, msg.slot, msg.ch); } catch(e) {}
            } else if (msg.type === "clear-bus-source") {
                if (this._mixer) try { this._mixer.clear_bus_source(msg.bus, msg.slot); } catch(e) {}
            } else if (msg.type === "set-bus-gain") {
                if (this._mixer) try { this._mixer.set_bus_gain(msg.bus, msg.gain); } catch(e) {}
            } else if (msg.type === "set-bus-mute") {
                if (this._mixer) try { this._mixer.set_bus_mute(msg.bus, msg.muted); } catch(e) {}
            } else if (msg.type === "pcm") {
                // External PCM from WebSRT worker (relayed via main thread).
                // msg.samples is a Float32Array, msg.pid identifies the stream.
                if (this._mixer) {
                    try { this._mixer.feed_pcm(msg.pid, msg.samples); } catch(e) {}
                }
            } else if (msg.type === "pub-start") {
                // Enable the publish tap for msg.channels outputs (default 2).
                // 2 taps the master stereo pair exactly as before; 16/32/64/128
                // switch to the mixer's per-channel direct-out tap (set via
                // set_channel_tap, feature-detected). Idempotent; does not
                // affect running state. A start while already started cleanly
                // restarts the accumulator + sample counter (partial batch
                // dropped).
                var ch = msg.channels;
                if (ch !== 2 && ch !== 16 && ch !== 32 && ch !== 64 && ch !== 128) {
                    if (ch !== undefined) console.warn("[pub] invalid channels " + ch + " — using 2");
                    ch = 2;
                }
                this._pubChannels = ch;
                this._pubBuf = new Float32Array(PUB_BATCH_FRAMES * ch);
                this._pubFill = 0;
                this._pubFrames = 0;
                this._pubActive = true;
                this._setChannelTap(ch > 2 ? ch : 0);
            } else if (msg.type === "pub-stop") {
                // Disable publish tap, drop any partial batch. Idempotent.
                this._pubActive = false;
                this._pubFill = 0;
                this._pubFrames = 0;
                this._setChannelTap(0);
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
            // Publish tap keeps posting silent batches while stopped/missing
            // so the downstream publisher's stream stays continuous (all
            // _pubChannels of them zero).
            if (this._pubActive) this._pubTap(outL, outR, n, true); // outL/outR are zeros
            // Send zeroed meters when stopped so UI clears.
            this._meterInterval++;
            if (this._meterInterval >= 10) {
                this._meterInterval = 0;
                this.port.postMessage({
                    type: "meter",
                    peakL: -Infinity, peakR: -Infinity,
                    rmsL: -Infinity, rmsR: -Infinity,
                    clip: false,
                    limiterGr: 0,
                    channels: [],
                    buses: [],
                });
            }
            return true;
        }

        if (this._tonesOn) {
            // Test tones: sine generators fed to mixer inputs 0..3.
            for (var ch = 0; ch < TONE_FREQS.length; ch++) {
                var freq = TONE_FREQS[ch];
                for (var i = 0; i < n; i++) {
                    this._toneBuf[i] = 0.2 * Math.sin(2 * Math.PI * freq * this._tones[ch] / SAMPLE_RATE);
                    this._tones[ch] += 1;
                }
                try { this._mixer.set_channel_input(ch, this._toneBuf); } catch(e) {}
            }
        }
        // Live PCM arrives via feed_pcm messages — just call process().

        var output;
        try { output = this._mixer.process(BLOCK_SIZE); }
        catch(e) { if (this._pubActive) this._pubTap(outL, outR, n, true); return true; }
        for (var i = 0; i < n; i++) {
            outL[i] = output[i*2];
            outR[i] = output[i*2+1];
        }

        // Publish tap: post batched output PCM. 2ch publishes the master
        // pair above; >2ch publishes the channel direct-out tap instead.
        if (this._pubActive) this._pubTap(outL, outR, n, false);

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
                    buses: JSON.parse(this._mixer.bus_meters_json()),
                });
            } catch(e) {}
        }

        return true;
    }

    // Publish tap: append this block's audio to the accumulator — in 2ch
    // mode exactly what was written to the audio output (master pair), in
    // >2ch mode the channel direct-out tap (silence when silent or the tap
    // failed). When a full PUB_BATCH_FRAMES batch is ready, post it
    // transferred with the pts of its first frame.
    _pubTap(outL, outR, n, silent) {
        if (this._pubChannels === 2) {
            this._pubFill = pubInterleaveBlock(this._pubBuf, this._pubFill, outL, outR, n);
        } else {
            var tap = silent ? null : this._takeChannelTap();
            this._pubFill = pubInterleaveNch(this._pubBuf, this._pubFill, tap, n, this._pubChannels);
        }
        this._pubFrames += n;
        if (this._pubFill < this._pubBuf.length) return;
        // Transfer detaches the buffer, so the posted samples array is fresh.
        var batch = new Float32Array(this._pubBuf);
        this.port.postMessage({
            type: "pub-pcm",
            samples: batch,
            ptsUs: pubBatchPtsUs(this._pubFrames - PUB_BATCH_FRAMES),
            channels: this._pubChannels,
        }, [batch.buffer]);
        this._pubFill = 0;
    }

    // Configure the mixer's per-channel direct-out tap (n = channel count,
    // 0 = off). Feature-detected (typeof) so wasm builds without
    // set_channel_tap keep the 2ch master mode working; >2ch without the
    // tap logs once and publishes silence instead. Errors log once only.
    _setChannelTap(n) {
        if (!this._mixer || typeof this._mixer.set_channel_tap !== "function") {
            if (n > 0 && !this._tapMissingWarned) {
                this._tapMissingWarned = true;
                console.warn("[pub] wasm mixer has no set_channel_tap — publishing silence");
            }
            return;
        }
        try { this._mixer.set_channel_tap(n); }
        catch (e) {
            if (!this._tapErrorWarned) {
                this._tapErrorWarned = true;
                console.warn("[pub] set_channel_tap(" + n + ") failed: " + e);
            }
        }
    }

    // Drain one block from the mixer's channel direct-out tap, or null when
    // unavailable (the caller accumulates silence). Empty when the tap is
    // disabled per the wasm contract. Failures log once only.
    _takeChannelTap() {
        if (!this._mixer || typeof this._mixer.take_channel_tap !== "function") {
            if (!this._tapMissingWarned) {
                this._tapMissingWarned = true;
                console.warn("[pub] wasm mixer has no take_channel_tap — publishing silence");
            }
            return null;
        }
        try {
            return this._mixer.take_channel_tap();
        } catch (e) {
            if (!this._tapErrorWarned) {
                this._tapErrorWarned = true;
                console.warn("[pub] take_channel_tap failed: " + e);
            }
            return null;
        }
    }
}

registerProcessor("mixer-processor", MixerProcessor);
