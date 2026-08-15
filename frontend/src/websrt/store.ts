// WebSRT receiver store — the UI-facing contract for the receive path.
//
// Owns the single receive worker instance (created via ./client so Vite
// emits the worker chunk), relays PCM into the mixer AudioWorklet and
// auto-maps every audio PID to consecutive mixer input channels
// (INTEGRATION.md "PCM Handoff Contract" / "PID Mapping").
//
// Mapping policy:
// - A PID is mapped on its FIRST 'pcm' message. The channelCount carried
//   there is authoritative (auto-detected from the AES3 frame header by the
//   WebSRT demuxer — INTEGRATION.md "Channel Count Discovery"); the PMT
//   never triggers mapping.
// - PIDs are packed consecutively from mixer channel 0, capped at 128
//   channels total. Overflow PIDs are logged once and their PCM dropped
//   (chStart stays -1).
// - While the mixer WASM is not ready (wasmReady() === false) PCM is
//   dropped silently and counted (websrtDroppedPcm()); first-PCM mapping is
//   deferred with it, so a PID maps only once PCM can actually flow.
//
// Worklet relay messages (see web/mixer-worklet-processor.js):
//   { type: "map-pid",   pid, chStart, channelCount }  → mixer.map_pid
//   { type: "pcm",       pid, samples }                → mixer.feed_pcm
//   { type: "unmap-pid", pid }                         → mixer.unmap_pid

import { createSignal } from "solid-js";
import { createWebsrtWorker, type WorkerCmd, type WorkerMsg } from "./client";
import { wasmReady, sendToWorklet } from "../stores/mixer";
import { userGestureUnlock } from "../audio/unlock";

export type WebsrtStatus = "disconnected" | "connecting" | "connected" | "error";

export interface WebsrtPidInfo {
  pid: number;
  channelCount: number;
  /** First mixer channel for this PID; -1 = not mapped (128-ch cap exceeded). */
  chStart: number;
  /** True once this PID has been mapped and PCM relayed. */
  seenPcm: boolean;
}

/** Mixer input-channel cap (AGENTS.md: 128 input strips max). */
const MAX_MIXER_CHANNELS = 128;

const [status, setStatus] = createSignal<WebsrtStatus>("disconnected");
const [statusDetail, setStatusDetail] = createSignal("");
const [pids, setPids] = createSignal<WebsrtPidInfo[]>([]);
const [latencyMs, setLatencyMsSignal] = createSignal(1000);
const [droppedPcm, setDroppedPcm] = createSignal(0);

export const websrtStatus = status;
/** Last log/error/stats line from the receive path. */
export const websrtStatusDetail = statusDetail;
/** Discovered audio PIDs and their mixer channel assignment. */
export const websrtPids = pids;
/** TSBPD latency in ms (the glass-to-glass buffer). Applies on next connect. */
export const websrtLatencyMs = latencyMs;
/** PCM messages dropped: mixer WASM not ready, or PID beyond the 128-ch cap. */
export const websrtDroppedPcm = droppedPcm;

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
let nextChStart = 0;

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
  } catch (e) {
    terminateWorker();
    setStatus("error");
    setStatusDetail(e instanceof Error ? e.message : String(e));
  }
}

/** Disconnect: unmap every mapped PID in the worklet, terminate the worker,
 *  reset the channel allocator and the discovered-PID list. */
export function disconnectWebsrt(): void {
  if (worker) {
    unmapAll();
    terminateWorker();
    setStatusDetail("disconnected");
  }
  setPids([]);
  nextChStart = 0;
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
      onPcm(msg.pid, msg.channelCount, msg.samples);
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
      terminateWorker();
      setPids([]);
      nextChStart = 0;
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

function onPcm(pid: number, channelCount: number, samples: Float32Array): void {
  // Mixer WASM not instantiated yet — drop (counted) and defer mapping, so
  // the PID maps on the first PCM that can actually be fed to the mixer.
  if (!wasmReady()) {
    setDroppedPcm((n) => n + 1);
    return;
  }
  let info = pids().find((p) => p.pid === pid);
  if (!info) {
    info = { pid, channelCount, chStart: -1, seenPcm: false };
    if (nextChStart + channelCount > MAX_MIXER_CHANNELS) {
      setStatusDetail(
        `pid ${pid}: +${channelCount} ch exceeds the ${MAX_MIXER_CHANNELS}-channel cap — dropping`,
      );
    } else {
      info.chStart = nextChStart;
      nextChStart += channelCount;
    }
    setPids([...pids(), info]);
  }
  if (info.chStart < 0) {
    setDroppedPcm((n) => n + 1);
    return;
  }
  if (!info.seenPcm) {
    // First PCM for this PID — channelCount is authoritative here.
    sendToWorklet({ type: "map-pid", pid, chStart: info.chStart, channelCount: info.channelCount });
    setPids((list) => list.map((p) => (p.pid === pid ? { ...p, seenPcm: true } : p)));
  }
  // Zero-copy relay: the worker transferred the samples' ArrayBuffer to us;
  // transfer it on into the worklet.
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
