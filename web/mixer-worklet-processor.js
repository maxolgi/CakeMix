/**
 * mixer-worklet-processor.js — CakeMix WASM mixer in an AudioWorklet.
 *
 * Self-contained: wasm-bindgen glue is inlined below (no import() or
 * dynamic import() — both disallowed on WorkletGlobalScope).
 * Main thread sends the compiled WebAssembly.Module via MessagePort.
 * The worklet calls initSync(module) to instantiate the wasm instance.
 */

// ═══════════════════════════════════════════════════════════════════
// BEGIN wasm-bindgen glue (auto-stripped from mixer_wasm.js)
// ═══════════════════════════════════════════════════════════════════

/* @ts-self-types="./mixer_wasm.d.ts" */

/**
 * WASM binding for the oximedia-mixer audio engine.
 *
 * Construction: `new(sample_rate, block_size, max_channels)`.
 *
 * # PCM transport contract
 *
 * Audio arrives from the WebSRT demuxer as **Float32 interleaved** per PID
 * (i32→f32 conversion done in the demuxer, not JS). 48 kHz is fixed.
 * PTS comes from PES PTS (s302m, ffmpeg-populated).
 *
 * Two input modes:
 * - `set_channel_input(ch, data)` — mono planar Float32 (one channel).
 * - `set_channel_input_interleaved(ch_start, data, num_channels)` —
 *   interleaved stereo/multichannel, de-interleaved into consecutive
 *   mixer channels starting at `ch_start`.
 *
 * PID mapping:
 * - `map_pid(pid, ch_start)` — route a TS PID's audio to mixer channels.
 * - `unmap_pid(pid)` — remove mapping (idempotent, for mid-stream reconfig).
 *
 * Process via `process(block_size)` → interleaved stereo Float32Array.
 *
 * # Per-channel input architecture
 *
 * The engine's `process()` feeds the SAME input to every channel.
 * We resolve this at the binding layer by calling `engine.process_mix()`
 * once per active channel with that channel's own input, then summing
 * the master outputs.
 */
class MixerWasm {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        MixerWasmFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_mixerwasm_free(ptr, 0);
    }
    /**
     * Convenience: feed PCM data for a specific PID directly.
     * Looks up the PID mapping and calls set_channel_input_interleaved.
     * This matches the PcmPacket handoff from the WebSRT worker.
     * @param {number} pid
     * @param {Float32Array} data
     */
    feed_pcm(pid, data) {
        const ret = wasm.mixerwasm_feed_pcm(this.__wbg_ptr, pid, data);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Map a TS PID to starting channel index with metadata.
     *
     * Aligns with the PidMap handoff contract from audioplan.md:
     * each PID carries channelCount (1/2/6/8) and is subscribed by default.
     *
     * Idempotent: calling twice with the same PID updates the mapping.
     * Safe for mid-stream reconfiguration.
     * @param {number} pid
     * @param {number} ch_start
     * @param {number} channel_count
     */
    map_pid(pid, ch_start, channel_count) {
        const ret = wasm.mixerwasm_map_pid(this.__wbg_ptr, pid, ch_start, channel_count);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Check if master output is clipping (peak ≥ 0 dBFS).
     * @returns {boolean}
     */
    master_clipping() {
        const ret = wasm.mixerwasm_master_clipping(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * Get master peak level in dB for left channel.
     * @returns {number}
     */
    master_peak_db_l() {
        const ret = wasm.mixerwasm_master_peak_db_l(this.__wbg_ptr);
        return ret;
    }
    /**
     * Get master peak level in dB for right channel.
     * @returns {number}
     */
    master_peak_db_r() {
        const ret = wasm.mixerwasm_master_peak_db_r(this.__wbg_ptr);
        return ret;
    }
    /**
     * Get master RMS level in dB for left channel.
     * @returns {number}
     */
    master_rms_db_l() {
        const ret = wasm.mixerwasm_master_rms_db_l(this.__wbg_ptr);
        return ret;
    }
    /**
     * Get master RMS level in dB for right channel.
     * @returns {number}
     */
    master_rms_db_r() {
        const ret = wasm.mixerwasm_master_rms_db_r(this.__wbg_ptr);
        return ret;
    }
    /**
     * @param {number} sample_rate
     * @param {number} buffer_size
     * @param {number} max_channels
     */
    constructor(sample_rate, buffer_size, max_channels) {
        const ret = wasm.mixerwasm_new(sample_rate, buffer_size, max_channels);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        MixerWasmFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Get the starting channel index a PID is mapped to, or -1 if unmapped.
     * @param {number} pid
     * @returns {number}
     */
    pid_channel(pid) {
        const ret = wasm.mixerwasm_pid_channel(this.__wbg_ptr, pid);
        return ret;
    }
    /**
     * Get the channel count for a PID, or 0 if unmapped.
     * @param {number} pid
     * @returns {number}
     */
    pid_channel_count(pid) {
        const ret = wasm.mixerwasm_pid_channel_count(this.__wbg_ptr, pid);
        return ret >>> 0;
    }
    /**
     * Process one block. Returns interleaved stereo (L, R, L, R, ...).
     * @param {number} _block_size
     * @returns {Float32Array}
     */
    process(_block_size) {
        const ret = wasm.mixerwasm_process(this.__wbg_ptr, _block_size);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return takeFromExternrefTable0(ret[0]);
    }
    /**
     * Set channel gain (linear 0.0–2.0).
     * @param {number} ch
     * @param {number} gain
     */
    set_channel_gain(ch, gain) {
        const ret = wasm.mixerwasm_set_channel_gain(this.__wbg_ptr, ch, gain);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set pending input audio for a channel (planar f32, mono).
     * @param {number} ch
     * @param {Float32Array} data
     */
    set_channel_input(ch, data) {
        const ret = wasm.mixerwasm_set_channel_input(this.__wbg_ptr, ch, data);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set pending input audio from an interleaved Float32 buffer.
     *
     * WebSRT delivers PCM as interleaved Float32 per PID (s302m).
     * This de-interleaves into consecutive mixer channels starting at `ch_start`.
     *
     * For stereo: L,R,L,R,... → ch_start gets L stream, ch_start+1 gets R stream.
     * For mono: passes through as-is to ch_start.
     * @param {number} ch_start
     * @param {Float32Array} data
     * @param {number} num_channels
     */
    set_channel_input_interleaved(ch_start, data, num_channels) {
        const ret = wasm.mixerwasm_set_channel_input_interleaved(this.__wbg_ptr, ch_start, data, num_channels);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Mute a channel.
     * @param {number} ch
     * @param {boolean} muted
     */
    set_channel_mute(ch, muted) {
        const ret = wasm.mixerwasm_set_channel_mute(this.__wbg_ptr, ch, muted);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set channel pan (-1.0 left, 0.0 center, 1.0 right).
     * @param {number} ch
     * @param {number} pan
     */
    set_channel_pan(ch, pan) {
        const ret = wasm.mixerwasm_set_channel_pan(this.__wbg_ptr, ch, pan);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Subscribe to a PID (enable audio output). Default is subscribed.
     * @param {number} pid
     */
    subscribe_pid(pid) {
        wasm.mixerwasm_subscribe_pid(this.__wbg_ptr, pid);
    }
    /**
     * Remove a PID mapping. Idempotent — safe to call on an unmapped PID.
     * @param {number} pid
     */
    unmap_pid(pid) {
        wasm.mixerwasm_unmap_pid(this.__wbg_ptr, pid);
    }
    /**
     * Unsubscribe from a PID (mute its channels).
     * @param {number} pid
     */
    unsubscribe_pid(pid) {
        wasm.mixerwasm_unsubscribe_pid(this.__wbg_ptr, pid);
    }
}
if (Symbol.dispose) MixerWasm.prototype[Symbol.dispose] = MixerWasm.prototype.free;
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_throw_bb96b2010945f0bc: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_error_757e9472f8410341: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_getRandomValues_eb590f34c5dc8fa0: function() { return handleError(function (arg0, arg1) {
            globalThis.crypto.getRandomValues(getArrayU8FromWasm0(arg0, arg1));
        }, arguments); },
        __wbg_length_1009454859bb3e03: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_new_227d7c05414eb861: function() {
            const ret = new Error();
            return ret;
        },
        __wbg_new_from_slice_709ab7061ebcc5da: function(arg0, arg1) {
            const ret = new Float32Array(getArrayF32FromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_prototypesetcall_10722f4fde830f07: function(arg0, arg1, arg2) {
            Float32Array.prototype.set.call(getArrayF32FromWasm0(arg0, arg1), arg2);
        },
        __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
            const ret = arg1.stack;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./mixer_wasm_bg.js": import0,
    };
}

const MixerWasmFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_mixerwasm_free(ptr, 1));

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

function getArrayF32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getFloat32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

let cachedFloat32ArrayMemory0 = null;
function getFloat32ArrayMemory0() {
    if (cachedFloat32ArrayMemory0 === null || cachedFloat32ArrayMemory0.byteLength === 0) {
        cachedFloat32ArrayMemory0 = new Float32Array(wasm.memory.buffer);
    }
    return cachedFloat32ArrayMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        const idx = addToExternrefTable0(e);
        wasm.__wbindgen_exn_store(idx);
    }
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedFloat32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (!module.ok) {
            throw new Error(`failed to fetch Wasm: ${module.status} ${module.statusText} fetching '${module.url}'`);
        }

        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        throw new Error('init() not available in worklet; use initSync(module) instead');
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}




// ═══════════════════════════════════════════════════════════════════
// END wasm-bindgen glue
// ═══════════════════════════════════════════════════════════════════

const BLOCK_SIZE = 128;
const SAMPLE_RATE = 48000;
const FREQS = [220.0, 277.18, 329.63, 440.0];

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

        this.port.onmessage = (e) => {
            const msg = e.data;
            switch (msg.type) {
                case "init-wasm": {
                    try {
                        initSync(msg.module);
                        this._mixer = new MixerWasm(SAMPLE_RATE, BLOCK_SIZE, 32);
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
