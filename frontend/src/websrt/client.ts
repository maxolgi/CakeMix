// WebSRT receive client — build plumbing only (real PCM wiring into the
// mixer is a later milestone).
//
// The receive worker is imported at source level from the vendor/WebSRT
// submodule (docs/embedding.md "Supported embed model"). Vite bundles it as
// a module worker chunk and emits its srt-wasm + mpeg2ts-wasm binaries; the
// ts-muxer glue (publish path) is imported here so its wasm is emitted too.
// Requires build/build-websrt-wasm.sh to have run (stages the wasm glue at
// vendor/WebSRT/web/wasm/, which the worker's imports expect).

import type { WorkerCmd } from "../../../vendor/WebSRT/web/src/worker";
import initTsMuxer from "../../../vendor/WebSRT/web/wasm/ts-muxer-wasm/ts_muxer_wasm.js";

export { initTsMuxer };

export interface WebsrtInit {
  /** WebTransport URL of the gateway, e.g. https://host:4433/wt?stream=name */
  url: string;
  /** DER SHA-256 of a self-signed gateway cert, or null for real/mkcert PKI. */
  certHash: Uint8Array | null;
  /** TSBPD latency in ms (the glass-to-glass buffer). */
  latencyMs: number;
}

export interface WebsrtReceiver {
  /** The receive worker. Listen for `{ type: "batch", msgs }` WorkerMsg events. */
  worker: Worker;
  /** Post a command to the worker (init is already sent by initWebsrtReceiver). */
  send(cmd: WorkerCmd): void;
  terminate(): void;
}

export function initWebsrtReceiver(opts: WebsrtInit): WebsrtReceiver {
  const worker = new Worker(
    new URL("../../../vendor/WebSRT/web/src/worker.ts", import.meta.url),
    { type: "module" },
  );
  const send = (cmd: WorkerCmd) => worker.postMessage(cmd);
  send({ cmd: "init", url: opts.url, certHash: opts.certHash, latencyMs: opts.latencyMs });
  return { worker, send, terminate: () => worker.terminate() };
}
