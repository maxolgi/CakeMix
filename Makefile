# CakeMix — WASM Audio Mixer
# All commands. No push — that's the user's job.

RUST_TARGET := wasm32-unknown-unknown
CRATE_DIR := crates/mixer-wasm

.PHONY: build-wasm build-web build-node test-native test-wasm test-all check clean serve serve-tls ci clippy fmt-check build-ui build-worklet build-quicsrt-engine build-all

# Build for wasm32-unknown-unknown (first gate)
build-wasm:
	cargo build --target $(RUST_TARGET)

# Build wasm-pack for web (AudioWorklet)
build-web:
	cd $(CRATE_DIR) && wasm-pack build --target web --release

# Build wasm-pack for nodejs (testing)
build-node:
	cd $(CRATE_DIR) && wasm-pack build --target nodejs --release

# Native Rust tests (no browser/wasm needed)
test-native:
	cargo test

# WASM tests via node: the JS known-answer runner + the #[wasm_bindgen_test]
# suite (bus_parallel, channel_params, channel_tap, known_answer, pcm_fifo).
test-wasm: build-node
	node $(CRATE_DIR)/tests/run_tests.mjs
	cd $(CRATE_DIR) && wasm-pack test --node --release

# Everything
test-all: test-native test-wasm

# Quick check
check:
	cargo check --target $(RUST_TARGET)

clean:
	cargo clean
	rm -rf $(CRATE_DIR)/pkg web

# Build WASM pkg + start the server (http://localhost:8200).
# The server rust-embeds web/ (incl. the staged QuicSRT engine at
# web/build/) and crates/mixer-wasm/pkg at COMPILE time — both are built
# first. build-ui wipes web/ (generated dir), so build-worklet runs after
# it.
serve: build-web build-ui build-worklet
	cargo run -p cakemix-server -- --no-tls --port 8200

# Same but with HTTPS (auto-generates self-signed cert)
serve-tls: build-web build-ui build-worklet
	cargo run -p cakemix-server -- --port 8200


# Full CI check (matches .github/workflows/ci.yml). Order matters: the rust
# gates compile cakemix-server, which needs all rust-embed inputs present.
ci: build-web build-quicsrt-engine build-ui build-worklet fmt-check clippy test-native test-wasm
	@echo "✓ All CI checks pass"

# Clippy lints
clippy:
	cargo clippy --all-targets -- -D warnings

# Format check. Scoped to the workspace members explicitly (`--all` would
# also follow dependencies outside this repo and flag files that aren't
# ours to fix).
fmt-check:
	cargo fmt -p mixer-wasm -p cakemix-server -- --check


# Build SolidJS frontend → web/ (generated, gitignored; the server embeds it
# at compile time). Wipes web/ first so stale hashed assets never linger,
# then stages the QuicSRT engine into web/build/ BEFORE vite runs: vite has
# emptyOutDir: false, so it won't wipe web/build/ — and the engine MUST stay
# a standalone file (the emcc module self-imports for its pthreads workers
# and fetches its .wasm relative to itself). Hence the build-quicsrt-engine
# prerequisite.
build-ui: build-quicsrt-engine
	rm -rf web
	mkdir -p web/build
	@for f in vendor/QuicSRT-wasm/build/srt_engine.mjs vendor/QuicSRT-wasm/build/srt_engine.wasm; do \
		[ -f $$f ] || { echo "error: $$f missing — run: make build-quicsrt-engine" >&2; exit 1; }; \
	done
	cp vendor/QuicSRT-wasm/build/srt_engine.mjs vendor/QuicSRT-wasm/build/srt_engine.wasm web/build/
	cd frontend && { [ -d node_modules ] || npm ci; } && npx vite build

# Worklet bundle → web/mixer-worklet-processor.js (polyfill + wasm glue +
# processor, from frontend/src/worklet/ + the mixer pkg).
build-worklet: build-web
	node build/build-worklet.js

# Build the QuicSRT engine module with emcc: libquicsrt's receiver-only
# flavor + the srt fork + the single engine module →
# vendor/QuicSRT-wasm/build/srt_engine.{mjs,wasm} (gitignored). `setup`
# inits the submodule's own deps (deps/libquicsrt + its nested deps/srt);
# needs emsdk — ~/emsdk on this machine. Idempotent/incremental.
build-quicsrt-engine:
	cd vendor/QuicSRT-wasm && ./build.sh setup && ./build.sh lib && ./build.sh engine

# Full rebuild: WASM + UI + worklet.
# build-ui wipes web/, so build-worklet (which writes into web/) runs last.
build-all: build-web build-ui build-worklet
