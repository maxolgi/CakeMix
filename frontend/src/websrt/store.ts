// WebSRT receiver store — the UI-facing contract for the receive path.
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
// worklet encodes). The pre-sessions single-session exports (websrtStatus,
// connectWebsrt, …) are views/operations on the PRIMARY session = the
// first live one, so the settings drawer keeps working until it migrates
// to the session list (websrtSessions()).
//
// PCM path: one MessageChannel per session — port1 transferred to the
// worker (its 'pcm-port' cmd), port2 to the mixer worklet — so raw pcm
// flows worker→worklet with zero main-thread hops (vendor/WebSRT
// docs/embedding.md "pcm-port"; the worker splits pcm batches onto the
// port, control/stats stay on the parent channel). A fresh channel per
// session is required: transferred ports die with their worker.
//
// Mapping policy (executed worklet-side so the direct path needs no
// main-thread round trip — the worklet posts "pid-mapped" events back,
// which feed each session's pids list):
// - A PID is mapped on its FIRST 'pcm' message. The channelCount carried
//   there is authoritative (auto-detected from the AES3 frame header by the
//   WebSRT demuxer); the PMT never triggers mapping.
// - PIDs are packed consecutively from mixer channel 0, capped at 128
//   channels total. Overflow PIDs report chStart -1 and their PCM is
//   dropped (counted per-session in `dropped` and globally in
//   websrtDroppedPcm(), as are pre-WASM drops).
// - The parent-channel relay remains as a fallback (worker pcm-port not
//   wired, e.g. mid-handshake): identical worklet-side auto-mapping, and
//   counted per-session in `relay` and cumulatively in websrtRelayPcm()
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
import { createWebsrtWorker, type WorkerCmd, type WorkerMsg } from "./client";
import { sendToWorklet } from "../stores/mixer";
import { userGestureUnlock } from "../audio/unlock";

export type WebsrtStatus = "disconnected" | "connecting" | "connected" | "error";

export interface WebsrtPidInfo {
  pid: number;
  channelCount: number;
  /** First mixer channel for this PID; -1 = not mapped (128-ch cap exceeded). */
  chStart: number;
}

/** One receive session as the UI sees it (websrtSessions()). */
export interface WebsrtSessionInfo {
  id: number;
  /** Resolved subscribe URL (cert-hash + WT port + stream/token applied). */
  url: string;
  /** TSBPD latency the session's worker was init'ed with. */
  latencyMs: number;
  status: WebsrtStatus;
  statusDetail: string;
  pids: WebsrtPidInfo[];
  /** PIDs of this session past the 128-ch cap — their PCM is dropped
   *  worklet-side. (The cumulative dropped-pcm count is global — the
   *  worklet cannot attribute drops per session; see websrtDroppedPcm.) */
  dropped: number;
  /** PCM batches that arrived via this session's parent-channel fallback. */
  relay: number;
}

/** Internal session record — WebsrtSessionInfo plus its worker. */
interface WebsrtSession extends WebsrtSessionInfo {
  worker: Worker | null;
}

/** Mixer input-channel cap (AGENTS.md: 128 input strips max). */
const MAX_MIXER_CHANNELS = 128;

const [sessions, setSessions] = createSignal<WebsrtSession[]>([]);

/** Session ids start at 1 — 0 is the worklet's legacy default for
 *  sessionId-less messages, so every message from here carries an
 *  explicit id ≥ 1 and the legacy bucket stays unused. */
let nextSessionId = 1;

/** Live sessions for the UI (workers stripped). First entry = the primary
 *  session the single-session exports below view. */
export const websrtSessions: Accessor<WebsrtSessionInfo[]> = createMemo(() =>
  sessions().map((s) => ({
    id: s.id, url: s.url, latencyMs: s.latencyMs, status: s.status,
    statusDetail: s.statusDetail, pids: s.pids, dropped: s.dropped, relay: s.relay,
  })),
);

// What the single-session views show while no session is live: the final
// status/detail of the last torn-down primary, so an error line or
// "disconnected" survives its record's removal.
const [idleStatus, setIdleStatus] = createSignal<WebsrtStatus>("disconnected");
const [idleDetail, setIdleDetail] = createSignal("");

const primary = createMemo(() => sessions()[0]);

export const websrtStatus: Accessor<WebsrtStatus> = createMemo(
  () => primary()?.status ?? idleStatus(),
);
/** Last log/error/stats line from the receive path. */
export const websrtStatusDetail: Accessor<string> = createMemo(
  () => primary()?.statusDetail ?? idleDetail(),
);
/** Discovered audio PIDs and their mixer channel assignment. */
export const websrtPids: Accessor<WebsrtPidInfo[]> = createMemo(
  () => primary()?.pids ?? [],
);

const [defaultLatencyMs, setDefaultLatencyMsSignal] = createSignal(120);
/** TSBPD latency in ms (the glass-to-glass buffer). Default for new
 *  connects (websrtLatencyMs-override per call aside); applies on next
 *  connect. */
export const websrtLatencyMs: Accessor<number> = defaultLatencyMs;

export function setWebsrtLatencyMs(ms: number): void {
  setDefaultLatencyMsSignal(ms);
}

/** PCM dropped worklet-side (WASM not ready, or PID beyond the 128-ch
 *  cap). Global cumulative — the worklet is the single counting authority. */
const [droppedPcm, setDroppedPcm] = createSignal(0);
export const websrtDroppedPcm = droppedPcm;
/** PCM that arrived via a parent-channel fallback instead of the direct
 *  port — 0 in normal operation (proves the direct path is live).
 *  Lifetime cumulative across sessions. */
const [relayPcm, setRelayPcm] = createSignal(0);
export const websrtRelayPcm = relayPcm;

// ── Connection target ─────────────────────────────────────────────────────
// One URL (settings drawer): the target's web-viewer URL, e.g.
//   https://192.168.1.214:5173/?stream=audio
// Host + stream name + token are parsed from it; the gateway's cert hash
// and WT port are fetched from that origin's /cert-hash.js — exactly what
// the target's own viewer does. Empty = the gateway serving this page,
// stream "default" (same-origin cert-hash.js).
const initialParams = new URLSearchParams(location.search);
const initialTargetUrl = initialParams.get("host")
  ? `https://${initialParams.get("host")}${initialParams.get("port") ? ":" + initialParams.get("port") : ""}/?stream=${initialParams.get("stream") ?? "default"}`
  : "";
const [targetUrl, setTargetUrl] = createSignal(initialTargetUrl);
export const websrtTarget = { url: targetUrl, setUrl: setTargetUrl };

// ── Sessions ───────────────────────────────────────────────────────────────

/** Connect one session: parse the target URL (empty = this page's
 *  gateway, stream "default"), fetch the target origin's /cert-hash.js
 *  for its cert hash + WT port, build the WebTransport URL, start a
 *  receive worker and post 'init'. User-triggered. Concurrent sessions
 *  are independent (one worker + one direct pcm channel each). */
export async function connectWebsrtSession(url: string, latencyMs?: number): Promise<void> {
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
export function disconnectWebsrtSession(id: number): void {
  teardownSession(id, "disconnected", "disconnected");
}

/** Update a session's latency. A live worker was init'ed with the old
 *  value, so — exactly like setWebsrtLatencyMs — the new value applies on
 *  the next connect (and becomes the default for new sessions); the
 *  record updates immediately so the UI shows it. */
export function setWebsrtSessionLatency(id: number, ms: number): void {
  if (!getSession(id)) return;
  patchSession(id, { latencyMs: ms });
  setDefaultLatencyMsSignal(ms);
}

// ── Single-session back-compat (settings drawer) ──────────────────────────
// The pre-sessions API, reimplemented on the session core: connect/
// disconnect operate on the primary session via the drawer's target URL;
// the status/pids signals above are primary-session views. No-ops /
// idle values when no session is live.

export function connectWebsrt(): Promise<void> {
  if (primary()) return Promise.resolve(); // one drawer session at a time
  return connectWebsrtSession(targetUrl(), defaultLatencyMs());
}

export function disconnectWebsrt(): void {
  const p = primary();
  if (p) disconnectWebsrtSession(p.id);
}

// ── Session internals ──────────────────────────────────────────────────────

function getSession(id: number): WebsrtSession | undefined {
  return sessions().find((s) => s.id === id);
}

function patchSession(id: number, patch: Partial<WebsrtSession>): void {
  setSessions((list) => list.map((s) => (s.id === id ? { ...s, ...patch } : s)));
}

function startWorker(id: number): Worker {
  const w = createWebsrtWorker();
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
function teardownSession(id: number, finalStatus: WebsrtStatus, finalDetail: string): void {
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

function unmapSessionPids(s: WebsrtSession): void {
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
  const entry: WebsrtPidInfo = { pid: msg.pid, channelCount: msg.channelCount, chStart: msg.chStart };
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

/** Resolve a drawer target URL ("" = this page's gateway, stream
 *  "default") to what the worker needs: the target origin's cert hash +
 *  WT port via its /cert-hash.js, the subscribe WT URL, the parsed
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
    ({ certHashHex, wtPort } = await resolveCertHash());
    const pageHost = location.hostname || "127.0.0.1";
    wtHost = pageHost === "localhost" ? "127.0.0.1" : pageHost;
    streamName = "default";
  }
  const wtUrl = buildWtUrl(wtHost, wtPort, streamName, token);
  return { wtUrl, certHash: certHashHex ? hexToBytes(certHashHex) : null, certHashHex };
}

/** Fetch + parse a gateway's cert-hash.js. Same origin when `origin` is
 *  omitted; a remote origin goes through OUR server's /api/cert-hash proxy
 *  (embedding.md "Delivering the cert hash cross-origin" — the browser
 *  cannot fetch another origin's cert-hash.js directly, CORS). Shape
 *  (written by websrt-gateway/src/main.rs):
 *      window.CERT_HASH = "<64 hex chars>";   …or null (mkcert/PKI mode)
 *      window.WT_PORT = 4433; */
interface CertHashInfo {
  certHashHex: string | null;
  wtPort: number;
}

async function resolveCertHash(origin?: string): Promise<CertHashInfo> {
  if (!origin) {
    const resp = await fetch("/cert-hash.js", { cache: "no-store" });
    if (!resp.ok) {
      throw new Error(`No cert-hash.js (HTTP ${resp.status}) — is the gateway running?`);
    }
    const text = await resp.text();
    return parseCertHashJs(text);
  }
  const resp = await fetch(`/api/cert-hash?url=${encodeURIComponent(origin)}`, { cache: "no-store" });
  const j = await resp.json().catch(() => null);
  if (!j) throw new Error(`proxy response not JSON (HTTP ${resp.status})`);
  if (j.error) throw new Error(String(j.error));
  return {
    certHashHex: typeof j.hash === "string" && j.hash.length === 64 ? j.hash : null,
    wtPort: Number(j.wtPort) || 4433,
  };
}

function parseCertHashJs(text: string): CertHashInfo {
  const hash = text.match(/window\.CERT_HASH\s*=\s*(?:"([^"]*)"|null)/);
  const port = text.match(/window\.WT_PORT\s*=\s*(\d+)/);
  if (!hash || !port) {
    throw new Error("cert-hash.js is not parseable");
  }
  return { certHashHex: hash[1] ?? null, wtPort: parseInt(port[1], 10) };
}

/** Hex → 32 bytes, tolerant of ':' / whitespace separators.
 *  Copied from vendor/WebSRT/web/src/shared/viewer.ts hexToBytes. */
function hexToBytes(hex: string): Uint8Array {
  const clean = hex.replace(/[:\s]/g, "");
  if (clean.length !== 64) {
    throw new Error(`expected 32-byte (64 hex char) hash, got ${clean.length} hex chars`);
  }
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++) {
    out[i] = parseInt(clean.substring(i * 2, i * 2 + 2), 16);
  }
  return out;
}

/** Build the subscribe WT URL: fixed /wt path, WT port from the target's
 *  cert-hash.js (4433 fallback), ?stream= + ?token passthrough. */
function buildWtUrl(wtHost: string, wtPort: number, streamName: string, token: string | null): string {
  const qp = new URLSearchParams({ stream: streamName });
  if (token) qp.set("token", token);
  return `https://${wtHost}:${wtPort || 4433}/wt?${qp}`;
}
