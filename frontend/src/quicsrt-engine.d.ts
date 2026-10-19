// Ambient declarations for the vendored QuicSRT-wasm plain-JS engine
// wrappers (vendor/QuicSRT-wasm/js/*.js) so typechecking of the files
// importing them works (client.ts pulls in the vendored page/vendor/
// worker.ts chain; publish-worker.ts imports the wrappers directly).
// The wildcard patterns match by suffix, e.g.
// "…/vendor/QuicSRT-wasm/js/srt_engine.js". Surfaces mirror the wrappers'
// documented contracts (see each wrapper's header comment); the emcc
// module behind them is typed as unknown.

/** One engine action (plain JS object from js/srt_engine.js makeAction).
 *  kind: 0 SendDatagram, 1 DeliverMessage, 2 HandshakeComplete,
 *  3 WaitForData, 4 Close, 5 Log. free() is a no-op (GC owns them);
 *  takeData() hands over the payload Uint8Array (nulls it — the kinds
 *  consumers feed to takeData() always carry one). */
declare module "*js/srt_engine.js" {
  export default function init(): Promise<unknown>;

  export interface SrtAction {
    kind: number;
    data: Uint8Array | null;
    text: string;
    wait_ms: number;
    schedUs: number;
    takeData(): Uint8Array;
    free(): void;
  }

  /** The 17 Number fields the worker's serializeStats/emitLossEvents
   *  read; all zeros before the connection is live. */
  export interface SrtStats {
    elapsedMs: number;
    rttMs: number;
    bandwidthBps: number;
    rxData: number;
    rxBytes: number;
    rxLoss: number;
    rxRetransmit: number;
    rxDropped: number;
    rxBelated: number;
    rxBuffered: number;
    rxAck: number;
    rxNak: number;
    txData: number;
    txBytes: number;
    txRetransmit: number;
    txLoss: number;
    txBuffered: number;
  }

  export class SrtReceiver {
    constructor(latencyMs?: number, rttMs?: number);
    static newWithLatency(latencyMs: number): SrtReceiver;
    static newWithLatencyAndRtt(latencyMs: number, initialRttMs: number): SrtReceiver;
    /** now_us is accepted but ignored (the C engine keeps its own clocks);
     *  it must still be passed (the wrapper throws on undefined). */
    handle_datagram(bytes: Uint8Array, now_us: number): SrtAction[];
    poll(now_us: number): SrtAction[];
    /** C ABI return: >0 bytes accepted (possibly partial — resume feeding
     *  from bytes.subarray(rc)), 0 empty buffer, -1 send buffer full,
     *  -2 closed, -3 not handshaked, -4 invalid arg, -5 error. */
    sendMessage(bytes: Uint8Array, timeoutMs?: number): number;
    isHandshakeComplete(): boolean;
    isClosed(): boolean;
    getStats(): SrtStats;
    free(): void;
  }
}

/** SMPTE 302M / PES TS muxer behind the same emcc module
 *  (js/muxer_engine.js). poll() returns a fresh JS-owned copy. */
declare module "*js/muxer_engine.js" {
  export default function init(): Promise<unknown>;

  export class TsMuxer {
    constructor();
    /** 'av1' | 'hevc' | else → H.264 */
    setVideoCodec(codec: string): void;
    setVideoEnabled(enabled: boolean): void;
    setAudioCodec(codec: string, channel_count: number): void;
    addAudioPid(pid: number, codec: string, channel_count: number): void;
    setSparseEnabled(enabled: boolean): void;
    setSparseThreshold(ms: number): void;
    push_video(data: Uint8Array, pts_us: number, dts_us: number, is_keyframe: boolean): void;
    /** Opus packet (0x7F 0xE0 AU header prepended inside). */
    push_audio(data: Uint8Array, pts_us: number): void;
    /** Interleaved f32 PCM ([-1,1), 48 kHz); AES3-wrapped as s302m. */
    push_pcm(samples: Float32Array, pts_us: number): void;
    push_pcm_pid(pid: number, samples: Float32Array, pts_us: number): void;
    /** Fresh drain of accumulated TS output (whole 188-B packets). */
    poll(): Uint8Array;
    free(): void;
  }
}

/** TS demuxer behind the same emcc module (js/demux_engine.js), consumed
 *  by the vendored page/vendor/demux.ts. TsEvent kind: 0 pat, 1 pmt,
 *  2 pes, 3 random_access, 4 error, 5 pcm. Field views are fresh
 *  JS-owned typed-array copies (fieldView in the wrapper). */
declare module "*js/demux_engine.js" {
  export default function init(): Promise<unknown>;

  export class TsEvent {
    readonly kind: number;
    readonly pid: number;
    readonly pts: number;
    readonly dts: number;
    readonly stream_type: number;
    readonly program_num: number;
    readonly randomAccess: boolean;
    readonly text: string;
    readonly data: Uint8Array;
    readonly nalOffsets: Uint32Array;
    readonly nalTypes: Uint8Array;
    readonly samples: Float32Array;
    /** PMT events: flat [pid0, stream_type0, pid1, stream_type1, …]. */
    pmtEntries(): Uint16Array;
    /** PMT events: per-entry format id ("" when none). */
    pmtFormatIds(): string[];
    free(): void;
  }

  export class DebugSnapshot {
    readonly programNum: number;
    readonly pmtPid: number;
    readonly pmtPids: Uint16Array;
    readonly pmtStreamTypes: Uint8Array;
    readonly pmtFormatIds: string[];
    readonly pids: Uint16Array;
    readonly pesCounts: Float64Array;
    readonly byteTotals: Float64Array;
    readonly bitratesMbps: Float64Array;
    readonly raCounts: Float64Array;
    readonly lastPts: Float64Array;
    readonly lastDts: Float64Array;
    readonly ptsJumps: Float64Array;
    readonly ccErrors: Float64Array;
    readonly teiCounts: Float64Array;
    readonly pusiCounts: Float64Array;
    readonly scramblingCounts: Float64Array;
    readonly afControlCounts: Float64Array;
    readonly pcrPids: Uint16Array;
    readonly pcrIntervalsMs: Float64Array;
    readonly pcrJitterMs: Float64Array;
    readonly nalPids: Uint16Array;
    readonly nalStats: Float64Array;
    readonly errorT: Float64Array;
    readonly errorMsg: string[];
    readonly ringT: Float64Array;
    readonly ringPid: Uint16Array;
    readonly ringKind: Uint8Array;
    readonly ringPts: Float64Array;
    readonly ringDts: Float64Array;
    readonly ringSize: Float64Array;
    readonly ringRa: Uint8Array;
    readonly ringTei: Uint8Array;
    readonly ringPusi: Uint8Array;
    readonly ringNal: Uint8Array;
    readonly ringNalOffsets: Uint32Array;
    readonly ringHex: string[];
    free(): void;
  }

  export interface MeterSnapshot {
    pids: ArrayLike<number>;
    channelCounts: ArrayLike<number>;
    peaks: ArrayLike<number>;
    rms: ArrayLike<number>;
    clips: ArrayLike<number>;
    lufs: ArrayLike<number>;
    phase: ArrayLike<number>;
    scopeL: ArrayLike<number>;
    scopeR: ArrayLike<number>;
    spectrum: ArrayLike<number>;
    selectedPid: number;
    selectedChannel: number;
    free(): void;
  }

  export class TsDemuxer {
    constructor();
    feed(bytes: Uint8Array): TsEvent[];
    debugSnapshot(): DebugSnapshot;
    meter_snapshot(): MeterSnapshot;
    setMeterSelection(pid: number, channel: number): void;
    free(): void;
  }
}

// MediaStreamTrackGenerator (Chromium's insertable-streams extension,
// used by the vendored page/vendor/decode.ts audio pipelines) is not in
// TypeScript's DOM lib — minimal declaration for the surface decode.ts
// touches (constructor + writable).
interface MediaStreamTrackGenerator extends MediaStreamTrack {
  readonly writable: WritableStream<AudioData> | null;
}
declare var MediaStreamTrackGenerator: {
  prototype: MediaStreamTrackGenerator;
  new (init: { kind: string }): MediaStreamTrackGenerator;
};
