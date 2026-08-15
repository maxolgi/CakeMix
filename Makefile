# CakeMix — WASM Audio Mixer
# All commands. No push — that's the user's job.

RUST_TARGET := wasm32-unknown-unknown
CRATE_DIR := crates/mixer-wasm

.PHONY: build-wasm build-web build-node test-native test-wasm test-all check clean serve build-websrt-wasm bump-websrt

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
serve: build-web build-ui
	cargo run -p cakemix-server -- --no-tls --port 8200

# Same but with HTTPS (auto-generates self-signed cert)
serve-tls: build-web build-ui
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

# Build WebSRT wasm crates from vendor/WebSRT + stage where the submodule's
# worker expects them (vendor/WebSRT/web/wasm/). Required before build-ui
# bundles the receive worker. Idempotent.
build-websrt-wasm:
	bash build/build-websrt-wasm.sh

# Update the vendor/WebSRT submodule pin to the remote's current HEAD.
# The new pin is NOT auto-committed — commit the gitlink yourself.
bump-websrt:
	git submodule update --remote vendor/WebSRT
	@echo "Reminder: vendor/WebSRT pin changed — commit it: git add vendor/WebSRT && git commit"

# Full rebuild: WASM + worklet + UI
build-all: build-web build-websrt-wasm
	cd frontend && npx vite build
	node build/build-worklet.js
