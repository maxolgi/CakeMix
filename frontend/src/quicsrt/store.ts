// QuicSRT receiver store — the UI-facing contract for the receive path.
//
// Owns N concurrent receive sessions (each created via ./client so Vite
// emits the worker chunk), relays PCM into the mixer AudioWorklet and
// mirrors the worklet-side auto-mapping of every audio PID to consecutive
// mixer input channels (README.md "PCM handoff contract").
//
// Sessions: one Web Worker + one MessageChannel per session, ids
// incrementing from 1 (0 is the worklet's legacy bucket for messages
// that omit sessionId — every message this store sends carries an
// explicit id ≥ 1). WASM pid keys are (sessionId << 16) | pid (the
// worklet encodes). The pre-sessions single-session exports (quicsrtStatus,
// connectQuicsrt, …) are views/operations on the PRIMARY session = the
// first live one, so the settings drawer keeps working until it migrates
// to the session list (quicsrtSessions()).
//
// PCM path: one MessageChannel per session — port1 transferred to the
// worker (its 'pcm-port' cmd), port2 to the mixer worklet — so raw pcm
// flows worker→worklet with zero main-thread hops (the vendor worker's
// "pcm-port" command; the worker splits pcm batches onto the
// port, control/stats stay on the parent channel). A fresh channel per
// session is required: transferred ports die with their worker.
//
// Mapping policy (executed worklet-side so the direct path needs no
// main-thread round trip — the worklet posts "pid-mapped" events back,
// which feed each session's pids list):
// - A PID is mapped on its FIRST 'pcm' message. The channelCount carried
//   there is authoritative (auto-detected from the AES3 frame header by the
//   QuicSRT demuxer); the PMT never triggers mapping.
// - PIDs are packed consecutively from mixer channel 0, capped at 128
//   channels total. Overflow PIDs report chStart -1 and their PCM is
//   dropped (counted per-session in `dropped` and globally in
//   quicsrtDroppedPcm(), as are pre-WASM drops).
// - The parent-channel relay remains as a fallback (worker pcm-port not
//   wired, e.g. mid-handshake): identical worklet-side auto-mapping, and
//   counted per-session in `relay` and cumulatively in quicsrtRelayPcm()
//   so the direct path is verifiable — it should stay 0 in normal
//   operation.
//
// Worklet messages (see web/worklet-template.js):
//   { type: "pcm-port", port, sessionId }   → direct pcm channel (null port + sessionId = session teardown)
//   { type: "pcm",       pid, samples, sessionId } → fallback relay
//   { type: "unmap-pid", pid, sessionId }   → mixer.unmap_pid
//   { type: "pid-mapped", pid, sessionId, chStart, channelCount }  (worklet → main)
//   { type: "pcm-dropped", total }          (worklet → main, global cumulative)

import { createSignal, createMemo, type Accessor } from "solid-js";
import { createQuicsrtWorker, type WorkerCmd, type WorkerMsg } from "./client";
import { DEFAULT_GATEWAY_URL, resolveCertHash, hexToBytes } from "./gateway";
import { sendToWorklet } from "../stores/mixer";
import { userGestureUnlock } from "../audio/unlock";

export type QuicsrtStatus = "disconnected" | "connecting" | "connected" | "error";

export interface QuicsrtPidInfo {
  pid: number;
  channelCount: number;
  /** First mixer channel for this PID; -1 = not mapped (128-ch cap exceeded). */
  chStart: number;
}

/** One receive session as the UI sees it (quicsrtSessions()). */
export interface QuicsrtSessionInfo {
  id: number;
  /** Resolved subscribe URL (cert-hash + WT port + stream/token applied). */
  url: string;
  /** TSBPD latency the session's worker was init'ed with. */
  latencyMs: number;
  status: QuicsrtStatus;
  statusDetail: string;
  pids: QuicsrtPidInfo[];
  /** PIDs of this session past the 128-ch cap — their PCM is dropped
   *  worklet-side. (The cumulative dropped-pcm count is global — the
   *  worklet cannot attribute drops per session; see quicsrtDroppedPcm.) */
  dropped: number;
  /** PCM batches that arrived via this session's parent-channel fallback. */
  relay: number;
}

/** Internal session record — QuicsrtSessionInfo plus its worker. */
interface QuicsrtSession extends QuicsrtSessionInfo {
  worker: Worker | null;
}

/** Mixer input-channel cap (AGENTS.md: 128 input strips max). */
const MAX_MIXER_CHANNELS = 128;

const [sessions, setSessions] = createSignal<QuicsrtSession[]>([]);

/** Session ids start at 1 — 0 is the worklet's legacy default for
 *  sessionId-less messages, so every message from here carries an
 *  explicit id ≥ 1 and the legacy bucket stays unused. */
let nextSessionId = 1;

/** Live sessions for the UI (workers stripped). First entry = the primary
 *  session the single-session exports below view. */
export const quicsrtSessions: Accessor<QuicsrtSessionInfo[]> = createMemo(() =>
  sessions().map((s) => ({
    id: s.id, url: s.url, latencyMs: s.latencyMs, status: s.status,
    statusDetail: s.statusDetail, pids: s.pids, dropped: s.dropped, relay: s.relay,
  })),
);

// What the single-session views show while no session is live: the final
// status/detail of the last torn-down primary, so an error line or
// "disconnected" survives its record's removal.
const [idleStatus, setIdleStatus] = createSignal<QuicsrtStatus>("disconnected");
const [idleDetail, setIdleDetail] = createSignal("");

const primary = createMemo(() => sessions()[0]);

export const quicsrtStatus: Accessor<QuicsrtStatus> = createMemo(
  () => primary()?.status ?? idleStatus(),
);
/** Last log/error/stats line from the receive path. */
export const quicsrtStatusDetail: Accessor<string> = createMemo(
  () => primary()?.statusDetail ?? idleDetail(),
);
/** Discovered audio PIDs and their mixer channel assignment. */
export const quicsrtPids: Accessor<QuicsrtPidInfo[]> = createMemo(
  () => primary()?.pids ?? [],
);

const [defaultLatencyMs, setDefaultLatencyMsSignal] = createSignal(120);
/** TSBPD latency in ms (the glass-to-glass buffer). Default for new
 *  connects (quicsrtLatencyMs-override per call aside); applies on next
 *  connect. */
export const quicsrtLatencyMs: Accessor<number> = defaultLatencyMs;

export function setQuicsrtLatencyMs(ms: number): void {
  setDefaultLatencyMsSignal(ms);
}

/** PCM dropped worklet-side (WASM not ready, or PID beyond the 128-ch
 *  cap). Global cumulative — the worklet is the single counting authority. */
const [droppedPcm, setDroppedPcm] = createSignal(0);
export const quicsrtDroppedPcm = droppedPcm;
/** PCM that arrived via a parent-channel fallback instead of the direct
 *  port — 0 in normal operation (proves the direct path is live).
 *  Lifetime cumulative across sessions. */
const [relayPcm, setRelayPcm] = createSignal(0);
export const quicsrtRelayPcm = relayPcm;

// ── Connection target ─────────────────────────────────────────────────────
// One URL (settings drawer): the target's web-viewer URL, e.g.
//   https://192.168.1.214:5173/?stream=audio
// Host + stream name + token are parsed from it; the gateway's cert hash
// and WT port are fetched from that origin via our /api/cert-hash proxy —
// exactly what the target's own viewer does. Empty = the default gateway
// (DEFAULT_GATEWAY_URL), stream "default".
const initialParams = new URLSearchParams(location.search);
const initialTargetUrl = initialParams.get("host")
  ? `https://${initialParams.get("host")}${initialParams.get("port") ? ":" + initialParams.get("port") : ""}/?stream=${initialParams.get("stream") ?? "default"}`
  : "";
const [targetUrl, setTargetUrl] = createSignal(initialTargetUrl);
export const quicsrtTarget = { url: targetUrl, setUrl: setTargetUrl };

// ── Sessions ───────────────────────────────────────────────────────────────

/** Connect one session: parse the target URL (empty = the default
 *  gateway, stream "default"), resolve the target origin's cert hash +
 *  WT port via our /api/cert-hash proxy, build the WebTransport URL,
 *  start a receive worker and post 'init'. User-triggered. Concurrent
 *  sessions are independent (one worker + one direct pcm channel each). */
export async function connectQuicsrtSession(url: string, latencyMs?: number): Promise<void> {
  userGestureUnlock(); // synchronous: keep the click's autoplay gesture
  const id = nextSessionId++;
  const lat = latencyMs ?? defaultLatencyMs();
  setSessions((list) => [...list, {
    id, url: "", latencyMs: lat, status: "connecting",
    statusDetail: "resolving cert-hash.js…", pids: [], dropped: 0, relay: 0,
    worker: null,
  }]);
  try {
    const t = await resolveTarget(url);
    patchSession(id, {
      url: t.wtUrl,
      statusDetail: t.certHashHex
        ? `connecting to ${t.wtUrl} (self-signed, hash ${t.certHashHex.slice(0, 8)}…)`
        : `connecting to ${t.wtUrl} (mkcert/PKI)`,
    });
    const w = startWorker(id);
    const cmd: WorkerCmd = { cmd: "init", url: t.wtUrl, certHash: t.certHash, latencyMs: lat };
    w.postMessage(cmd);
    // Direct pcm channel (queued behind init — the worker processes
    // cmds in order, so no pcm can bypass it once the stream starts).
    const chan = new MessageChannel();
    w.postMessage({ cmd: "pcm-port", port: chan.port1 } as WorkerCmd, [chan.port1]);
    sendToWorklet({ type: "pcm-port", port: chan.port2, sessionId: id }, [chan.port2]);
  } catch (e) {
    teardownSession(id, "error", e instanceof Error ? e.message : String(e));
  }
}

/** Disconnect one session: unmap its mapped PIDs in the worklet (per-
 *  session — other sessions keep their channels), close its direct pcm
 *  channel (the worklet forgets only this session's mappings), terminate
 *  its worker, remove the record. */
export function disconnectQuicsrtSession(id: number): void {
  teardownSession(id, "disconnected", "disconnected");
}

/** Update a session's latency. A live worker was init'ed with the old
 *  value, so — exactly like setQuicsrtLatencyMs — the new value applies on
 *  the next connect (and becomes the default for new sessions); the
 *  record updates immediately so the UI shows it. */
export function setQuicsrtSessionLatency(id: number, ms: number): void {
  if (!getSession(id)) return;
  patchSession(id, { latencyMs: ms });
  setDefaultLatencyMsSignal(ms);
}

// ── Single-session back-compat (settings drawer) ──────────────────────────
// The pre-sessions API, reimplemented on the session core: connect/
// disconnect operate on the primary session via the drawer's target URL;
// the status/pids signals above are primary-session views. No-ops /
// idle values when no session is live.

export function connectQuicsrt(): Promise<void> {
  if (primary()) return Promise.resolve(); // one drawer session at a time
  return connectQuicsrtSession(targetUrl(), defaultLatencyMs());
}

export function disconnectQuicsrt(): void {
  const p = primary();
  if (p) disconnectQuicsrtSession(p.id);
}

// ── Session internals ──────────────────────────────────────────────────────

function getSession(id: number): QuicsrtSession | undefined {
  return sessions().find((s) => s.id === id);
}

function patchSession(id: number, patch: Partial<QuicsrtSession>): void {
  setSessions((list) => list.map((s) => (s.id === id ? { ...s, ...patch } : s)));
}

function startWorker(id: number): Worker {
  const w = createQuicsrtWorker();
  w.onmessage = (e: MessageEvent) => {
    const data = e.data as WorkerMsg;
    if (data.type === "batch") {
      for (const m of data.msgs) onWorkerMsg(m, id);
    }
  };
  w.onerror = (e: ErrorEvent) => {
    patchSession(id, { status: "error", statusDetail: `worker error: ${e.message}` });
  };
  patchSession(id, { worker: w });
  return w;
}

/** Tear down one session: unmap its PIDs, close its direct pcm port (null
 *  port WITH a sessionId — the worklet forgets only this session's
 *  mappings; its global channel cursor never rewinds per-session), kill
 *  its worker, remove the record. If it was the primary, its final
 *  status/detail move to the idle views so they outlive the record. */
function teardownSession(id: number, finalStatus: QuicsrtStatus, finalDetail: string): void {
  const s = getSession(id);
  if (!s) return;
  const wasPrimary = sessions()[0]?.id === id;
  unmapSessionPids(s);
  sendToWorklet({ type: "pcm-port", port: null, sessionId: id });
  s.worker?.terminate();
  setSessions((list) => list.filter((x) => x.id !== id));
  if (wasPrimary) {
    setIdleStatus(finalStatus);
    setIdleDetail(finalDetail);
  }
}

function unmapSessionPids(s: QuicsrtSession): void {
  for (const p of s.pids) {
    if (p.chStart >= 0) sendToWorklet({ type: "unmap-pid", pid: p.pid, sessionId: s.id });
  }
}

function onWorkerMsg(msg: WorkerMsg, id: number): void {
  if (!getSession(id)) return; // session already torn down — drop late msgs
  switch (msg.type) {
    case "pcm":
      onPcm(msg.pid, msg.samples, id);
      break;
    case "pmt":
      // Informational only — mapping happens on first PCM per PID.
      patchSession(id, { statusDetail: `PMT: video pid ${msg.videoPid}, audio pid ${msg.audioPid}` });
      break;
    case "log":
      patchSession(id, { statusDetail: msg.msg });
      break;
    case "stats":
      patchSession(id, {
        statusDetail:
          `rtt ${msg.stats.rttMs.toFixed(0)} ms · loss ${msg.stats.rxLoss} · dropped ${msg.stats.rxDropped} · ${(msg.stats.bandwidthBps / 1e6).toFixed(2)} Mb/s`,
      });
      break;
    case "wtReady":
      patchSession(id, { status: "connected", statusDetail: "WebTransport ready — awaiting stream" });
      break;
    case "wtClosed":
      // Stream is definitively over — full session teardown so a reconnect
      // of the same target maps PIDs freshly instead of leaking this
      // session's allocation.
      teardownSession(id, msg.error ? "error" : "disconnected", msg.error ?? "WebTransport closed");
      break;
    case "close":
      patchSession(id, { statusDetail: "stream closed by peer" });
      break;
  }
}

// ── Worklet events (direct path bookkeeping) ──────────────────────────────

/** Worklet "pid-mapped": a PID was auto-mapped on its first pcm, scoped
 *  to sessionId. Upserts into that session's pids list; chStart -1 =
 *  capped (128-channel cap), its PCM is dropped worklet-side. Unknown
 *  session ids (record already torn down) are ignored. */
export function onWorkletPidMapped(msg: {
  pid: number;
  sessionId: number;
  chStart: number;
  channelCount: number;
}): void {
  const s = getSession(msg.sessionId);
  if (!s) return;
  const entry: QuicsrtPidInfo = { pid: msg.pid, channelCount: msg.channelCount, chStart: msg.chStart };
  const known = s.pids.some((p) => p.pid === msg.pid);
  patchSession(s.id, {
    pids: known ? s.pids.map((p) => (p.pid === msg.pid ? entry : p)) : [...s.pids, entry],
  });
  if (msg.chStart < 0) {
    patchSession(s.id, {
      dropped: s.dropped + 1,
      statusDetail:
        `pid ${msg.pid}: +${msg.channelCount} ch exceeds the ${MAX_MIXER_CHANNELS}-channel cap — dropping`,
    });
  }
}

/** Worklet "pcm-dropped": cumulative drop count (WASM not ready, or cap).
 *  Absolute and global — the worklet is the single counting authority
 *  and cannot attribute drops per session. */
export function onWorkletPcmDropped(total: number): void {
  setDroppedPcm(total);
}

// Fallback parent-channel pcm path: the worker posts pcm on its parent
// channel only while no pcm-port is wired. Relay to the worklet tagged
// with the session id (which auto-maps identically) and count — 0 in
// normal operation.
function onPcm(pid: number, samples: Float32Array, id: number): void {
  const s = getSession(id);
  if (s) patchSession(id, { relay: s.relay + 1 });
  setRelayPcm((n) => n + 1);
  sendToWorklet({ type: "pcm", pid, samples, sessionId: id }, [samples.buffer]);
}

// ── Target resolution (shared by every connect) ───────────────────────────

/** Resolve a drawer target URL ("" = the default gateway, stream
 *  "default") to what the worker needs: the target origin's cert hash +
 *  WT port via our /api/cert-hash proxy, the subscribe WT URL, the parsed
 *  token. Throws on an unparseable URL or a failed cert-hash fetch. */
interface ResolvedTarget {
  wtUrl: string;
  certHash: Uint8Array | null;
  certHashHex: string | null;
}

async function resolveTarget(raw: string): Promise<ResolvedTarget> {
  let target: URL | null = null;
  const trimmed = raw.trim();
  if (trimmed) {
    try { target = new URL(trimmed); }
    catch { throw new Error(`invalid URL: ${trimmed}`); }
  }
  let certHashHex: string | null;
  let wtPort: number;
  let wtHost: string;
  let streamName: string;
  let token: string | null = null;
  if (target) {
    ({ certHashHex, wtPort } = await resolveCertHash(target.origin));
    wtHost = target.hostname;
    streamName = target.searchParams.get("stream") ?? "default";
    token = target.searchParams.get("token");
  } else {
    const gw = new URL(DEFAULT_GATEWAY_URL);
    ({ certHashHex, wtPort } = await resolveCertHash(gw.origin));
    wtHost = gw.hostname;
    streamName = "default";
  }
  const wtUrl = buildWtUrl(wtHost, wtPort, streamName, token);
  return { wtUrl, certHash: certHashHex ? hexToBytes(certHashHex) : null, certHashHex };
}

/** Build the subscribe WT URL: fixed /wt path, WT port from the target's
 *  cert-hash.js (4435 fallback), ?stream= + ?token passthrough. */
function buildWtUrl(wtHost: string, wtPort: number, streamName: string, token: string | null): string {
  const qp = new URLSearchParams({ stream: streamName });
  if (token) qp.set("token", token);
  return `https://${wtHost}:${wtPort || 4435}/wt?${qp}`;
}
