# CakeMix — WASM Audio Mixer
# All commands. No push — that's the user's job.

RUST_TARGET := wasm32-unknown-unknown
CRATE_DIR := crates/mixer-wasm

.PHONY: build-wasm build-web build-node test-native test-wasm test-all check clean serve serve-tls ci clippy fmt-check build-ui build-websrt-wasm build-websrt-web bump-websrt build-all

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

# WASM tests via node
test-wasm: build-node
	node $(CRATE_DIR)/tests/run_tests.mjs

# Everything
test-all: test-native test-wasm

# Quick check
check:
	cargo check --target $(RUST_TARGET)

clean:
	cargo clean
	rm -rf $(CRATE_DIR)/pkg

# Build WASM pkg + start the server (http://localhost:8200).
# build-ui pulls in build-websrt-wasm (the Vite build bundles the WebSRT
# receive worker, which needs the submodule's web/wasm/ staged first).
serve: build-web build-ui
	cargo run -p cakemix-server -- --no-tls --port 8200

# Same but with HTTPS (auto-generates self-signed cert)
serve-tls: build-web build-ui
	cargo run -p cakemix-server -- --port 8200


# Full CI check (matches .github/workflows/ci.yml)
ci: fmt-check clippy test-native test-wasm build-web build-websrt-wasm build-ui
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


# Build SolidJS frontend → web/ (one-time, no runtime dependency).
# Needs the WebSRT wasm staged in vendor/WebSRT/web/wasm/ before Vite bundles
# the receive worker — hence the build-websrt-wasm prerequisite.
build-ui: build-websrt-wasm
	cd frontend && npx vite build

# Build WebSRT wasm crates from vendor/WebSRT + stage where the submodule's
# worker expects them (vendor/WebSRT/web/wasm/). Required before build-ui
# bundles the receive worker. Idempotent.
build-websrt-wasm:
	bash build/build-websrt-wasm.sh

# Build the reference WebSRT web app (vendor/WebSRT/web) into its dist/.
# The server embeds dist/ at COMPILE time (ref_web.rs rust-embed) — after
# this target, rebuild cakemix-server for changes to show on :8201.
build-websrt-web:
	cd vendor/WebSRT/web && npx vite build

# Update the vendor/WebSRT submodule pin to the remote's current HEAD.
# The new pin is NOT auto-committed — commit the gitlink yourself.
bump-websrt:
	git submodule update --remote vendor/WebSRT
	@echo "Reminder: vendor/WebSRT pin changed — commit it: git add vendor/WebSRT && git commit"

# Full rebuild: WASM + worklet + UI.
# Order: build-web (mixer pkg) and build-ui (→ build-websrt-wasm, then Vite)
# complete first; build-worklet.js only needs the mixer pkg, so it runs last.
build-all: build-web build-ui
	node build/build-worklet.js
