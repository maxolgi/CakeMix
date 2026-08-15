// CakeMix PCM publish worker — audio-only WebSRT publisher, NO codecs.
//
// Structurally mirrors vendor/WebSRT/web/src/stream-worker.ts (the reference
// publish worker): doInit's WebTransport open (datagrams readable/writable
// compat shims, getStats smoothedRtt → newWithLatencyAndRtt), the SRT loop
// (runSrtLoop: reader.read → rx.handle_datagram → poll(nowUs) →
// processActions), processActions (SendDatagram → writer.write),
// wt.closed handling, queue/batch message flushing, 1s stats timer.
//
// The audio path differs: instead of Opus WebCodecs, raw PCM arrives from
// the mixer AudioWorklet (relayed main-thread by publish.ts, see
// web/worklet-template.js _pubTap: 1024-frame interleaved batches, pts =
// frames/48000 µs since pub-start) and is muxed as SMPTE 302M by
// ts-muxer-wasm — N channels (2..128) packed as ceil(N/2) stereo PIDs.
// Nothing here touches WebCodecs.

import initSrt, { SrtReceiver, type SrtAction, type SrtStats } from "../../../vendor/WebSRT/web/wasm/srt-wasm/srt_wasm.js";
import initMux, { TsMuxer } from "../../../vendor/WebSRT/web/wasm/ts-muxer-wasm/ts_muxer_wasm.js";

/** Publish-link stats surfaced to the UI (subset of SrtStats). */
export interface PubStats {
  /** TS payload bitrate over the last stats interval (kbit/s). */
  kbps: number;
  /** SRT round-trip time estimate (ms). */
  rttMs: number;
  /** Cumulative retransmit-lost packets (tx side). */
  txLoss: number;
}

// v2: configurable channel count — the mixer is published as `channels`
// (2|16|32|64|128) packed as ceil(channels/2) stereo s302m PIDs. PID
// convention (matches the muxer's own multi-PID tests in
// vendor/WebSRT/crates/ts-muxer-wasm/src/lib.rs, ef18993): setAudioCodec
// fixes the first s302m stream at PID 0x101, addAudioPid registers
// 0x102, 0x103, … — so PID i carries input channels 2i (L) and 2i+1 (R),
// the same packing the receive path auto-maps by PMT discovery and the
// fixture stream_pcm.sh mirrors (one stereo pair per PID).
//
// Today only channels=2 carries real audio (the worklet's master-output
// tap, stereo); the N>2 path is exercised by the tmp/ muxer round-trip
// harness until the worklet input tap lands.
export type PubCmd =
  | { cmd: "init"; url: string; certHash: Uint8Array | null; latencyMs: number; channels: number }
  | { cmd: "pcm"; samples: Float32Array; ptsUs: number; channels: number }
  | { cmd: "stop" };

// Subset of stream-worker.ts's PublishMsg set (no credit/encode: there is
// no encoder back-pressure on the PCM path — the worklet tap is the source).
export type PubMsg =
  | { type: "log"; msg: string; cls?: string }
  | { type: "wtReady" }
  | { type: "handshakeComplete" }
  | { type: "wtClosed"; error?: string }
  | { type: "close" }
  | { type: "stats"; stats: PubStats }
  | { type: "batch"; msgs: PubMsg[] };

let rx: SrtReceiver | null = null;
let muxer: TsMuxer | null = null;
let wt: WebTransport | null = null;
let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
let writer: WritableStreamDefaultWriter<Uint8Array> | null = null;
let gen = 0;
let epoch = 0;
let prevTxLoss = 0;
let prevTxBytes = 0;
let lastStatsAt = 0;
let statsTimer: ReturnType<typeof setInterval> | null = null;
let pcmDroppedLogged = false;
let pcmChannelsMismatchLogged = false;

/** First audio PID (muxer's DEFAULT_AUDIO_PID — setAudioCodec's stream). */
const FIRST_PID = 0x101;
/** Channel count the muxer was configured for at init. */
let cfgChannels = 2;

let outgoing: PubMsg[] = [];
let inited = false;
let srtWasmReady = false;
let muxWasmReady = false;

// ─── Message handler ──────────────────────────────────────────────

self.onmessage = async (e: MessageEvent) => {
  const cmd = e.data as PubCmd;
  switch (cmd.cmd) {
    case "init":
      await doInit(cmd.url, cmd.certHash, cmd.latencyMs, cmd.channels);
      break;
    case "pcm":
      handlePcm(cmd.samples, cmd.ptsUs, cmd.channels);
      break;
    case "stop":
      gen++;
      doStop();
      break;
  }
  flushOutgoing();
};

// ─── Queue / flush ────────────────────────────────────────────────

function queue(msg: PubMsg) {
  outgoing.push(msg);
}

function flushOutgoing() {
  if (outgoing.length === 0) return;
  (self as unknown as Worker).postMessage({ type: "batch", msgs: outgoing });
  outgoing = [];
}

// ─── Init ─────────────────────────────────────────────────────────

async function doInit(url: string, certHash: Uint8Array | null, latencyMs: number, channels: number) {
  const myGen = ++gen;
  try {
    doStop();

    if (!srtWasmReady) { await initSrt(); srtWasmReady = true; }
    if (!muxWasmReady) { await initMux(); muxWasmReady = true; }
    if (myGen !== gen) return;

    epoch = performance.now();
    pcmDroppedLogged = false;
    pcmChannelsMismatchLogged = false;
    prevTxLoss = 0;
    prevTxBytes = 0;
    lastStatsAt = 0;
    cfgChannels = channels;

    // Audio-only TsMuxer: video off, ceil(channels/2) stereo s302m streams.
    // setAudioCodec registers the first at fixed PID 0x101; the rest are
    // consecutive PIDs via addAudioPid (muxer test convention, ef18993).
    const pidCount = Math.ceil(channels / 2);
    muxer = new TsMuxer();
    muxer.setVideoEnabled(false);
    muxer.setAudioCodec("s302m", 2);
    for (let i = 1; i < pidCount; i++) {
      muxer.addAudioPid(FIRST_PID + i, "s302m", 2);
    }
    const pidRange = pidCount > 1 ? `PIDs 0x101–0x${(0x100 + pidCount).toString(16)}` : "PID 0x101";
    queue({ type: "log", msg: `muxer: audio-only TS — ${channels}ch as ${pidCount}× stereo s302m (${pidRange})`, cls: "info" });

    inited = true;

    // WebTransport
    const opts: WebTransportOptions = {};
    if (certHash) {
      opts.serverCertificateHashes = [{ algorithm: "sha-256", value: certHash as BufferSource }];
    }
    wt = new WebTransport(url, opts);
    await wt.ready;
    if (myGen !== gen) { try { wt.close({}); } catch {} return; }

    let initialRttMs: number | undefined;
    try {
      const stats = await (wt as any).getStats();
      if (stats && typeof stats.smoothedRtt === "number" && stats.smoothedRtt > 0) {
        initialRttMs = stats.smoothedRtt;
      }
    } catch { /* getStats not supported */ }

    rx = initialRttMs !== undefined
      ? SrtReceiver.newWithLatencyAndRtt(latencyMs, initialRttMs)
      : SrtReceiver.newWithLatency(latencyMs);
    const dg = wt.datagrams as any;
    const datagrams = typeof dg === "function" ? dg() : dg;
    const readableStream = typeof datagrams.createReadable === "function"
      ? datagrams.createReadable()
      : datagrams.readable;
    const writableStream = typeof datagrams.createWritable === "function"
      ? datagrams.createWritable()
      : datagrams.writable;
    reader = readableStream.getReader();
    writer = writableStream.getWriter();

    wt.closed
      .then(() => { if (myGen === gen) { queue({ type: "wtClosed" }); flushOutgoing(); } })
      .catch((e) => { if (myGen === gen) { queue({ type: "wtClosed", error: String(e) }); flushOutgoing(); } });

    queue({ type: "wtReady" });
    flushOutgoing();
    runSrtLoop(myGen);

    statsTimer = setInterval(() => {
      if (!rx || !inited) return;
      const s = rx.getStats();
      if (!s) return;
      emitLossEvents(s);
      const now = performance.now();
      const dtS = lastStatsAt > 0 ? (now - lastStatsAt) / 1000 : 0;
      const kbps = dtS > 0 ? Math.round(((s.txBytes - prevTxBytes) * 8) / dtS / 1000) : 0;
      lastStatsAt = now;
      prevTxBytes = s.txBytes;
      queue({ type: "stats", stats: { kbps, rttMs: s.rttMs, txLoss: s.txLoss } });
      flushOutgoing();
    }, 1000);
  } catch (e) {
    if (myGen === gen) {
      doStop();
      queue({ type: "log", msg: `worker init failed: ${e}`, cls: "err" });
      queue({ type: "wtClosed", error: String(e) });
      flushOutgoing();
    }
  }
}

// ─── Stop ─────────────────────────────────────────────────────────

function doStop() {
  if (statsTimer) { clearInterval(statsTimer); statsTimer = null; }
  prevTxLoss = 0;
  prevTxBytes = 0;
  lastStatsAt = 0;
  pcmDroppedLogged = false;
  pcmChannelsMismatchLogged = false;

  if (muxer) {
    try { muxer.free(); } catch {}
    muxer = null;
  }

  const w = wt;
  wt = null;
  reader = null;
  writer = null;
  rx = null;
  inited = false;

  if (w) { try { w.close({}); } catch {} }
}

// ─── PCM → s302m → SRT ────────────────────────────────────────────

/** De-interleave N-channel PCM into ceil(N/2) stereo-interleaved buffers:
 *  PID i gets samples[2i::N] (L) + samples[2i+1::N] (R). channels===2
 *  passes the buffer through untouched (master stereo — zero copies).
 *  A trailing odd channel lands as the last PID's L with silent R. */
export function deinterleaveStereoPids(samples: Float32Array, channels: number): Float32Array[] {
  if (channels <= 2) return [samples];
  const frames = Math.floor(samples.length / channels);
  const pidCount = Math.ceil(channels / 2);
  const out: Float32Array[] = [];
  for (let p = 0; p < pidCount; p++) {
    const stereo = new Float32Array(frames * 2);
    const hasR = 2 * p + 1 < channels;
    for (let f = 0; f < frames; f++) {
      stereo[2 * f] = samples[f * channels + 2 * p];
      stereo[2 * f + 1] = hasR ? samples[f * channels + 2 * p + 1] : 0;
    }
    out.push(stereo);
  }
  return out;
}

function handlePcm(samples: Float32Array, ptsUs: number, channels: number) {
  if (!muxer || !rx || !inited) return;
  if (channels !== cfgChannels) {
    // Wiring bug (store/worklet disagreement) — dropping, not guessing.
    if (!pcmChannelsMismatchLogged) {
      pcmChannelsMismatchLogged = true;
      queue({ type: "log", msg: `pcm batch has ${channels}ch, stream configured for ${cfgChannels}ch — dropping`, cls: "err" });
      flushOutgoing();
    }
    return;
  }
  // Before the SRT handshake completes the sender half can't ship anything;
  // bufferless v1 drops the batch (logged once) — the worklet tap's pts
  // timeline keeps running, so the stream simply starts at the handshake.
  if (!rx.isHandshakeComplete()) {
    if (!pcmDroppedLogged) {
      pcmDroppedLogged = true;
      queue({ type: "log", msg: "handshake pending — dropping PCM until complete", cls: "info" });
      flushOutgoing();
    }
    return;
  }
  const pids = deinterleaveStereoPids(samples, channels);
  try {
    for (let i = 0; i < pids.length; i++) {
      muxer.push_pcm_pid(FIRST_PID + i, pids[i], ptsUs);
    }
  } catch (e) {
    // Muxer-side Rust panic (currently: write_pmt overflows its single TS
    // packet at ≥16 audio PIDs — upstream bug, needs PMT section splitting
    // in WebSRT). State may be corrupt; stop the session cleanly instead of
    // feeding every following batch into the same panic.
    queue({ type: "log", msg: `muxer error on pcm push: ${e} — stopping publish session`, cls: "err" });
    queue({ type: "wtClosed", error: `muxer: ${e}` });
    flushOutgoing();
    gen++;
    doStop();
    return;
  }
  flushTsToSrt();
}

function flushTsToSrt() {
  if (!muxer || !rx || !inited) return;
  if (!rx.isHandshakeComplete()) return;

  const tsBytes = muxer.poll();
  if (tsBytes.length === 0) return;

  const nowUs = (performance.now() - epoch) * 1000;
  const actions = rx.sendMessage(tsBytes, nowUs);
  processActions(actions);
  flushOutgoing();
}

// ─── SRT loop (same structure as stream-worker) ───────────────────

async function runSrtLoop(myGen: number) {
  const r = reader;
  if (!r) return;
  let readPromise = r.read();

  for (;;) {
    if (myGen !== gen || !rx || !inited) break;

    const POLL_MS = 5;
    let timeoutId: ReturnType<typeof setTimeout> | undefined;
    const readWithLabel = readPromise.then(
      (res) => ({ kind: "dgram" as const, res }),
      (err: unknown) => ({ kind: "read_error" as const, err }),
    );
    const tickPromise = new Promise<{ kind: "tick" }>((resolve) => {
      timeoutId = setTimeout(() => resolve({ kind: "tick" }), POLL_MS);
    });

    const winner = await Promise.race([readWithLabel, tickPromise]);
    if (timeoutId !== undefined) clearTimeout(timeoutId);

    if (myGen !== gen || !rx || !inited) break;

    const nowUs = (performance.now() - epoch) * 1000;

    if (winner.kind === "dgram") {
      if (winner.res.done) break;
      const value = winner.res.value;
      if (!value) break;
      processActions(rx.handle_datagram(value, nowUs));
      readPromise = r.read();
    } else if (winner.kind === "read_error") {
      if (myGen === gen) {
        queue({ type: "log", msg: `wt read: ${winner.err}`, cls: "err" });
        flushOutgoing();
      }
      break;
    }

    processActions(rx.poll(nowUs));
    flushOutgoing();
  }
}

function processActions(actions: SrtAction[]) {
  for (const a of actions) {
    try {
      switch (a.kind) {
        case 0:
          writeDatagram(a.takeData());
          break;
        case 1:
          break;
        case 2:
          queue({ type: "handshakeComplete" });
          flushTsToSrt();
          break;
        case 3:
          break;
        case 4:
          queue({ type: "close" });
          break;
        case 5:
          queue({ type: "log", msg: `srt: ${a.text}`, cls: "info" });
          break;
        default:
          break;
      }
    } finally {
      a.free();
    }
  }
}

function writeDatagram(bytes: Uint8Array) {
  const w = writer;
  if (!w) return;
  try {
    w.write(bytes).catch((e) => {
      queue({ type: "log", msg: `wt write: ${e}`, cls: "err" });
      flushOutgoing();
    });
  } catch (e) {
    queue({ type: "log", msg: `wt write: ${e}`, cls: "err" });
    flushOutgoing();
  }
}

// ─── Stats ────────────────────────────────────────────────────────

function emitLossEvents(s: SrtStats) {
  const newLoss = s.txLoss - prevTxLoss;
  if (newLoss > 0) {
    queue({ type: "log", msg: `SRT tx loss: ${newLoss} packets (total ${s.txLoss})`, cls: "err" });
  }
  prevTxLoss = s.txLoss;
}
