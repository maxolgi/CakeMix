# AGENTS.md — WASM Audio Mixer

Behavioral guidelines + repo-specific guidance for any agent working on
this project. Read `IMPLEMENTATION_PLAN.md` for the milestone plan and
`FINDINGS.md` for the verified technical facts this project is built on.

---

## Generic guidelines

**Think before coding.** State assumptions explicitly; if uncertain, ask.
If multiple interpretations exist, present them — don't pick silently.
Push back when a simpler approach exists.

**Simplicity first.** Minimum code that solves the problem. No speculative
features, no abstractions for single-use code, no configurability that
wasn't requested. If 200 lines could be 50, rewrite.

**Surgical changes.** Touch only what the task requires. Match existing
style. Mention dead code you notice — don't delete it unless asked.
Remove only the orphans your own changes create.

**Goal-driven.** Transform tasks into verifiable goals ("write a test that
reproduces it, then make it pass"). For multi-step work, state a plan with
per-step verification before starting. Loop until verified.

---

## Project: standalone pro WASM audio mixer with WebSRT I/O

**One-line goal:** a professional digital audio mixer (up to 128 input
strips, full console: EQ, dynamics, buses, sends, scenes, automation)
compiled to `wasm32-unknown-unknown`, with audio arriving and departing
the browser over WebSRT (SRT over WebTransport).

**This is a standalone repo, not a SlopShady feature.** SlopShady
(`/home/flibb/SlopShady`) is **read-only reference** for the WebSRT I/O
glue — three files (see FINDINGS.md §6). Never modify SlopShady from here.

### Engine source: port `oximedia-mixer`

- The engine is `crates/oximedia-mixer/` from
  https://github.com/cool-japan/oximedia. It is real, deep (~45 modules,
  ~500KB Rust), and worth reusing. See FINDINGS.md §1 for the full module
  inventory.
- **Do NOT use `oximedia-wasm/src/mixer_wasm.rs`.** It is a ~250-line
  hand-rolled toy (gain + pan + sum) that does not touch the real engine.
  See FINDINGS.md §2. Write the real binding from zero.
- Source the engine via the cleanest path that compiles on wasm (crates.io
  dep, git dep, or fork-and-strip into this repo). Decide when wiring
  `Cargo.toml`.

### WASM target: single-threaded, no COOP/COEP

- Target `wasm32-unknown-unknown`. Real-time audio is single-threaded by
  design (every DAW, Web Audio's 128-frame render quantum) — no audio-thread
  parallelism is lost.
- **`rayon` is cfg-gated behind a `parallel` feature, OFF for wasm.** Only
  `parallel_mix.rs` and `offline_bounce.rs` use it (offline paths). Never
  pull in `wasm-bindgen-rayon`.
- **No `SharedArrayBuffer`, no cross-origin isolation (COOP/COEP).** PCM
  handoff uses transferred `MessagePort`, exactly as SlopShady's
  `stream-audio-worklet.js` does. Host apps must not need special headers.
- Resolve `scirs2-core` wasm status early (FINDINGS.md §1). Strip or stub
  non-wasm pieces if it blocks the build.

### The honesty rule (load-bearing)

oximedia has a documented history of modules that compile and pass tests
but do not actually perform the DSP (fake decoders were removed — see
FINDINGS.md §3). Therefore: **every DSP path from `oximedia-mixer` must
pass an end-to-end known-answer test before being trusted or built upon.**
M0's known-answer mix test is the project's first gate, not a formality.
If a ported module's output doesn't match expected DSP math, treat it as
"compiles-but-fake" until proven otherwise — file the finding, don't paper
over it.

### WebSRT boundary

The user **owns WebSRT** (https://github.com/maxolgi/WebSRT) and will
extend it. WebSRT extensions implied by this project (FINDINGS.md §4):
audio-only mux, MPTS (multi-program), multi-PID-per-program with per-PID
channel count. **Those edits happen in WebSRT, not in this repo.** This
repo consumes WebSRT as a submodule (or git dep) and bumps the pin when
WebSRT lands a needed change. If you conclude a fix must live in WebSRT,
stop, describe the exact edit (file, lines, diff) for the user to apply
there, then bump the pin here once it's merged.

### Transport architecture (decided)

- **MPTS** for dense/local sources (one SRT connection, multi-program TS,
  shared PCR → alignment is free within the stream).
- **Multiple WebSRT sessions** for distributed sources (different origins).
  Cross-session phase alignment is **protocol-level** (matched SRT
  `latency` + playout buffer), **not a mixer subsystem.** The mixer
  consumes each receiver's paced output. Do not design a master-clock or
  alignment-buffer subsystem — that was explicitly struck from the plan.
- **Per-PID channel count is flexible** (broadcaster-style): a program
  carries N audio elementary streams each tagged mono/stereo/arbitrary.
  16ch = 16×mono or 8×stereo, per the source.
- 128 input strips max.
- **The `--srt-port` ingest leg (SrtIngester, default 9001) is legacy
  direct-ingest compatibility — never use it for testing or demos.** All
  source ingest goes through the user's websrt-gateway (:9000, stream
  `audio`); CakeMix consumes over WebSRT via the drawer URL field. WebSRT
  paths are the point of this project; SRT shortcuts mask real workflow
  bugs (CORS, cert-hash, stream discovery).

### Verification

There is no CI yet for this repo. Establish one early (at minimum: a
known-answer DSP test that runs under `wasm-bindgen-test` in node, plus
`cargo check` on the wasm target). Every milestone ships with a test that
proves the milestone's claim.

### Frontend conventions

- **SolidJS** with TypeScript, built via Vite to static `web/` assets.
  Source lives in `frontend/`, Vite builds to `web/assets/`. The Rust
  server (rust-embed) serves the built output — no Vite at runtime.
  Build: `cd frontend && npx vite build` or `make build-ui`.
- The **reference WebSRT web app** (vendor/WebSRT/web — viewer, publisher,
  debug panels) is served unmodified on its own HTTPS port (`--web-port`,
  default 8201; `--no-tls` disables it). It is embedded at compile time:
  `make build-websrt-web` then rebuild cakemix-server. Never write a custom
  viewer page — the reference app IS the consumer.
- All CSS in `frontend/src/global.css`, no inline styles.
- Canvas-based meters via `requestAnimationFrame` — never update DOM
  elements at 60fps for meter bars. See `MeterCanvas.tsx`.
- EQ frequency response curves rendered via Canvas (biquad coefficient
  math ported from Eyevinn audio-mixer, AGPL-compatible).
- Every interactive element gets a tooltip.

### Commit posture

Committing completed work units in this repo is expected — you do not need
to ask first. Never commit in SlopShady (read-only reference). Never commit
in WebSRT from here (crosses the submodule boundary); bump the pin instead.
