import { defineConfig, type Plugin } from "vite";
import solid from "vite-plugin-solid";

// Keep the emcc engine module out of every bundle. The vendored wrappers
// (vendor/QuicSRT-wasm/js/engine_module.js) load it via
// `import('../build/srt_engine.mjs')`; the module self-imports to spawn
// its pthread workers and fetches its .wasm relative to itself, so it
// must be served standalone at /build/srt_engine.mjs (+ .wasm) under
// web/. Resolving to an absolute URL specifier makes the import survive
// any chunk depth (main graph or worker chunks) as a runtime fetch.
function externalSrtEngine(): Plugin {
  return {
    name: "external-srt-engine",
    enforce: "pre",
    resolveId(id) {
      if (id.endsWith("build/srt_engine.mjs")) {
        // external:"absolute" keeps the leading "/" — a plain `true`
        // external would be rewritten to a chunk-relative path by
        // Rollup's makeAbsoluteExternalsRelative default.
        return { id: "/build/srt_engine.mjs", external: "absolute" };
      }
      return null;
    },
  };
}

export default defineConfig({
  plugins: [solid(), externalSrtEngine()],
  worker: {
    // Vite bundles module workers with the worker config, not the main
    // plugins — the receive worker imports the engine wrapper, so the
    // externalization must apply there too (separate plugin instance).
    plugins: () => [externalSrtEngine()],
  },
  server: {
    port: 3000,
    proxy: {
      "/pkg": "http://localhost:8200",
      "/mixer-worklet-processor.js": "http://localhost:8200",
    },
  },
  build: {
    outDir: "../web",
    emptyOutDir: false,
    rollupOptions: {
      output: {
        entryFileNames: "assets/[name].js",
        chunkFileNames: "assets/[name].js",
        assetFileNames: "assets/[name].[ext]",
      },
    },
  },
});
