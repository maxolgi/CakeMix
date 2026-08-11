# CakeMix — WASM Audio Mixer
# All commands. No push — that's the user's job.

RUST_TARGET := wasm32-unknown-unknown
CRATE_DIR := crates/mixer-wasm

.PHONY: build-wasm build-web build-node test-native test-wasm test-all check clean serve

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

# Build WASM pkg + start the server (http://localhost:8200)
serve: build-web
	cargo run -p cakemix-server -- --no-tls --port 8200

# Same but with HTTPS (auto-generates self-signed cert)
serve-tls: build-web
	cargo run -p cakemix-server -- --port 8200


# Full CI check (matches .github/workflows/ci.yml)
ci: test-native test-wasm build-web
	@echo "✓ All CI checks pass"

# Clippy lints
clippy:
	cargo clippy --all-targets -- -D warnings

# Format check
fmt-check:
	cargo fmt --all -- --check


# Build SolidJS frontend → web/ (one-time, no runtime dependency)
build-ui:
	cd frontend && npx vite build

# Full rebuild: WASM + worklet + UI
build-all: build-web
	cd frontend && npx vite build
	node build/build-worklet.js
