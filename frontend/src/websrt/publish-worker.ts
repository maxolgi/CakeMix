// CakeMix PCM publish worker — audio-only WebSRT publisher, NO codecs.
//
// Structurally mirrors vendor/WebSRT/web/src/stream-worker.ts (the reference
// publish worker): doInit's WebTransport open (datagrams readable/writable
// compat shims, getStats smoothedRtt → newWithLatencyAndRtt), the SRT loop
// (runSrtLoop: reader.read → rx.handle_datagram → poll(nowUs) →
// processActions), processActions (SendDatagram → writer.write),
// wt.closed handling, queue/batch message flushing, 1s stats timer.
//
// The audio path differs: instead of Opus WebCodecs, raw master-mix PCM
// arrives from the mixer AudioWorklet (relayed main-thread by publish.ts,
// see web/worklet-template.js _pubTap: 1024-frame interleaved stereo
// batches, pts = frames/48000 µs since pub-start) and is muxed as SMPTE
// 302M by ts-muxer-wasm. Nothing here touches WebCodecs.

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

// v1: one s302m stereo PID (the master bus). The message is shaped for the
// future multi-PID extension — { cmd:'init', …, pids: [{pid, channels}] }
// would carry one entry per mixer bus, registered via addAudioPid(). Today
// the muxer's push_pcm() always feeds its FIRST s302m stream (the one
// setAudioCodec configures, fixed PID 0x101), so `pid` is carried in the
// protocol but no extra PID is registered: addAudioPid(pid ≠ 0x101) would
// advertise a PMT entry that never carries data.
export type PubCmd =
  | { cmd: "init"; url: string; certHash: Uint8Array | null; latencyMs: number; pid: number; channels: number }
  | { cmd: "pcm"; samples: Float32Array; ptsUs: number }
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
      handlePcm(cmd.samples, cmd.ptsUs);
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
    prevTxLoss = 0;
    prevTxBytes = 0;
    lastStatsAt = 0;

    // Audio-only TsMuxer: video off, first (only) audio stream = s302m.
    muxer = new TsMuxer();
    muxer.setVideoEnabled(false);
    muxer.setAudioCodec("s302m", channels);
    queue({ type: "log", msg: `muxer: audio-only TS — s302m ${channels}ch (PID 0x101)`, cls: "info" });

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

function handlePcm(samples: Float32Array, ptsUs: number) {
  if (!muxer || !rx || !inited) return;
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
  muxer.push_pcm(samples, ptsUs);
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
