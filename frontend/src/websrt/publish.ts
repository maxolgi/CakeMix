// WebSRT publish store — the UI-facing contract for the PCM publish path.
//
// Publishes the mixer's output as SMPTE 302M PCM (no codecs) to the WebSRT
// gateway via ./publish-worker.ts: 2ch = master stereo on one PID, 16–128ch
// = ceil(N/2) stereo PIDs (multi-PID muxing in the worker) fed by the
// worklet's channel direct-out tap. Mirrors ./store.ts (the receive
// store): same cert-hash.js discovery + ?certHash/?host/?port overrides,
// same status/detail signal shape, batched worker messages
// ({type:'batch', msgs}). The default publish target is the SAME gateway
// the receive path resolves to; the stream name defaults to "cakemix"
// (?pubstream= overrides).
//
// PCM flow: worklet output tap (pub-start/pub-stop, see
// web/worklet-template.js) → App.tsx relay → relayPubPcm() →
// {cmd:'pcm'} transferred into the worker.

import { createSignal } from "solid-js";
import { sendToWorklet } from "../stores/mixer";
import { websrtLatencyMs } from "./store";
import { userGestureUnlock } from "../audio/unlock";
import type { PubCmd, PubMsg, PubStats } from "./publish-worker";

export type PublishStatus = "disconnected" | "connecting" | "connected" | "error";

/** Allowed publish channel counts: 2 = master stereo pair; 16–128 =
 *  per-channel direct outs (mono per channel, post input-gain/gate/comp/
 *  EQ/fader, pre-pan; muted channels publish silence). */
export type PublishChannels = 2 | 16 | 32 | 64 | 128;

const STREAM_NAME = new URLSearchParams(location.search).get("pubstream") ?? "cakemix";

const [status, setStatus] = createSignal<PublishStatus>("disconnected");
const [statusDetail, setStatusDetail] = createSignal("");
const [stats, setStats] = createSignal<PubStats | null>(null);
const [target, setTarget] = createSignal("");
const [channels, setChannels] = createSignal<PublishChannels>(2);

export const publishStatus = status;
/** Last log/error/stats line from the publish path. */
export const publishStatusDetail = statusDetail;
/** Publish-link stats from the worker's 1s timer; null while disconnected. */
export const publishStats = stats;
/** "host:port" of the gateway being published to (filled on connect). */
export const publishTarget = target;
/** Output channel count: 2|16|32|64|128, packed as ceil(N/2) stereo s302m PIDs. */
export const publishChannels = channels;

/** Stream name the master mix is published as (?pubstream= overrides). */
export function publishStreamName(): string {
  return STREAM_NAME;
}

/** Change the publish channel count. Only takes effect while disconnected
 *  (applied at next connect — the live muxer's PID set is fixed at init);
 *  silently ignored otherwise. */
export function setPublishChannels(n: PublishChannels): void {
  if (status() === "disconnected") setChannels(n);
}

let worker: Worker | null = null;

/** Connect: resolve the gateway cert hash (same discovery + overrides as the
 *  receive store — ?certHash=<64-hex>|null, ?host, ?port), build
 *  https://<host>:<port>/wt?publish=<name>, start the publish worker, post
 *  'init' and switch on the worklet's master-output tap. User-triggered
 *  (WebSRTPanel). */
export async function connectPublish(): Promise<void> {
  if (worker) return;
  userGestureUnlock(); // synchronous: keep the click's autoplay gesture
  setStatus("connecting");
  const certHashParam = new URLSearchParams(location.search).get("certHash");
  setStatusDetail("resolving cert-hash.js…");
  try {
    let certHashHex: string | null;
    let wtPort: number;
    if (certHashParam !== null) {
      // URL override — skip the same-origin fetch entirely (same rationale
      // as store.ts: ?host/?port may point at a different gateway).
      certHashHex = certHashParam === "null" ? null : certHashParam;
      wtPort = 0;
    } else {
      ({ certHashHex, wtPort } = await resolveCertHash());
    }
    const url = buildPublishWtUrl(wtPort);
    const certHash = certHashHex ? hexToBytes(certHashHex) : null;
    const w = startWorker();
    setTarget(new URL(url).host);
    setStatusDetail(certHashHex
      ? `connecting to ${url} (self-signed, hash ${certHashHex.slice(0, 8)}…)`
      : `connecting to ${url} (mkcert/PKI)`);
    const cmd: PubCmd = {
      cmd: "init", url, certHash,
      latencyMs: websrtLatencyMs(), channels: channels(),
    };
    w.postMessage(cmd);
    // Output tap on: the worklet posts {type:'pub-pcm', samples, ptsUs,
    // channels} — the master pair for 2ch, channel direct outs otherwise
    // (silence while the mixer is stopped, so the stream stays continuous).
    sendToWorklet({ type: "pub-start", channels: channels() });
  } catch (e) {
    sendToWorklet({ type: "pub-stop" });
    terminateWorker();
    setTarget("");
    setStatus("error");
    setStatusDetail(e instanceof Error ? e.message : String(e));
  }
}

/** Disconnect: switch the worklet tap off, stop the worker, reset state. */
export function disconnectPublish(): void {
  sendToWorklet({ type: "pub-stop" });
  if (worker) {
    worker.postMessage({ cmd: "stop" });
    terminateWorker();
    setStatusDetail("disconnected");
  }
  setStats(null);
  setTarget("");
  setStatus("disconnected");
}

/** Relay one PCM batch from the worklet into the publish worker (zero-copy:
 *  the worklet transferred the buffer to the main thread; hand it straight
 *  through). Called by App.tsx on pub-pcm. msgChannels is the batch's
 *  channel count when the caller forwards it; the fallback is this store's
 *  configured count — the same value pub-start armed the worklet tap with,
 *  and it cannot change while connected. */
export function relayPubPcm(samples: Float32Array, ptsUs: number, msgChannels?: number): void {
  if (!worker) return;
  worker.postMessage({ cmd: "pcm", samples, ptsUs, channels: msgChannels ?? channels() }, [samples.buffer]);
}

function startWorker(): Worker {
  const w = new Worker(new URL("./publish-worker.ts", import.meta.url), { type: "module" });
  w.onmessage = (e: MessageEvent) => {
    const data = e.data as PubMsg;
    if (data.type === "batch") {
      for (const m of data.msgs) onWorkerMsg(m);
    }
  };
  w.onerror = (e: ErrorEvent) => {
    setStatus("error");
    setStatusDetail(`publish worker error: ${e.message}`);
  };
  worker = w;
  return w;
}

function terminateWorker(): void {
  worker?.terminate();
  worker = null;
}

function onWorkerMsg(msg: PubMsg): void {
  switch (msg.type) {
    case "log":
      setStatusDetail(msg.msg);
      break;
    case "wtReady":
      setStatus("connected");
      setStatusDetail("WebTransport ready — SRT handshake…");
      break;
    case "handshakeComplete":
      setStatusDetail("handshake complete — publishing master mix");
      break;
    case "stats":
      setStats(msg.stats);
      setStatusDetail(
        `↑${msg.stats.kbps} kb/s · rtt ${msg.stats.rttMs.toFixed(0)} ms · tx loss ${msg.stats.txLoss}`,
      );
      break;
    case "wtClosed":
      // Publish session is definitively over — full reset, no auto-reconnect.
      sendToWorklet({ type: "pub-stop" });
      terminateWorker();
      setStats(null);
      setTarget("");
      if (msg.error) {
        setStatus("error");
        setStatusDetail(msg.error);
      } else {
        setStatus("disconnected");
        setStatusDetail("WebTransport closed");
      }
      break;
    case "close":
      setStatusDetail("publish stream closed by peer");
      break;
  }
}

/** cert-hash.js shape — identical to store.ts resolveCertHash (duplicated
 *  rather than imported: store.ts keeps this private). */
interface CertHashInfo {
  certHashHex: string | null;
  wtPort: number;
}

async function resolveCertHash(): Promise<CertHashInfo> {
  const resp = await fetch("cert-hash.js");
  if (!resp.ok) {
    throw new Error(`No cert-hash.js (HTTP ${resp.status}) — is the gateway running?`);
  }
  const text = await resp.text();
  const hash = text.match(/window\.CERT_HASH\s*=\s*(?:"([^"]*)"|null)/);
  const port = text.match(/window\.WT_PORT\s*=\s*(\d+)/);
  if (!hash || !port) {
    throw new Error("cert-hash.js is not parseable");
  }
  return { certHashHex: hash[1] ?? null, wtPort: parseInt(port[1], 10) };
}

/** Hex → 32 bytes, tolerant of ':' / whitespace separators.
 *  Duplicated from store.ts (kept private there). */
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

/** Build the publish WT URL — store.ts buildWtUrl with ?publish= instead of
 *  ?stream= (gateway.rs: "?publish=<name> publishes"). Same host/port
 *  resolution: ?host > page host ("localhost" → 127.0.0.1), ?port >
 *  cert-hash WT_PORT > 4433; ?token passthrough. */
function buildPublishWtUrl(wtPort: number): string {
  const urlParams = new URLSearchParams(location.search);
  const pageHost = location.hostname || "127.0.0.1";
  const wtHost = urlParams.get("host") ?? (pageHost === "localhost" ? "127.0.0.1" : pageHost);
  const port = urlParams.get("port") ?? String(wtPort || 4433);
  const qp = new URLSearchParams({ publish: STREAM_NAME });
  const token = urlParams.get("token");
  if (token) qp.set("token", token);
  return `https://${wtHost}:${port}/wt?${qp}`;
}
