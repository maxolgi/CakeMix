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
        this._pubSource = "master"; // "master" | "bus" (bus stereo output via take_bus_output)
        this._pubBus = 0;      // bus index published when _pubSource === "bus"
        this._tapMissingWarned = false; // log-once guards for the channel tap
        this._tapErrorWarned = false;
        this._busTapMissingWarned = false; // log-once guards for the bus tap
        this._busTapErrorWarned = false;
        // Direct pcm paths (transferred MessagePorts; see _attachPcmPort
        // and the 'pub-port' message). sessionId → port so multiple
        // WebSRT receive sessions can carry pcm concurrently; no entry
        // = parent-channel relay fallback for that session.
        this._pcmPorts = new Map();
        this._pubOutPort = null;
        // Worklet-side (session, PID)→mixer-channel auto-mapper (the
        // store's old mapping policy, relocated so the direct port path
        // can map without a main-thread round trip). Keyed "sid:pid"
        // (string) so sessions may reuse TS PID numbers; entries carry
        // {sid, pid, chStart, channelCount}. The store's UI list
        // mirrors the "pid-mapped" events posted here.
        this._pidMap = {};
        this._pidAlloc = 0;
        // PIDs already sent their capped-out (chStart:-1) notice;
        // keyed "sid:pid", value = owning sid so a session teardown
        // can drop just its own entries.
        this._cappedPids = {};
        this._droppedPcm = 0;
        // Log-once guard: non-zero-session pcm against a wasm build
        // without the keyed pid API (see _mapPidSid).
        this._keyedMissingWarned = false;

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
                if (this._mixer) try {
                    var mapSid = msg.sessionId || 0;
                    this._mapPidSid(mapSid, msg.pid, msg.chStart, msg.channelCount);
                    // Explicit mapping registers without allocating (the
                    // auto-mapper skips it from then on).
                    this._pidMap[mapSid + ":" + msg.pid] = { sid: mapSid, pid: msg.pid, chStart: msg.chStart, channelCount: msg.channelCount };
                } catch(e) {}
            } else if (msg.type === "unmap-pid") {
                var unmapSid = msg.sessionId || 0;
                if (this._mixer) this._unmapPidSid(unmapSid, msg.pid);
                delete this._pidMap[unmapSid + ":" + msg.pid];
                if (Object.keys(this._pidMap).length === 0) this._pidAlloc = 0;
            } else if (msg.type === "pcm-port") {
                // Session-scoped direct pcm port (see _attachPcmPort):
                // a port with no sessionId defaults to session 0 (exact
                // legacy single-session behavior); a null port with no
                // sessionId is the legacy full reset; a null port WITH
                // a sessionId tears down just that session.
                this._attachPcmPort(msg.port || null, msg.port ? (msg.sessionId || 0) : msg.sessionId);
            } else if (msg.type === "pub-port") {
                // Publish output channel: completed pub batches flow
                // worklet → publish worker over this port instead of the
                // parent relay (publish.ts wires it per connect; null
                // closes it and reverts to the parent path).
                if (this._pubOutPort) { try { this._pubOutPort.close(); } catch(e) {} }
                this._pubOutPort = msg.port || null;
            } else if (msg.type === "pcm") {
                // Fallback parent-channel relay (direct port not wired, or
                // mid-handshake): same auto-map + feed path. Optional
                // sessionId (default 0 in _onPcm).
                this._onPcm(msg, msg.sessionId);
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
            } else if (msg.type === "set-main-assign") {
                if (this._mixer) try { this._mixer.set_channel_main_assign(msg.ch, msg.on); } catch(e) {}
            } else if (msg.type === "set-bus-feeds-main") {
                // Feature-detected: wasm builds without set_bus_feeds_main
                // keep the bus as an independent mix feeding master (default).
                if (this._mixer && typeof this._mixer.set_bus_feeds_main === "function")
                    try { this._mixer.set_bus_feeds_main(msg.bus, msg.on); } catch(e) {}
            } else if (msg.type === "set-bus-source") {
                if (this._mixer) try { this._mixer.set_bus_source(msg.bus, msg.slot, msg.ch); } catch(e) {}
            } else if (msg.type === "clear-bus-source") {
                if (this._mixer) try { this._mixer.clear_bus_source(msg.bus, msg.slot); } catch(e) {}
            } else if (msg.type === "set-bus-gain") {
                if (this._mixer) try { this._mixer.set_bus_gain(msg.bus, msg.gain); } catch(e) {}
            } else if (msg.type === "set-bus-mute") {
                if (this._mixer) try { this._mixer.set_bus_mute(msg.bus, msg.muted); } catch(e) {}
            } else if (msg.type === "scene-save") {
                // Feature-detected: wasm builds without the scene API stay
                // inert (no scene-saved event comes back, so no chip appears).
                // The new scene id is posted back like pid-mapped events.
                if (this._mixer && typeof this._mixer.save_scene === "function") {
                    try {
                        var sceneId = this._mixer.save_scene();
                        this.port.postMessage({ type: "scene-saved", id: sceneId });
                    } catch(e) {}
                }
            } else if (msg.type === "scene-recall") {
                // Timed cross-fade when the wasm has it, instant otherwise;
                // fadeMs defaults to 0 (instant).
                if (this._mixer && typeof this._mixer.recall_scene_fade === "function") {
                    try { this._mixer.recall_scene_fade(msg.id, msg.fadeMs || 0); } catch(e) {}
                } else if (this._mixer && typeof this._mixer.recall_scene === "function") {
                    try { this._mixer.recall_scene(msg.id); } catch(e) {}
                }
            } else if (msg.type === "scene-delete") {
                if (this._mixer && typeof this._mixer.delete_scene === "function") {
                    try { this._mixer.delete_scene(msg.id); } catch(e) {}
                }
            } else if (msg.type === "pub-start") {
                // Enable the publish tap. Source "master" (default): msg.channels
                // outputs — 2 taps the master stereo pair, 16/32/64/128 switch
                // to the mixer's per-channel direct-out tap (set via
                // set_channel_tap, feature-detected). Source "bus": msg.bus's
                // stereo output via take_bus_output (feature-detected), channels
                // forced to 2. Idempotent; does not affect running state. A
                // start while already started cleanly restarts the accumulator
                // + sample counter (partial batch dropped).
                var src = msg.source === "bus" ? "bus" : "master";
                var ch = msg.channels;
                if (src === "bus") {
                    ch = 2; // bus publish is stereo (channels must be/defaults 2)
                } else if (ch !== 2 && ch !== 16 && ch !== 32 && ch !== 64 && ch !== 128) {
                    if (ch !== undefined) console.warn("[pub] invalid channels " + ch + " — using 2");
                    ch = 2;
                }
                this._pubSource = src;
                this._pubBus = typeof msg.bus === "number" ? msg.bus : 0;
                this._pubChannels = ch;
                this._pubBuf = new Float32Array(PUB_BATCH_FRAMES * ch);
                this._pubFill = 0;
                this._pubFrames = 0;
                this._pubActive = true;
                this._setChannelTap(ch > 2 ? ch : 0);
            } else if (msg.type === "pub-stop") {
                // Disable publish tap, drop any partial batch, reset the
                // source to master. Idempotent.
                this._pubActive = false;
                this._pubFill = 0;
                this._pubFrames = 0;
                this._pubSource = "master";
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
                    droppedPcm: this._droppedPcm,
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
                var meter = {
                    type: "meter",
                    peakL: this._mixer.master_peak_db_l(),
                    peakR: this._mixer.master_peak_db_r(),
                    rmsL: this._mixer.master_rms_db_l(),
                    rmsR: this._mixer.master_rms_db_r(),
                    clip: this._mixer.master_clipping(),
                    limiterGr: this._mixer.limiter_gain_reduction_db(),
                    channels: JSON.parse(this._mixer.channel_meters_json()),
                    buses: JSON.parse(this._mixer.bus_meters_json()),
                    droppedPcm: this._droppedPcm,
                };
                // Elastic playout diagnostics (drift corrections applied by
                // the wasm FIFOs; nonzero slips/inserts are normal — they
                // reconcile source-clock vs audio-clock ppm drift. Growing
                // starved counts or maxed depth indicate delivery problems).
                if (typeof this._mixer.elastic_slips === "function") {
                    meter.elasticSlips = Number(this._mixer.elastic_slips());
                    meter.elasticInserts = Number(this._mixer.elastic_inserts());
                    meter.starvedBlocks = Number(this._mixer.starved_blocks());
                    meter.fifoMaxDepth = Number(this._mixer.fifo_max_depth());
                }
                this.port.postMessage(meter);
            } catch(e) {}
        }

        return true;
    }

    // Publish tap: append this block's audio to the accumulator — in "bus"
    // mode the selected bus's stereo output (take_bus_output, silence when
    // nothing new or the tap failed), in 2ch mode exactly what was written
    // to the audio output (master pair), in >2ch mode the channel direct-out
    // tap (silence when silent or the tap failed). When a full
    // PUB_BATCH_FRAMES batch is ready, post it transferred with the pts of
    // its first frame.
    _pubTap(outL, outR, n, silent) {
        if (this._pubSource === "bus") {
            // Bus stereo output, one take_bus_output block per process block
            // (already interleaved L/R — pubInterleaveNch is a defensive
            // copy: null/short contributes silence).
            var busTap = silent ? null : this._takeBusOutput(this._pubBus);
            this._pubFill = pubInterleaveNch(this._pubBuf, this._pubFill, busTap, n, 2);
        } else if (this._pubChannels === 2) {
            this._pubFill = pubInterleaveBlock(this._pubBuf, this._pubFill, outL, outR, n);
        } else {
            var tap = silent ? null : this._takeChannelTap();
            this._pubFill = pubInterleaveNch(this._pubBuf, this._pubFill, tap, n, this._pubChannels);
        }
        this._pubFrames += n;
        if (this._pubFill < this._pubBuf.length) return;
        // Transfer detaches the buffer, so the posted samples array is fresh.
        var batch = new Float32Array(this._pubBuf);
        // Direct port to the publish worker when wired; parent relay
        // (App.tsx → publish.ts relayPubPcm) as fallback.
        var dst = this._pubOutPort || this.port;
        dst.postMessage({
            type: "pub-pcm",
            samples: batch,
            ptsUs: pubBatchPtsUs(this._pubFrames - PUB_BATCH_FRAMES),
            channels: this._pubChannels,
        }, [batch.buffer]);
        this._pubFill = 0;
    }

    // Direct pcm channel (see the store's connectWebsrt): the store
    // creates a MessageChannel per connect, per session, transfers one
    // end to the WebSRT worker (its 'pcm-port' cmd) and this end here —
    // raw pcm then flows worker→worklet with zero main-thread hops.
    // Ports are scoped by sessionId: attaching replaces (closes) any
    // existing port for that session. port null WITH a sessionId closes
    // that session's port and forgets only its mappings — its strips go
    // silent as their FIFOs starve. The global _pidAlloc cursor NEVER
    // rewinds on a per-session disconnect: gaps in channel numbering are
    // acceptable (strips are scarce but reconnect cycles are rare, and a
    // rewind could hand a still-live session's channels to a newcomer).
    // Exceptions, both legacy rules: a null port WITHOUT a sessionId is
    // the full reset (close every port, wipe all mapping state, rewind
    // the allocator), and when _pidMap empties entirely the allocator
    // also rewinds — nothing is mapped either way, so fresh mappings
    // pack from channel 0 again.
    _attachPcmPort(port, sessionId) {
        if (sessionId === undefined) {
            // Full reset (legacy semantics for single-session stores).
            this._pcmPorts.forEach(function(p) { try { p.close(); } catch(e) {} });
            this._pcmPorts.clear();
            this._pidMap = {};
            this._cappedPids = {};
            this._pidAlloc = 0;
            return;
        }
        var old = this._pcmPorts.get(sessionId);
        if (old) { try { old.close(); } catch(e) {} this._pcmPorts.delete(sessionId); }
        if (!port) {
            this._forgetSessionMappings(sessionId);
            if (Object.keys(this._pidMap).length === 0) this._pidAlloc = 0;
            return;
        }
        this._pcmPorts.set(sessionId, port);
        var self = this;
        port.onmessage = function (e) {
            var d = e.data;
            if (d && d.type === "batch") {
                for (var i = 0; i < d.msgs.length; i++) self._onPcm(d.msgs[i], sessionId);
            }
        };
    }

    // Delete one session's entries from the pid maps (its port just
    // closed). The maps are tiny (≤ 128 channels / a handful of PIDs),
    // so a full key scan is fine.
    _forgetSessionMappings(sid) {
        var keys = Object.keys(this._pidMap);
        for (var i = 0; i < keys.length; i++) {
            if (this._pidMap[keys[i]].sid === sid) delete this._pidMap[keys[i]];
        }
        var capped = Object.keys(this._cappedPids);
        for (var j = 0; j < capped.length; j++) {
            if (this._cappedPids[capped[j]] === sid) delete this._cappedPids[capped[j]];
        }
    }

    // One pcm message from either arrival path (direct port or parent
    // relay), scoped to session sid (default 0 for the parent relay).
    // First sight of a (session, PID) auto-maps it: channels packed
    // from the global cursor, capped at 128 total (AGENTS.md "128 input
    // strips max") — the policy the store used to run main-thread.
    // "pid-mapped" events (now carrying the sessionId) mirror the
    // mapping to the store for the UI; drops (wasm not ready, or past
    // the cap) are counted and posted as "pcm-dropped" (one cumulative
    // global total — the worklet is the single counting authority).
    _onPcm(m, sid) {
        if (sid === undefined || sid === null) sid = 0;
        if (!m || m.type !== "pcm") return;
        if (!this._mixer) { this._droppedPcm++; this._postDropped(); return; }
        var pid = m.pid;
        var key = sid + ":" + pid;
        if (!(key in this._pidMap)) {
            var cc = m.channelCount || 1;
            if (this._pidAlloc + cc > 128) {
                if (!(key in this._cappedPids)) {
                    this._cappedPids[key] = sid;
                    this.port.postMessage({ type: "pid-mapped", pid: pid, sessionId: sid, chStart: -1, channelCount: cc });
                }
                this._droppedPcm++;
                this._postDropped();
                return;
            }
            var chStart = this._pidAlloc;
            this._pidAlloc += cc;
            this._pidMap[key] = { sid: sid, pid: pid, chStart: chStart, channelCount: cc };
            this._mapPidSid(sid, pid, chStart, cc);
            this.port.postMessage({ type: "pid-mapped", pid: pid, sessionId: sid, chStart: chStart, channelCount: cc });
        }
        this._feedPcmSid(sid, pid, m.samples);
    }

    // Session-scoped wasm pid-map access: key = (sid << 16) | pid lets
    // multiple WebSRT sessions reuse TS PID numbers in the mixer's map.
    // Feature-detected (typeof) like _setChannelTap: wasm builds
    // predating the keyed API only expose the legacy u16 calls, which
    // are exactly the keyed ones under session 0 — so the fallback is
    // valid for sid 0 only; a non-zero sid against such a build logs
    // once and drops (a legacy call would alias onto session 0's
    // mapping). Callers must have checked this._mixer.
    _mapPidSid(sid, pid, chStart, cc) {
        if (typeof this._mixer.map_pid_keyed === "function") {
            try { this._mixer.map_pid_keyed((sid << 16) | pid, chStart, cc); } catch(e) {}
        } else if (sid === 0) {
            try { this._mixer.map_pid(pid, chStart, cc); } catch(e) {}
        } else {
            this._warnKeyedMissing();
        }
    }

    _unmapPidSid(sid, pid) {
        if (typeof this._mixer.unmap_pid_keyed === "function") {
            try { this._mixer.unmap_pid_keyed((sid << 16) | pid); } catch(e) {}
        } else if (sid === 0) {
            try { this._mixer.unmap_pid(pid); } catch(e) {}
        }
        // No fallback for sid !== 0 (legacy unmap would remove session
        // 0's mapping) — nothing to do against an old build.
    }

    _feedPcmSid(sid, pid, samples) {
        if (typeof this._mixer.feed_pcm_keyed === "function") {
            try { this._mixer.feed_pcm_keyed((sid << 16) | pid, samples); } catch(e) {}
        } else if (sid === 0) {
            try { this._mixer.feed_pcm(pid, samples); } catch(e) {}
        } else {
            this._warnKeyedMissing();
        }
    }

    _warnKeyedMissing() {
        if (this._keyedMissingWarned) return;
        this._keyedMissingWarned = true;
        console.warn("[pcm] wasm mixer has no keyed pid API — pcm from non-zero sessions is dropped");
    }

    _postDropped() {
        this.port.postMessage({ type: "pcm-dropped", total: this._droppedPcm });
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

    // Drain one block of a bus's post-gain stereo output (interleaved L/R),
    // or null when unavailable (the caller accumulates silence). Empty when
    // nothing new per the wasm contract — drains per block. Feature-detected
    // (typeof) like the channel tap; failures log once only.
    _takeBusOutput(bus) {
        if (!this._mixer || typeof this._mixer.take_bus_output !== "function") {
            if (!this._busTapMissingWarned) {
                this._busTapMissingWarned = true;
                console.warn("[pub] wasm mixer has no take_bus_output — publishing silence");
            }
            return null;
        }
        try {
            return this._mixer.take_bus_output(bus);
        } catch (e) {
            if (!this._busTapErrorWarned) {
                this._busTapErrorWarned = true;
                console.warn("[pub] take_bus_output(" + bus + ") failed: " + e);
            }
            return null;
        }
    }
}

registerProcessor("mixer-processor", MixerProcessor);
