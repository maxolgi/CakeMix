// QuicSRT receive client — worker bootstrap for the quicsrt store
// (frontend/src/quicsrt/store.ts owns the workers, one per receive
// session).
//
// The receive worker is imported at source level from the
// vendor/QuicSRT-wasm submodule (page/vendor/worker.ts — the reference
// player's worker, retargeted at the C++ engine wrapper js/srt_engine.js;
// its WorkerCmd/WorkerMsg contracts are unchanged from the Rust-engine
// era, so store.ts needs no protocol changes). Vite bundles it as a
// module worker chunk, but the emcc engine module itself stays OUT of the
// bundle (vite.config.ts externalSrtEngine): it self-imports to spawn its
// pthread workers and fetches its .wasm relative to itself, so it is
// served standalone at /build/srt_engine.mjs (+ .wasm) under web/. The
// pthreads engine requires COOP/COEP — the server sends those headers.

import type { WorkerCmd, WorkerMsg } from "../../../vendor/QuicSRT-wasm/page/vendor/worker";

export type { WorkerCmd, WorkerMsg };

/** Create the QuicSRT receive worker. The `new URL(..., import.meta.url)`
 *  module-worker construction is what makes Vite emit the worker chunk.
 *  All worker output arrives batched:
 *  `{ type: "batch", msgs: WorkerMsg[] }` (see vendor worker.ts flushOutgoing). */
export function createQuicsrtWorker(): Worker {
  return new Worker(
    new URL("../../../vendor/QuicSRT-wasm/page/vendor/worker.ts", import.meta.url),
    { type: "module" },
  );
}
