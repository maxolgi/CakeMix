// WebSRT receive client — worker bootstrap for the websrt store
// (frontend/src/websrt/store.ts owns the single worker instance).
//
// The receive worker is imported at source level from the vendor/WebSRT
// submodule (docs/embedding.md "Supported embed model"). Vite bundles it as
// a module worker chunk and emits its srt-wasm + mpeg2ts-wasm binaries; the
// ts-muxer glue (publish path) is imported here so its wasm is emitted too.
// Requires build/build-websrt-wasm.sh to have run (stages the wasm glue at
// vendor/WebSRT/web/wasm/, which the worker's imports expect).

import type { WorkerCmd, WorkerMsg } from "../../../vendor/WebSRT/web/src/worker";
import initTsMuxer from "../../../vendor/WebSRT/web/wasm/ts-muxer-wasm/ts_muxer_wasm.js";

export { initTsMuxer };
export type { WorkerCmd, WorkerMsg };

/** Create the WebSRT receive worker. The `new URL(..., import.meta.url)`
 *  module-worker construction is what makes Vite emit the worker chunk and
 *  its wasm assets. All worker output arrives batched:
 *  `{ type: "batch", msgs: WorkerMsg[] }` (see vendor worker.ts flushOutgoing). */
export function createWebsrtWorker(): Worker {
  return new Worker(
    new URL("../../../vendor/WebSRT/web/src/worker.ts", import.meta.url),
    { type: "module" },
  );
}
