// WebSRT receiver store — the UI-facing contract for the receive path.
//
// Owns the single receive worker instance (created via ./client so Vite
// emits the worker chunk), relays PCM into the mixer AudioWorklet and
// auto-maps every audio PID to consecutive mixer input channels
// (INTEGRATION.md "PCM Handoff Contract" / "PID Mapping").
//
// PCM path: one MessageChannel per connect — port1 transferred to the
// worker (its 'pcm-port' cmd), port2 to the mixer worklet — so raw pcm
// flows worker→worklet with zero main-thread hops (vendor/WebSRT
// docs/embedding.md "pcm-port"; the worker splits pcm batches onto the
// port, control/stats stay on the parent channel). A fresh channel per
// connect is required: the worker is recreated on reconnect and
// transferred ports die with it.
//
// Mapping policy (executed worklet-side so the direct path needs no
// main-thread round trip — the worklet posts "pid-mapped" events back,
// which feed websrtPids()):
// - A PID is mapped on its FIRST 'pcm' message. The channelCount carried
//   there is authoritative (auto-detected from the AES3 frame header by the
//   WebSRT demuxer — INTEGRATION.md "Channel Count Discovery"); the PMT
//   never triggers mapping.
// - PIDs are packed consecutively from mixer channel 0, capped at 128
//   channels total. Overflow PIDs report chStart -1 and their PCM is
//   dropped (counted in websrtDroppedPcm(), as are pre-WASM drops).
// - The parent-channel relay remains as a fallback (worker pcm-port not
//   wired, e.g. mid-handshake): identical worklet-side auto-mapping, and
//   counted in websrtRelayPcm() so the direct path is verifiable — it
//   should stay 0 in normal operation.
//
// Worklet messages (see web/worklet-template.js):
//   { type: "pcm-port", port }                       → direct pcm channel
//   { type: "pcm",       pid, samples }              → fallback relay
//   { type: "unmap-pid", pid }                       → mixer.unmap_pid
//   { type: "pid-mapped", pid, chStart, channelCount }  (worklet → main)
//   { type: "pcm-dropped", total }                      (worklet → main)

import { createSignal } from "solid-js";
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

/** Mixer input-channel cap (AGENTS.md: 128 input strips max). */
const MAX_MIXER_CHANNELS = 128;

const [status, setStatus] = createSignal<WebsrtStatus>("disconnected");
const [statusDetail, setStatusDetail] = createSignal("");
const [pids, setPids] = createSignal<WebsrtPidInfo[]>([]);
const [latencyMs, setLatencyMsSignal] = createSignal(120);
const [droppedPcm, setDroppedPcm] = createSignal(0);
const [relayPcm, setRelayPcm] = createSignal(0);

export const websrtStatus = status;
/** Last log/error/stats line from the receive path. */
export const websrtStatusDetail = statusDetail;
/** Discovered audio PIDs and their mixer channel assignment. */
export const websrtPids = pids;
/** TSBPD latency in ms (the glass-to-glass buffer). Applies on next connect. */
export const websrtLatencyMs = latencyMs;
/** PCM dropped worklet-side (WASM not ready, or PID beyond the 128-ch cap). Cumulative. */
export const websrtDroppedPcm = droppedPcm;
/** PCM that arrived via the parent-channel fallback instead of the direct
 *  port — 0 in normal operation (proves the direct path is live). */
export const websrtRelayPcm = relayPcm;

export function setWebsrtLatencyMs(ms: number): void {
  setLatencyMsSignal(ms);
}

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

let worker: Worker | null = null;

/** Connect: parse the target URL (empty = this page's gateway, stream
 *  "default"), fetch the target origin's /cert-hash.js for its cert hash +
 *  WT port, build the WebTransport URL, start the receive worker and post
 *  'init'. User-triggered (settings drawer). */
export async function connectWebsrt(): Promise<void> {
  if (worker) return;
  userGestureUnlock(); // synchronous: keep the click's autoplay gesture
  let target: URL | null = null;
  const raw = targetUrl().trim();
  if (raw) {
    try { target = new URL(raw); }
    catch {
      setStatus("error");
      setStatusDetail(`invalid URL: ${raw}`);
      return;
    }
  }
  setStatus("connecting");
  setStatusDetail("resolving cert-hash.js…");
  try {
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
    const url = buildWtUrl(wtHost, wtPort, streamName, token);
    const certHash = certHashHex ? hexToBytes(certHashHex) : null;
    const w = startWorker();
    setStatusDetail(certHashHex
      ? `connecting to ${url} (self-signed, hash ${certHashHex.slice(0, 8)}…)`
      : `connecting to ${url} (mkcert/PKI)`);
    const cmd: WorkerCmd = { cmd: "init", url, certHash, latencyMs: latencyMs() };
    w.postMessage(cmd);
    // Direct pcm channel (queued behind init — the worker processes
    // cmds in order, so no pcm can bypass it once the stream starts).
    const chan = new MessageChannel();
    w.postMessage({ cmd: "pcm-port", port: chan.port1 } as WorkerCmd, [chan.port1]);
    sendToWorklet({ type: "pcm-port", port: chan.port2 }, [chan.port2]);
  } catch (e) {
    terminateWorker();
    setStatus("error");
    setStatusDetail(e instanceof Error ? e.message : String(e));
  }
}

/** Disconnect: unmap every mapped PID in the worklet, close the direct
 *  pcm channel (also resets the worklet-side auto-mapper so a reconnect
 *  maps from channel 0), terminate the worker, reset the discovered-PID
 *  list. */
export function disconnectWebsrt(): void {
  if (worker) {
    unmapAll();
    sendToWorklet({ type: "pcm-port", port: null });
    terminateWorker();
    setStatusDetail("disconnected");
  }
  setPids([]);
  setStatus("disconnected");
}

function startWorker(): Worker {
  const w = createWebsrtWorker();
  w.onmessage = (e: MessageEvent) => {
    const data = e.data as WorkerMsg;
    if (data.type === "batch") {
      for (const m of data.msgs) onWorkerMsg(m);
    }
  };
  w.onerror = (e: ErrorEvent) => {
    setStatus("error");
    setStatusDetail(`worker error: ${e.message}`);
  };
  worker = w;
  return w;
}

function terminateWorker(): void {
  worker?.terminate();
  worker = null;
}

function unmapAll(): void {
  for (const p of pids()) {
    if (p.chStart >= 0) sendToWorklet({ type: "unmap-pid", pid: p.pid });
  }
}

function onWorkerMsg(msg: WorkerMsg): void {
  switch (msg.type) {
    case "pcm":
      onPcm(msg.pid, msg.samples);
      break;
    case "pmt":
      // Informational only — mapping happens on first PCM per PID.
      setStatusDetail(`PMT: video pid ${msg.videoPid}, audio pid ${msg.audioPid}`);
      break;
    case "log":
      setStatusDetail(msg.msg);
      break;
    case "stats":
      setStatusDetail(
        `rtt ${msg.stats.rttMs.toFixed(0)} ms · loss ${msg.stats.rxLoss} · dropped ${msg.stats.rxDropped} · ${(msg.stats.bandwidthBps / 1e6).toFixed(2)} Mb/s`,
      );
      break;
    case "wtReady":
      setStatus("connected");
      setStatusDetail("WebTransport ready — awaiting stream");
      break;
    case "wtClosed":
      // Stream is definitively over — full reset so a reconnect re-maps PIDs
      // from channel 0 instead of leaking the old allocation.
      unmapAll();
      sendToWorklet({ type: "pcm-port", port: null });
      terminateWorker();
      setPids([]);
      if (msg.error) {
        setStatus("error");
        setStatusDetail(msg.error);
      } else {
        setStatus("disconnected");
        setStatusDetail("WebTransport closed");
      }
      break;
    case "close":
      setStatusDetail("stream closed by peer");
      break;
  }
}

// ── Worklet events (direct path bookkeeping) ──────────────────────────────

/** Worklet "pid-mapped": a PID was auto-mapped on its first pcm.
 *  chStart -1 = capped (128-channel cap); its PCM is dropped worklet-side. */
export function onWorkletPidMapped(msg: {
  pid: number;
  chStart: number;
  channelCount: number;
}): void {
  if (pids().some((p) => p.pid === msg.pid)) return;
  setPids([...pids(), { pid: msg.pid, channelCount: msg.channelCount, chStart: msg.chStart }]);
  if (msg.chStart < 0) {
    setStatusDetail(
      `pid ${msg.pid}: +${msg.channelCount} ch exceeds the ${MAX_MIXER_CHANNELS}-channel cap — dropping`,
    );
  }
}

/** Worklet "pcm-dropped": cumulative drop count (WASM not ready, or cap).
 *  Absolute — the worklet is the single counting authority. */
export function onWorkletPcmDropped(total: number): void {
  setDroppedPcm(total);
}

// Fallback parent-channel pcm path: the worker posts pcm on its parent
// channel only while no pcm-port is wired. Relay to the worklet (which
// auto-maps identically) and count — 0 in normal operation.
function onPcm(pid: number, samples: Float32Array): void {
  setRelayPcm((n) => n + 1);
  sendToWorklet({ type: "pcm", pid, samples }, [samples.buffer]);
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
