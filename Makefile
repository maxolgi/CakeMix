# CakeMix — WASM Audio Mixer
# All commands. No push — that's the user's job.

RUST_TARGET := wasm32-unknown-unknown
CRATE_DIR := crates/mixer-wasm

.PHONY: build-wasm build-web build-node test-native test-wasm test-all check clean serve serve-tls ci clippy fmt-check build-ui build-worklet build-websrt-wasm build-websrt-web bump-websrt build-all

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
# The server rust-embeds web/, crates/mixer-wasm/pkg and
# vendor/WebSRT/web/dist at COMPILE time — all three are built first.
# build-ui wipes web/ (generated dir), so build-worklet runs after it.
serve: build-web build-websrt-web build-ui build-worklet
	cargo run -p cakemix-server -- --no-tls --port 8200

# Same but with HTTPS (auto-generates self-signed cert)
serve-tls: build-web build-websrt-web build-ui build-worklet
	cargo run -p cakemix-server -- --port 8200


# Full CI check (matches .github/workflows/ci.yml). Order matters: the rust
# gates compile cakemix-server, which needs all rust-embed inputs present.
ci: build-web build-websrt-wasm build-websrt-web build-ui build-worklet fmt-check clippy test-native test-wasm
	@echo "✓ All CI checks pass"

# Clippy lints
clippy:
	cargo clippy --all-targets -- -D warnings

# Format check. Scoped to workspace members, not `--all`: rustfmt follows the
# cakemix-server path dependency into vendor/WebSRT and would reformat the
# pinned submodule (its files were formatted by an older rustfmt — not ours
# to fix here).
fmt-check:
	cargo fmt -p mixer-wasm -p cakemix-server -- --check


# Build SolidJS frontend → web/ (generated, gitignored; the server embeds it
# at compile time). Wipes web/ first so stale hashed assets never linger.
# Needs the WebSRT wasm staged in vendor/WebSRT/web/wasm/ before Vite bundles
# the receive worker — hence the build-websrt-wasm prerequisite.
build-ui: build-websrt-wasm
	rm -rf web
	cd frontend && npx vite build

# Worklet bundle → web/mixer-worklet-processor.js (polyfill + wasm glue +
# processor, from frontend/src/worklet/ + the mixer pkg).
build-worklet: build-web
	node build/build-worklet.js

# Build WebSRT wasm crates from vendor/WebSRT + stage where the submodule's
# worker expects them (vendor/WebSRT/web/wasm/). Required before build-ui
# bundles the receive worker. Idempotent.
build-websrt-wasm:
	bash build/build-websrt-wasm.sh

# Build the reference WebSRT web app (vendor/WebSRT/web) into its dist/.
# The server embeds dist/ at COMPILE time (ref_web.rs rust-embed) — after
# this target, rebuild cakemix-server for changes to show on :8201.
build-websrt-web:
	cd vendor/WebSRT/web && { [ -d node_modules ] || npm ci; } && npx vite build

# Update the vendor/WebSRT submodule pin to the remote's current HEAD.
# The new pin is NOT auto-committed — commit the gitlink yourself.
bump-websrt:
	git submodule update --remote vendor/WebSRT
	@echo "Reminder: vendor/WebSRT pin changed — commit it: git add vendor/WebSRT && git commit"

# Full rebuild: WASM + reference web app + UI + worklet.
# build-ui wipes web/, so build-worklet (which writes into web/) runs last.
build-all: build-web build-websrt-web build-ui build-worklet
