#!/usr/bin/env bash
# build-websrt-wasm.sh — build the WebSRT browser WASM crates from the
# vendor/WebSRT submodule and stage them where the submodule's web/src/worker.ts
# expects them (web/wasm/<crate>/, gitignored inside the submodule).
#
# Mirrors vendor/WebSRT/build.sh's `wasm` command but:
#   - strips inherited rustc wrapper / RUSTFLAGS env vars (our CI exports
#     RUSTC_WORKSPACE_WRAPPER=clippy-driver + -D warnings; wasm-pack's inner
#     cargo build would otherwise run clippy on the vendored crates and fail
#     on lints we cannot fix here — same pattern as SlopShady's build.rs), and
#   - never modifies tracked files in the submodule (only creates untracked
#     build outputs: crates/<crate>/pkg/ and web/wasm/).
#
# Idempotent: safe to re-run. Run this before `make build-ui` — the Vite build
# bundles the worker from the submodule source and needs web/wasm/ to exist.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUBMODULE_DIR="$REPO_ROOT/vendor/WebSRT"

CRATES=(srt-wasm mpeg2ts-wasm ts-muxer-wasm)

fail() {
    printf 'ERROR: %s\n' "$*" >&2
    exit 1
}

command -v wasm-pack >/dev/null 2>&1 \
    || fail "wasm-pack not found — install with 'cargo install wasm-pack' and add the target: rustup target add wasm32-unknown-unknown"

[ -f "$SUBMODULE_DIR/.git" ] || [ -d "$SUBMODULE_DIR/.git" ] \
    || fail "vendor/WebSRT submodule not initialized — run: git submodule update --init vendor/WebSRT"

[ -d "$SUBMODULE_DIR/web/src" ] \
    || fail "vendor/WebSRT/web/src missing — is the submodule checked out at the pinned revision?"

# Env vars that leak from our cargo/clippy CI into wasm-pack's inner cargo
# build (see header comment). Unset for every wasm-pack invocation below.
unset_env() {
    unset RUSTC_WRAPPER \
          RUSTC_WORKSPACE_WRAPPER \
          CARGO_BUILD_RUSTC_WRAPPER \
          RUSTFLAGS \
          CARGO_ENCODED_RUSTFLAGS \
          CARGO_BUILD_RUSTFLAGS
}

for crate in "${CRATES[@]}"; do
    crate_dir="$SUBMODULE_DIR/crates/$crate"
    [ -d "$crate_dir" ] || fail "missing crate dir: $crate_dir"

    printf '==> wasm-pack build %s (--target web --release)\n' "$crate"
    if ! (unset_env; cd "$crate_dir" && wasm-pack build --target web --release); then
        fail "wasm-pack build failed for $crate"
    fi

    [ -f "$crate_dir/pkg/${crate//-/_}_bg.wasm" ] \
        || fail "expected $crate/pkg/${crate//-/_}_bg.wasm not found after build"

    mkdir -p "$SUBMODULE_DIR/web/wasm/$crate"
    cp -f "$crate_dir/pkg/"* "$SUBMODULE_DIR/web/wasm/$crate/"
    printf '    staged -> vendor/WebSRT/web/wasm/%s\n' "$crate"
done

printf '==> done: %s staged in vendor/WebSRT/web/wasm/\n' "${CRATES[*]}"
