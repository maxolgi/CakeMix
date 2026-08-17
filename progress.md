# CakeMix — Progress & Project State

Last updated: 2026-08-16 (engine-driven mixing / bus console phase)

---

## Executive Summary

CakeMix is a standalone professional WASM audio mixer built on the real
`oximedia-mixer` DSP engine, compiled to `wasm32-unknown-unknown`.
Audio is PCM-only (no codecs), transported over WebSRT/SRT inside
MPEG2-TS using SMPTE 302M (s302m) encapsulation.

**M0 is complete.** The mixer compiles to WASM, runs real DSP, and passes
92 tests (57 native + 35 WASM). The web demo server runs the WASM mixer
inside an AudioWorklet.

**M1 is complete** — PCM is received and published over direct worker↔worklet
MessagePorts (zero main-thread hops). WebSRT has all PCM/SMPTE 302M changes
(commit a4181b1+).

**Engine-driven mixing shipped (M5 core):** the binding now drives nine
`ProcessingEngine` instances per block through the fork's new
`process_mix_rt` (per-channel inputs, zero steady-state allocation), with an
8-bus console (MAIN / FEEDS-MAIN routing toggles, per-bus publish, live GR
meters). Scenes/automation remain future work.

---

## Milestone Status

| Milestone | Status | Notes |
|-----------|--------|-------|
| M0 — WASM build + DSP tests | ✅ Done | 57 native + 35 WASM tests |
| M1 — PCM end-to-end via WebSRT | ✅ Done | Direct worker↔worklet MessagePort PCM path |
| M2 — Multi-PID (8 stereo / 16 mono) | ⏳ Blocked | Needs M1 + WebSRT MPTS |
| M3 — Multi-session sum | ⏳ Blocked | Needs M1 |
| M4 — Pro DSP (EQ, dynamics, metering) | 🔨 In Progress | EQ/dynamics/gate/expander wired in staging; GR meters live |
| M5 — Bus architecture + scenes | 🔨 Partial | 8 summing buses + routing toggles + bus publish shipped (engine-driven); scenes pending |
| M6 — Talkback, cue, MIDI | 🔜 Future | |
| M7 — Control surfaces + AFV | 🔜 Future | |
| M8 — Reliability + multi-operator | 🔜 Future | |

---

## What's Built

### WASM Mixer Binding (`crates/mixer-wasm/`)

**57 native tests, 35 WASM tests — all pass.**

The `MixerWasm` struct owns **one 128-strip main `AudioMixer` + eight
16-channel bus `AudioMixers`** (the `AudioMixer`s are engine containers —
`AudioMixer::process()` is bypassed; all summing happens in their
`ProcessingEngine`s). Per block (`process_block`, pure Rust — the JS `process()`
is a thin copy-out wrapper):

1. **FIFO drain** — elastic per-channel input FIFOs → `block_inputs`
   (raw mono rows; starvation = zeros, drift-reconciled).
2. **STAGING** — per strip on the raw source: gate → expander → compressor →
   6-band EQ, in place. Bus slots stage the **RAW** row of their assigned
   source (parallel tap — the source's mute/fader/EQ/dynamics never affect the
   bus path; pinned by `tests/bus_parallel_test.rs`).
3. **Metering + direct-out tap** — per-channel peak/RMS (post-staging,
   pre-fader) and the mono post-fader/pre-pan channel tap.
4. **NINE `process_mix_rt` calls** — main engine mixes strips 0-127 with
   `main_assign`; bus engine i mixes its 16 slots. Call lists are fixed
   **stack** arrays (`[_;128]` + `[[_;16];8]` of `Copy` tuples — a struct
   field can't hold `&[f32]` into another field). Zero steady-state
   allocation (alloc_test green with buses active).
5. **Bus tail** — bus gain/mute → `bus_pub` (published via
   `take_bus_output`); `feeds_main && !muted` accumulates into master.
6. **Master** — master gain → limiter (L/R) → interleave → meter.

Key methods (full list in `lib.rs`; aux path `add_bus`/`set_aux_send`/
`remove_aux_send` deleted — superseded by the bus console):

| Method | Purpose |
|--------|---------|
| `new(sample_rate, buffer_size, max_channels)` | Constructor (max_channels advisory — always full 256-strip console) |
| `set_channel_input(ch, Float32Array)` | Mono planar input |
| `set_channel_input_interleaved(ch_start, Float32Array, num_channels)` | Interleaved input (s302m PCM) |
| `feed_pcm(pid, Float32Array)` | Convenience: looks up PID mapping, de-interleaves |
| `map_pid(pid, ch_start, channel_count)` / `unmap_pid(pid)` | PID→channel mapping (idempotent) |
| `subscribe_pid(pid)` / `unsubscribe_pid(pid)` | Per-PID mute control |
| `set_channel_gain/pan/mute/solo(ch, …)` | Strip controls (looked up via the strip's owning engine) |
| `set_eq_band_gain/freq/q`, `set_eq_bypass` | 6-band EQ per strip |
| `enable/disable_compressor|gate|expander`, `set_comp/gate/expander_param` | Per-strip dynamics (staging layer) |
| `set_channel_main_assign(ch, on)` | MAIN toggle (default on): strip in main engine's call list; off = reaches master only via bus slots |
| `set_bus_source(bus, slot, ch)` / `clear_bus_source` | Assign a bus slot to tap input channel `ch` (parallel raw tap) |
| `set_bus_feeds_main(bus, on)` | FEEDS MAIN toggle (default on): off = independent bus, still published by `take_bus_output` |
| `set_bus_gain(bus, g)` / `set_bus_mute(bus, m)` | Bus master controls |
| `take_bus_output(bus)` | Drain the bus's own post-gain stereo output (publish path) |
| `set_channel_tap(n)` / `take_channel_tap()` | N-channel direct-out tap (WebSRT Nch publish) |
| `channel_peak_db/rms_db`, `channel_comp_gr_db`, `channel_meters_json` | Per-strip meters incl. live compressor GR ("gr" field) |
| `bus_peak_db/rms_db`, `bus_meters_json` | Per-bus meters |
| `process(block_size)` | One block → interleaved stereo Float32Array |
| `master_peak_db_l/r()`, `master_rms_db_l/r()`, `master_clipping()` | Master meters |

### DSP Modules (M4 progress)

All verified with known-answer honesty tests per the AGENTS.md honesty rule.

| Module | Real DSP? | Tests | Wired into chain? |
|--------|-----------|-------|-------------------|
| Mixer (gain/pan/sum) | ✅ | engine parity + alloc tests in fork; 9 native + 5 WASM (binding) | ✅ **engine-driven** — 9 `process_mix_rt` calls per block |
| ParametricEq (RBJ biquads) | ✅ | 5 honesty + 3 integration + 4 six-band | ✅ staging layer (`EqEffect` adapter), pre-engine |
| Compressor | ✅ | 16 honesty + 3 integration | ✅ staging layer, per strip; GR readback via fork's `last_gain_reduction_db()` |
| Expander | ✅ | (in honesty suite) | ✅ staging layer (`enable_expander()`), gate → expander → comp order |
| Gate | ✅ | (in honesty suite) | ✅ staging layer (`enable_gate()`) |
| OversampledLimiter | ✅ | 3 tests | ✅ on master bus in `process_block()` (always on) |
| Metering (peak/RMS/clip/GR) | ✅ | 5 tests + bus/tap suites | ✅ per-strip, per-bus, master; COMP GR live |

**Bugs found in the engine:**
- `linear_to_db(0.0)` returned `-f32::INFINITY`, poisoning envelope
  followers at zero-crossings. Fixed on the old fork line (b88cf333, floor
  at −200 dB) — **but that fix was dropped in the 0.2.1 merge** (see Known
  Issues).
- `std::time::SystemTime` and `Instant` don't exist on
  `wasm32-unknown-unknown`. Fixed: cfg-gated behind `not(target_arch = "wasm32")`.

### Web Server (`crates/cakemix-server/`)

Rust binary crate using axum + rust-embed + axum-server (same pattern as
SlopShady). Serves the web demo over HTTP or HTTPS with auto-generated
self-signed cert. Auto-opens browser on start.

```bash
make serve     # HTTP on :8200
make serve-tls # HTTPS on :8200
```

### Web UI (`frontend/` → generated `web/`)

- SolidJS + TypeScript app built via Vite to static `web/` (gitignored,
  embedded into `cakemix-server` at compile time via rust-embed). Components:
  `ChannelStrip` (fader/pan/mute/solo) + `ChannelDetailPanel` (EQ + dynamics
  knobs, MAIN toggle, live GR meter), `BusManager`/`BusMasterStrip` (bus
  slots, FEEDS MAIN, bus gain/mute), `MasterStrip`, `MeterCanvas`
  (rAF-driven meters), `WebSRTPanel` (source drawer + publish SOURCE
  select: master | bus 1-8).
- Worklet processor source: `frontend/src/worklet/worklet-template.js`
  (wasm-bindgen glue + polyfills inlined at build time by
  `node build/build-worklet.js`; TextEncoder/TextDecoder polyfill for
  AudioWorkletGlobalScope).

**Build step:** `make build-ui` + `make build-worklet` (or `make serve`,
which builds everything first).

---

## Architecture Decisions

### PCM-only (no codecs)
No Opus, Vorbis, AAC, or WebCodecs. Raw linear PCM (Float32) end-to-end.
Transported in MPEG2-TS using SMPTE 302M (s302m).

### Per-channel input (decision b — revised)
The engine's `process()` feeds the same input to every channel, and
`process_mix` takes one shared input slice. ~~The binding resolved this by
calling `engine.process_mix()` once per channel with that channel's own
input, then summing master outputs.~~ **Now native in the fork API:**
`ProcessingEngine::process_mix_rt` (fork `0a517fb0`) takes per-channel
`(ChannelId, ChannelProcessParams, &[f32])` tuples, sums into caller-provided
stereo buffers, and allocates nothing at steady state. The binding's staging
layer resolves which raw row each strip contributes and builds the call
lists; **the engine performs all summing** (nine engines: 1 main + 8 buses).
See `ENGINE_API.md` §6.

### AudioWorklet integration
The WASM mixer runs inside an AudioWorkletProcessor for real-time audio.
Three polyfills are required for AudioWorkletGlobalScope:
1. **TextDecoder/TextEncoder** — not available in worklet scope
2. **crypto.getRandomValues** — not available in worklet scope
3. **No dynamic import()** — disallowed in worklet; glue is inlined

The main thread compiles the `WebAssembly.Module`, sends raw bytes to the
worklet via MessagePort, and the worklet compiles + instantiates via
`initSync()`.

### Transport contract (from `audioplan.md`)
- PCM arrives as **Float32 interleaved** per PID (i32→f32 done in demuxer)
- **48 kHz fixed** — resampling is the mixer's concern
- **ptsMs from PES PTS** — ffmpeg populates correctly for s302m
- **PID map can change mid-stream** — handled idempotently
- Stream type: `0x06` + registration descriptor "CUES" (SMPTE 302M)

---

## Fork Changes (`maxolgi/oximedia`)

Base: `aa9e68af` (upstream 0.2.1, `5263510`, merged in). Six commits beyond
upstream, **NOT yet pushed** — the CakeMix root `Cargo.toml` temporarily
path-patches `oximedia-{mixer,core,audio}` to `~/oximedia`; **revert to the
git branch patch after pushing** (`{ git = "https://github.com/maxolgi/oximedia", branch = "master" }`).

Wasm build fixes (below the 0.2.1 merge):

1. **`24a35975`** — Make rayon optional behind `parallel` feature.
   `parallel_mix` module + `process_parallel()` cfg-gated. Removed
   phantom `scirs2-core` dependency.
2. **`b2b065af`** — Patch `std::time` usage for `wasm32-unknown-unknown`
   (session.rs, offline_bounce.rs).
3. **`0e900b20`** — Fix scene_recall.rs `Scene::new` for wasm32.

Engine API additions (on top of the merge):

4. **`0a517fb0`** — **`process_mix_rt`**: real-time per-channel-input
   variant of `process_mix` — per-channel input slices, sums into
   caller-provided out_l/out_r, engine-pre-allocated scratch, zero
   steady-state allocation, step-identical DSP. Engine tests: distinct-input
   known answer, exact parity with `process_mix` (single-channel and a
   buses/sends/VCA scenario), muted/fader, zero-pad; plus integration test
   `process_mix_rt_alloc.rs` (counting allocator, 128ch × 200 blocks = 0 bytes).
5. **`14e623c8`** — `ChannelId::nil()` + `Copy` on `ChannelProcessParams`
   (enables fixed-size stack call lists in the binding).
6. **`3283cb3f`** — `Compressor::last_gain_reduction_db()` readback (GR meters).
7. **`9717f878`** — re-applied the `linear_to_db` −200 dB floor (the 0.2.1
   merge had dropped `b88cf333`): −∞ permanently poisoned the one-pole
   envelopes of Compressor/Expander/Gate on any exactly-zero sample. Pinned
   by an envelope-survives-silence regression test in the fork and by the
   (now positive) zero-crossing honesty tests in CakeMix.

> The pre-merge commit `b88cf333` (linear_to_db zero-crossing floor) did
> **not** survive the 0.2.1 merge — re-applied as `9717f878` above.

Fork test totals: **993 lib tests + 1 alloc integration test**, all passing.

All changes are additive: existing behavior is preserved when the
`parallel` feature is off and the target is not wasm32.

## oxifft

From crates.io: 0.4.2, `std`-only on wasm32. The oximedia fork
(`aa9e68af`, upstream 0.2.1 base) requests it with
`default-features = false, features = ["std"]` and gates oxifft's
`threading` (rayon) feature to non-wasm targets. No fork, patch,
or `vendor/` copy is needed.

---

## Test Inventory

Native (`cargo test`) and WASM (`wasm-pack test --node`, part of
`make test-wasm`):

| File | Tests | What it covers |
|------|-------|----------------|
| `tests/native_dsp.rs` | 9 native | Pan laws (Linear/-3dB/-6dB), gain, phase, mute, sum |
| `tests/interleave_test.rs` | 3 native | De-interleave + PID mapping idempotency |
| `tests/eq_honesty.rs` | 5 native | ParametricEq boost/cut/flat/highpass/bypass |
| `tests/eq_integration.rs` | 3 native | EQ through the real mix path |
| `tests/eq_six_band.rs` | 4 native | 6-band Fairlight preset |
| `tests/dynamics_honesty.rs` | 16 native | Comp/expander/gate/limiter gain math |
| `tests/dynamics_integration.rs` | 3 native | Comp/gate through the real mix path |
| `tests/limiter_test.rs` | 3 native | Oversampled limiter prevents overs |
| `tests/metering_test.rs` | 5 native | Peak/RMS/clip readings |
| `tests/alloc_test.rs` | 1 native | Zero allocation per block steady-state (full console, buses active) |
| `tests/drift_test.rs` | 4 native | Elastic FIFO drift reconciliation |
| `tests/glitch_sim.rs` | 1 native (+1 ignored) | Realistic load within CPU budget; full-load sim is `--ignored` (**Phase C numbers pending** — CPU ceiling measurement in progress) |
| `tests/known_answer.rs` | 5 WASM | Core mix known answers via JS interop |
| `tests/channel_params_test.rs` | 9 WASM | Param plumbing (gain/pan/mute/phase/pan-law) |
| `tests/channel_tap.rs` | 10 WASM | Direct-out tap (Nch, post-fader pre-pan, drain) |
| `tests/pcm_fifo_test.rs` | 3 WASM | Elastic FIFO behavior |
| `tests/bus_parallel_test.rs` | 4 WASM | Bus slots tap RAW source — parallel semantics |
| `tests/bus_output_test.rs` | 4 WASM | MAIN-off audible via bus; FEEDS-MAIN-off still published; bus mute = silence; COMP GR under compression |
| `tests/run_tests.mjs` | 13 (node) | JS-level end-to-end runner (sum, silence, mute, gain, interleaved, PID mapping, subscribe, tap, …) |

**Total: 57 native + 35 WASM = 92** (plus 13 JS-runner tests and 1 ignored
full-load sim). Engine fork: 993 lib + 1 alloc integration.

---

## Known Issues

### Web demo: WORKING ✅
The WASM mixer runs successfully inside an AudioWorklet. Verified in
headless Chrome via CDP: MixerWasm constructs, wasm-ready posted, page
shows "WASM mixer ready" / "WASM ACTIVE". Start button enables. Gain,
pan, and mute controls all work. Three polyfills for
AudioWorkletGlobalScope are in place (TextDecoder/TextEncoder,
crypto.getRandomValues, inlined glue with no dynamic import).

### M4 improvements: DONE ✅
- **Allocation-free RT path**: all binding buffers pre-allocated; the engine
  call lists live on the stack; the engines run `process_mix_rt` (engine
  scratch, zero steady-state alloc — pinned by the fork's
  `process_mix_rt_alloc.rs` and CakeMix's `alloc_test.rs` with buses active).
  One unavoidable JS FFI copy per block.
- **6-band EQ preset**: Fairlight-matching (HPF/Low/Lo-Mid/Mid/Hi-Mid/High).
  Added to EqEffect + 4 honesty tests.
- **Metering UI**: Worklet reports peak/RMS/clip/GR every 10 blocks. Main
  thread renders green/yellow/red meter bars in the UI.
- **Live mode**: Worklet accepts external PCM via feed_pcm() for WebSRT.
- **CI pipeline**: GitHub Actions (native tests + wasm build + node tests).

### Engine allocation (resolved)
~~`process_mix` allocates 4 Vecs/channel/block internally; `buffer_pool.rs`
needs wiring.~~ Superseded: the binding no longer calls `process_mix` —
`process_mix_rt` (fork `0a517fb0`) pre-allocates all per-channel and per-bus
scratch on the engine. `buffer_pool.rs` remains unused on the RT path.

### `linear_to_db(0.0)` regression (engine, post-0.2.1-merge) — FIXED
The old fork fix (b88cf333, −200 dB floor) was dropped when upstream 0.2.1
was merged in; `dynamics.rs` again returned `−inf` and an upstream test
pinned that. The one-pole envelope followers in `Compressor`/`Expander`/
`Gate` poison to `−inf` if a processed sample is **exactly** 0.0
(silence) — permanently (−∞ + finite = −∞), so a silent-then-live strip
would brick its dynamics. **Fixed in fork `9717f878`** (floor at −200 dB,
envelope-survives-silence regression test added; CakeMix's zero-crossing
honesty tests flipped to pin the fixed behavior).

### Phase C — CPU ceiling (glitch_sim drives the real `process_block`)
Measured (native release, 48 kHz / 128-frame blocks, 2667 µs budget):

| Load | avg | p99 | max | verdict |
|---|---|---|---|---|
| Realistic (60 strips + 2 buses × 4 slots, 4 comps, tap 16) | 253 µs | **275 µs** | 555 µs | 9.7× headroom (asserted) |
| Full console (128 strips + 8×16 slots, comp+gate on 32, tap 16, 2 buses off-main) | 1074 µs | **1161 µs** | 4186 µs¹ | 2.3× headroom (`--ignored`) |

¹ one OS-jitter outlier; p99 is the honest number.

Full load = ~3.8 µs/strip/block (staging + 6-band EQ dominates; the nine
engine sums add little). Cost scales ~linearly — 256 strips fit at ~44% of
budget; 512 would not. Zero starvation / zero NaN / bounded FIFO in both,
including a 300 ppm-fast full-load pass (depth ≤ 1920, no drift
accumulation). Noted, not asserted: limiter overshoot +0.6 dBFS at full
load (8876 samples > 0 dBFS against the −0.3 dB ceiling) — `limiter_test`
pins steady-state limiting; transient inter-sample overs at extreme sum
loads are a separate question. `realistic_load_within_budget` is the CI
gate; `full_load_256_strips_ceiling` runs via `--ignored --nocapture`.

### Registration ID
Correct value is "BSSD" (0x42535344), per ffmpeg s302m muxer.
Not "CUES" as initially proposed.

---

## What's Blocked on WebSRT

M1 (PCM end-to-end) requires changes in WebSRT's ts-muxer-wasm,
documented in `WEBSRT_CHANGES.md`:
1. LPCM stream type + "CUES" registration descriptor
2. `setAudioCodec("s302m")` setter
3. `push_audio` without Opus header for PCM
4. Audio-only PAT/PMT emission (no video keyframe dependency)
5. Configurable PCR PID for audio-only streams

The demuxer side (`mpeg2ts-wasm`) needs SMPTE 302M recognition + AES3
unwrap, per `audioplan.md` Phase 0.

The mixer binding is ready: `feed_pcm(pid, data)` implements the
`PcmPacket` handoff contract from `audioplan.md`.

---

## Repository

- **Root:** repo root (this directory)
- **Commits:** 118 local commits on `master`
- **Remote:** `git@github.com:maxolgi/CakeMix.git`
- **Fork:** `maxolgi/oximedia` — 6 commits beyond upstream 0.2.1 (3 wasm
  fixes below the `aa9e68af` merge, 3 API additions above it) — **not pushed**;
  root `Cargo.toml` carries the temporary `~/oximedia` path patch
- **No push policy:** The user handles all git push operations.

---

## Build & Run

```bash
# Build WASM for web target
make build-web

# Run all native tests (no browser needed)
make test-native          # 57 tests

# Run WASM tests in node (wasm-bindgen suite + JS runner)
make test-wasm            # 35 + 13 tests

# Start the web demo
make serve                # http://localhost:8200

# Rebuild the worklet (after wasm changes)
node build/build-worklet.js
```
