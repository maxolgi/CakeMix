# AGENTS.md — WASM Audio Mixer

Behavioral guidelines + repo-specific guidance for any agent working on
this project. `README.md` is the living project reference (status,
architecture, contracts, performance numbers).

**Tracked docs policy:** the ONLY tracked root docs are `AGENTS.md` and
`README.md`. Do not create or commit plan/summary/notes `.md` files — put
milestone state in README's Status table and durable facts in README's
sections. (Historical planning files exist gitignored, local-only, for the
user's reference; never re-add them.)

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

## Project: standalone pro WASM audio mixer with QuicSRT I/O

**One-line goal:** a professional digital audio mixer (up to 128 input
strips, full console: EQ, dynamics, buses, sends, scenes, automation)
compiled to `wasm32-unknown-unknown`, with audio arriving and departing
the browser over QuicSRT (SRT over WebTransport).

**This is a standalone repo, not a SlopShady feature.** SlopShady
(a separate local checkout) is **read-only reference** for the browser
audio I/O glue — three files: `src/js/ui/streaming-input.ts` (decoded-AudioData tap),
`src/js/features/stream-audio-worklet.js` (960-frame Float32 → transferred
MessagePort), `src/js/features/stream-worker.js` (Opus→TS→SRT publish
worker). Never modify SlopShady from here.

### Engine source: port `oximedia-mixer`

- The engine is `crates/oximedia-mixer/` from
  https://github.com/cool-japan/oximedia. It is real, deep (~45 modules,
  ~500KB Rust), and worth reusing.
- **Do NOT use `oximedia-wasm/src/mixer_wasm.rs`.** It is a ~250-line
  hand-rolled toy (gain + pan + sum) that does not touch the real engine.
  Write the real binding from zero.
- The engine comes from the fork `maxolgi/oximedia` (7 commits past
  upstream 0.2.1: wasm build fixes + `process_mix_rt`, the real-time
  per-channel-input mixing API). Pinned via `[patch.crates-io]` at the
  pushed fork master; the fork source is at `~/oximedia` on this machine.

### WASM targets: mixer single-threaded, no SAB

- The **mixer** targets `wasm32-unknown-unknown`. Real-time audio is
  single-threaded by design (every DAW, Web Audio's 128-frame render
  quantum) — no audio-thread parallelism is lost. No
  `SharedArrayBuffer`: PCM handoff uses transferred `MessagePort`,
  exactly as SlopShady's `stream-audio-worklet.js` does.
- The **QuicSRT engine** (`vendor/QuicSRT-wasm`, C++/emscripten) uses
  pthreads → `SharedArrayBuffer`, so pages MUST be cross-origin isolated:
  cakemix-server sends `Cross-Origin-Opener-Policy: same-origin` +
  `Cross-Origin-Embedder-Policy: require-corp` on EVERY response.
  `crossOriginIsolated` needs a secure context — `--no-tls` plain HTTP
  only works on localhost; remote LAN clients need TLS mode.
- **`rayon` is cfg-gated behind a `parallel` feature, OFF for wasm** (done,
  fork `24a35975`). Only `parallel_mix.rs` uses it (offline path). Never
  pull in `wasm-bindgen-rayon`.
- `scirs2-core` was a phantom dep and is removed in the fork — resolved.
  Per-channel input is native in the fork engine
  (`ProcessingEngine::process_mix_rt`); the binding stages per-strip audio
  and drives nine engine instances (1 main + 8 buses) per block.

### The honesty rule (load-bearing)

oximedia has a documented history of modules that compile and pass tests
but do not actually perform the DSP (fake decoders were removed upstream).
Therefore: **every DSP path from `oximedia-mixer` must
pass an end-to-end known-answer test before being trusted or built upon.**
M0's known-answer mix test is the project's first gate, not a formality.
If a ported module's output doesn't match expected DSP math, treat it as
"compiles-but-fake" until proven otherwise — file the finding, don't paper
over it.

The gxfx insert FX follow the same rule: the guitarix C++ DSP lives under
`crates/mixer-wasm/gxfx/`, compiled freestanding by `build.rs` (plain
`clang++` + `ar` — prerequisite; sources from the pinned
`vendor/guitarix` submodule: `git submodule update --init
vendor/guitarix`), and every newly ported effect must pass a known-answer
honesty test before its factory slot is enabled — distortion (id 2,
bossds1, landed 2026-10-20) is the applied example of that rule: it
stayed rejected by `set_slot` until its TU, freestanding object audit
and honesty test all landed. Because delay/echo/reverb allocate MB-scale tapes
at create, the engine caps concurrently created memory-class instances
at 96 (~384 MiB worst case, 1.5× the 64 pairs so every pair can run a
delay with headroom), while the livelooper (~128 MiB of tapes per
enabled slot) rides its own separate cap of 4 created loopers (~512 MiB
worst case); DSP create/activate is eager at message time,
never inside process().

### QuicSRT boundary

The user **owns the QuicSRT repos** (https://github.com/maxolgi/libquicsrt,
plus QuicSRT-wasm and QuicSRT-gateway) and will extend them. The demux/mux
surface is SMPTE 302M PCM ("BSSD") — the handoff contract is in README.md
(WorkerCmd/WorkerMsg contracts unchanged; byte parity pinned upstream by
QuicSRT-wasm's demux/muxer parity gates). **Any future QuicSRT edits happen
in the QuicSRT repos, not in this repo.** This repo consumes QuicSRT-wasm
as the `vendor/QuicSRT-wasm` submodule (which itself carries libquicsrt's
receiver-only core + the srt fork as its own deps) and bumps the pin when a
needed change lands: `git submodule update --remote vendor/QuicSRT-wasm`,
then commit the gitlink. If you conclude a fix must live in QuicSRT, stop,
describe the exact edit (repo, file, diff) for the user to apply there,
then bump the pin here once it's merged.

### Transport architecture (decided)

- **MPTS** for dense/local sources (one SRT connection, multi-program TS,
  shared PCR → alignment is free within the stream).
- **Multiple QuicSRT sessions** for distributed sources (different origins).
  Cross-session phase alignment is **protocol-level** (matched SRT
  `latency` + playout buffer), **not a mixer subsystem.** The mixer
  consumes each receiver's paced output. Do not design a master-clock or
  alignment-buffer subsystem — that was explicitly struck from the plan.
- **Per-PID channel count is flexible** (broadcaster-style): a program
  carries N audio elementary streams each tagged mono/stereo/arbitrary.
  16ch = 16×mono or 8×stereo, per the source.
- 128 input strips max.
- **There is no direct-ingest leg anymore** — SrtIngester was removed with
  the embedded gateway. ALL source ingest goes through the user's external
  QuicSRT-gateway (web/player HTTPS :8443, WebTransport ingest :4435, SRT
  ingest :9102, health :9292 — supervised on the dev machine);
  CakeMix subscribes over QuicSRT via the drawer URL field (default
  `https://127.0.0.1:8443`). QuicSRT paths are the point of this project;
  bypassing the gateway masks real workflow bugs (CORS, cert-hash, stream
  discovery).

### Verification

CI is GitHub Actions (`.github/workflows/ci.yml`): fmt + clippy + native
tests + the release-mode glitch_sim RT-budget gate + the wasm test suites
+ a 3-OS build matrix that produces release binaries. Jobs that need
`web/` also build the emcc QuicSRT engine first (CI installs emsdk 6.0.8;
`make build-quicsrt-engine`, staged into `web/build/` by `build-ui`) —
the gates themselves are otherwise unchanged. Every milestone
ships with a test that proves the milestone's claim; keep the suites
green locally (`make test-all`) before pushing.

### Frontend conventions

- **SolidJS** with TypeScript, built via Vite to static `web/` assets.
  Source lives in `frontend/` (the AudioWorklet processor + polyfill in
  `frontend/src/worklet/`; the QuicSRT client in `frontend/src/quicsrt/`).
  **`web/` is fully generated and gitignored — never commit it.** The
  Rust server (rust-embed) serves the built output at compile time — no
  Vite at runtime. Build: `make build-ui` (which first builds + stages
  the emcc QuicSRT engine at `web/build/`) + `make build-worklet`, or
  just `make serve` (builds everything first).
- The **reference web app** is the QuicSRT-gateway's own embedded
  player/publisher, served by the gateway at its web origin
  (https://127.0.0.1:8443) — NOT by CakeMix (cakemix-server embeds only
  its own UI + the wasm pkg). Never write a custom viewer page — the
  gateway's player IS the reference consumer.
- All CSS in `frontend/src/global.css`, no inline styles.
- Canvas-based meters via `requestAnimationFrame` — never update DOM
  elements at 60fps for meter bars. See `MeterCanvas.tsx`.
- EQ frequency response curves rendered via Canvas (biquad coefficient
  math ported from Eyevinn audio-mixer, AGPL-compatible).
- Every interactive element gets a tooltip.

### Commit posture

Committing completed work units in this repo is expected — you do not need
to ask first. Never commit in SlopShady (read-only reference). Never commit
in the QuicSRT repos from here (crosses the submodule boundary); bump the
pin instead.
