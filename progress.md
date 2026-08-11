# CakeMix — Progress & Project State

Last updated: Session 019fee59

---

## Executive Summary

CakeMix is a standalone professional WASM audio mixer built on the real
`oximedia-mixer` DSP engine, compiled to `wasm32-unknown-unknown`.
Audio is PCM-only (no codecs), transported over WebSRT/SRT inside
MPEG2-TS using SMPTE 302M (s302m) encapsulation.

**M0 is complete.** The mixer compiles to WASM, runs real DSP, and passes
56 known-answer tests (47 native + 9 WASM). The web demo server runs the
WASM mixer inside an AudioWorklet with a 4-channel tone generator.

**M1 is unblocked** — WebSRT has implemented all PCM/SMPTE 302M changes (commit a4181b1).
Integration is browser-only (worker PCM messages → mixer worklet).

---

## Milestone Status

| Milestone | Status | Notes |
|-----------|--------|-------|
| M0 — WASM build + DSP tests | ✅ Done | 47 native + 9 WASM tests |
| M1 — PCM end-to-end via WebSRT | ✅ Unblocked | WebSRT has all PCM changes (a4181b1) |
| M2 — Multi-PID (8 stereo / 16 mono) | ⏳ Blocked | Needs M1 + WebSRT MPTS |
| M3 — Multi-session sum | ⏳ Blocked | Needs M1 |
| M4 — Pro DSP (EQ, dynamics, metering) | 🔨 In Progress | Adapters + honesty tests done |
| M5 — Bus architecture + scenes | 🔜 Future | |
| M6 — Talkback, cue, MIDI | 🔜 Future | |
| M7 — Control surfaces + AFV | 🔜 Future | |
| M8 — Reliability + multi-operator | 🔜 Future | |

---

## What's Built

### WASM Mixer Binding (`crates/mixer-wasm/`)

**51 native tests, 9 WASM tests — all pass.**

The `MixerWasm` struct wraps `oximedia_mixer::AudioMixer` and exposes
a JS-friendly API via `wasm-bindgen`. Key methods:

| Method | Purpose |
|--------|---------|
| `new(sample_rate, buffer_size, max_channels)` | Constructor |
| `set_channel_input(ch, Float32Array)` | Mono planar input |
| `set_channel_input_interleaved(ch_start, Float32Array, num_channels)` | Interleaved input (s302m PCM) |
| `feed_pcm(pid, Float32Array)` | Convenience: looks up PID mapping, de-interleaves |
| `map_pid(pid, ch_start, channel_count)` | PID→channel mapping (idempotent) |
| `unmap_pid(pid)` | Remove PID mapping |
| `subscribe_pid(pid)` / `unsubscribe_pid(pid)` | Per-PID mute control |
| `set_channel_gain(ch, gain)` | Per-channel gain (0.0–2.0) |
| `set_channel_pan(ch, pan)` | Per-channel pan (-1.0–1.0) |
| `set_channel_mute(ch, muted)` | Mute/unmute |
| `process(block_size)` | Process one block → interleaved stereo Float32Array |
| `master_peak_db_l/r()`, `master_rms_db_l/r()` | Meter readings |
| `master_clipping()` | Clip detection |

### DSP Modules (M4 progress)

All verified with known-answer honesty tests per the AGENTS.md honesty rule.

| Module | Real DSP? | Tests | Wired into chain? |
|--------|-----------|-------|-------------------|
| Mixer (gain/pan/sum) | ✅ | 9 native + 9 WASM | ✅ |
| ParametricEq (RBJ biquads) | ✅ | 5 honesty + 3 integration | ✅ via `EqEffect` adapter |
| Compressor | ✅ | 16 honesty + 3 integration | ✅ via `CompressorEffect` adapter |
| Expander | ✅ | (in honesty suite) | ✅ via `ExpanderEffect` adapter |
| Gate | ✅ | (in honesty suite) | ✅ via `GateEffect` adapter |
| OversampledLimiter | ✅ | 3 tests | ✅ in `process()` when `limiter_enabled` |
| Metering (peak/RMS/clip) | ✅ | 5 tests | ✅ in MixerWasm binding |

**Bugs found and fixed in the engine:**
- `linear_to_db(0.0)` returned `-f32::INFINITY`, poisoning envelope
  followers at zero-crossings. Fixed: floors at -200 dB.
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

### Web Demo (`web/`)

- `index.html` — 4-channel mixer console UI with gain/pan/mute per channel
- `app.js` — Main thread: loads WASM, creates AudioWorkletNode, sends
  compiled module to worklet, wires UI controls
- `mixer-worklet-processor.js` — Auto-generated: wasm-bindgen glue
  inlined + polyfills + AudioWorklet processor. Generates 4 test tones
  (A major chord), feeds to WASM mixer, outputs stereo.
- `styles.css` — Mixer console styling
- `worklet-template.js` — Processor template (clean, no glue)
- `text-encoder-decoder-polyfill.js` — TextDecoder/TextEncoder for worklet

**Build step:** `node build/build-worklet.js` combines polyfill + glue +
processor into the self-contained worklet file.

---

## Architecture Decisions

### PCM-only (no codecs)
No Opus, Vorbis, AAC, or WebCodecs. Raw linear PCM (Float32) end-to-end.
Transported in MPEG2-TS using SMPTE 302M (s302m).

### Per-channel input (decision b)
The engine's `process()` feeds the same input to every channel. The
binding resolves this by calling `engine.process_mix()` once per channel
with that channel's own input, then summing master outputs.

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

## Fork Changes (`maxolgi/oximedia`, 4 commits)

1. **`24a35975`** — Make rayon optional behind `parallel` feature.
   `parallel_mix` module + `process_parallel()` cfg-gated. Removed
   phantom `scirs2-core` dependency.
2. **`b2b065af`** — Patch `std::time` usage for `wasm32-unknown-unknown`
   (session.rs, offline_bounce.rs).
3. **`0e900b20`** — Fix scene_recall.rs `Scene::new` for wasm32.
4. **`b88cf333`** — Fix dynamics zero-crossing bug: `linear_to_db(0)`
   returns -200 dB floor instead of `-infinity`.

All changes are additive: existing behavior is preserved when the
`parallel` feature is off and the target is not wasm32.

## Vendored Dependencies (`vendor/oxifft/`)

oxifft 0.3.2 with `default = ["std"]` (no threading/rayon).
Three crates (oxifft, oxifft-codegen, oxifft-codegen-impl) made
standalone (no workspace inheritance).

---

## Test Inventory

| File | Tests | What it covers |
|------|-------|----------------|
| `tests/native_dsp.rs` | 9 | Pan laws (Linear/-3dB/-6dB), gain, phase, mute, sum |
| `tests/known_answer.rs` | (WASM) | Same via JS interop (node) |
| `tests/run_tests.mjs` | 9 (WASM) | Sum, silence, both-channels, mute, gain, interleaved, PID mapping, subscribe, reconfig |
| `tests/eq_honesty.rs` | 5 | ParametricEq boost/cut/flat/highpass/bypass |
| `tests/eq_integration.rs` | 3 | EQ through real process_mix path |
| `tests/dynamics_honesty.rs` | 16 | Comp/expander/gate/limiter gain math |
| `tests/dynamics_integration.rs` | 3 | Comp/gate through real process_mix |
| `tests/limiter_test.rs` | 3 | Oversampled limiter prevents overs |
| `tests/metering_test.rs` | 5 | Peak/RMS/clip readings |
| `tests/interleave_test.rs` | 3 | De-interleave + PID mapping idempotency |

**Total: 47 native + 9 WASM = 56 tests.**

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
- **Allocation-free process()**: master_left/right/stereo_out/deinterleave
  buffers pre-allocated and reused. Only one unavoidable alloc (JS FFI copy).
- **6-band EQ preset**: Fairlight-matching (HPF/Low/Lo-Mid/Mid/Hi-Mid/High).
  Added to EqEffect + 4 honesty tests.
- **Metering UI**: Worklet reports peak/RMS/clip every 10 blocks. Main
  thread renders green/yellow/red meter bars in the UI.
- **Live mode**: Worklet accepts external PCM via feed_pcm() for WebSRT.
- **CI pipeline**: GitHub Actions (native tests + wasm build + node tests).

### Buffer pool wiring (future)
`process_mix` still allocates 4 Vecs/channel/block internally (engine-level).
CakeMix binding is now allocation-free; remaining allocs are in the engine's
`process_mix` itself. `buffer_pool.rs` exists in the engine but needs wiring.

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

- **Root:** `/home/flibb/CakeMix`
- **Commits:** 28 local commits on `master`
- **Remote:** `git@github.com:maxolgi/CakeMix.git` (not created yet — user creates + pushes)
- **Fork:** `maxolgi/oximedia` — 4 commits on `master` beyond upstream
- **No push policy:** The user handles all git push operations.

---

## Build & Run

```bash
# Build WASM for web target
make build-web

# Run all native tests (no browser needed)
make test-native          # 47 tests

# Run WASM tests in node
make test-wasm            # 9 tests

# Start the web demo
make serve                # http://localhost:8200

# Rebuild the worklet (after wasm changes)
node build/build-worklet.js
```
