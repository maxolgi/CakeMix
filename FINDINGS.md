# FINDINGS — verified facts from the investigation session

Everything below was verified by reading the actual repos/files during the
planning session (Aug 2026). A continuation agent can trust these without
re-fetching, except where a line is marked **[UNVERIFIED]** — those need a
first-hand read.

---

## 1. The engine: `oximedia-mixer`

### What oximedia is
- Repo: https://github.com/cool-japan/oximedia (Apache-2.0)
- Self-described: "Pure Rust reconstruction of FFmpeg + OpenCV." 114-crate
  workspace, ~2.95M SLOC, v0.2.0 active. Real, substantial codebase.
- **Commit history is a release-snapshot mirror:** 11 commits total, all
  message "Availability of X.Y.Z", single author `KitaSan`/`cool-japan`,
  Mar→Jul 2026. No visible development history, no PRs (0 open). The source
  exists elsewhere; GitHub only receives release blobs. Implication: you
  cannot trace how any module evolved — treat each crate as a black-box
  release artifact.

### `crates/oximedia-mixer/` — the real engine
Listed in the README as "Professional digital audio mixer (multi-channel,
automation)" — status **Stable** (self-assigned). It is genuine, deep code:
~45 source files, 8–50KB each (~500KB+ of Rust). `lib.rs` alone is 66KB.

**Module inventory (the feature roadmap):**

| Module | Likely purpose |
|--------|----------------|
| `lib.rs` (66KB) | Top-level public API **[UNVERIFIED — read first]** |
| `processing.rs` (50KB) | Main processing graph |
| `channel_strip.rs` (40KB) | Channel strip (gain/pan/EQ/dynamics/fader) |
| `automation.rs` (40KB), `automation_engine.rs`, `automation_lane.rs`, `automation_playback.rs`, `automation_player.rs`, `daw_automation.rs`, `gain_automation.rs` | Full automation system |
| `ambisonics.rs` (20KB) | HOA Ambisonics encode/decode |
| `analysis_meter.rs`, `metering.rs`, `meter_bridge.rs` | Metering (EBU R128 etc.) |
| `atomic_param.rs` (19KB) | Lock-free param storage (RT-safe param passing) |
| `aux_send.rs`, `send_return.rs` | Aux sends / FX returns |
| `bounce.rs`, `offline_bounce.rs` | Offline render/bounce |
| `buffer_pool.rs` | Buffer reuse pool |
| `bus.rs`, `group_bus.rs`, `mix_bus.rs` | Bus structures |
| `channel.rs`, `channel_fold.rs`, `channel_folder.rs`, `channel_prealloc.rs` | Channel implementations |
| `clip_guard.rs` | Clip protection |
| `crossfade.rs` | Crossfading |
| `cue_monitor.rs`, `monitor_mix.rs` | Cue / monitor mixes |
| `delay_line.rs` | Delay DSP |
| `dynamics.rs`, `gain_computer.rs` | Dynamics (comp/gate) |
| `effects.rs`, `effects_chain.rs`, `insert_chain.rs` | Effect routing |
| `eq_band.rs` | EQ band DSP |
| `fader_group.rs` | VCA-style fader grouping |
| `limiter.rs`, `oversampled_limiter.rs` | Brick-wall limiting |
| `matrix_mixer.rs` | Matrix mixing |
| `midi_control.rs` | MIDI control mapping |
| `mix_scene.rs`, `scene_recall.rs` | Scene memory / recall (snapshots) |
| `pan_matrix.rs` | Panning |
| `parallel_mix.rs` | Parallel processing (**uses rayon**) |
| `param_smoother.rs` | Parameter smoothing (de-zippering) |
| `plugin.rs` | Plugin slots |
| `routing.rs` | Routing matrix |

### Dependencies that block plain `wasm32-unknown-unknown`
From `crates/oximedia-mixer/Cargo.toml`:
```
oximedia-core, oximedia-audio, scirs2-core, rayon, serde, serde_json,
uuid, thiserror, bytes
```
- **`rayon`** — does NOT compile on `wasm32-unknown-unknown` (needs OS
  threads). Used by `parallel_mix.rs` and `offline_bounce.rs` (offline
  paths, not real-time). Resolution: cfg-gate behind a `parallel` feature,
  off for wasm. Real-time audio is single-threaded by design (DAWs, Web
  Audio's 128-frame render quantum) — no audio-thread parallelism lost.
- **`scirs2-core`** — wasm status **[UNVERIFIED]**. First porting task is
  confirming whether it builds clean; if not, strip/stub the pieces the
  mixer actually pulls.
- `oximedia-core`, `oximedia-audio` — `oximedia-audio` is confirmed
  wasm-clean (per oximedia's v0.1.4 notes). `oximedia-core` needs the
  `wasm` feature (see how `oximedia-wasm` pulls it: `features = ["wasm"]`).

---

## 2. The critical finding: there is NO real WASM mixer to wrap

### The existing `mixer_wasm.rs` is a hand-rolled toy
`oximedia-wasm/src/mixer_wasm.rs` (~250 lines, 11KB) is a self-contained
`WasmAudioMixer` struct that does **gain + constant-power pan + sum to
stereo**. That's it. No EQ, no dynamics, no buses, no automation, no
meters. It imports nothing from `oximedia_mixer` — it is a from-scratch
minimal mixer that ignores the 45-module engine entirely.

### `oximedia-mixer` is NOT a dependency of `oximedia-wasm`
The `oximedia-wasm/Cargo.toml` dependency list (~33 oximedia crates)
does **not** include `oximedia-mixer`. So the real engine is not compiled
into any WASM build. The oximedia `TODO.md` documents this pattern
explicitly under "Dependency hygiene": several `*_wasm.rs` modules are
"hand-rolled browser-side implementations that never called into those
crates." `mixer_wasm.rs` is one of these.

**Implication:** the plan's binding layer is written from zero against
the real engine API. It is not "extend the existing wrapper."

---

## 3. The honesty caveat (load-bearing)

oximedia's own `oximedia-wasm/TODO.md` records that **three decoder
classes were removed** because they returned fake data — most notably an
AV1 decoder that returned solid-black frames while the real decode
pipeline was dead code hidden behind `#![allow(dead_code)]`. Tests passed
because they tested the wrong thing.

Lesson: in this codebase, **"it compiles, tests pass, README says Stable"
does not mean "it works."** Every DSP path in `oximedia-mixer` must be
exercised end-to-end with a known-answer test before being trusted. The
M0 milestone (known-answer mix of two sine tones) is not a formality —
it is the gate that proves `process()` actually mixes.

---

## 4. WebSRT — current audio path (the I/O reference)

The user owns WebSRT (https://github.com/maxolgi/WebSRT) and will extend
it. Vendored read-only in SlopShady at `vendor/WebSRT/`.

### Publish path (browser → SRT)
`vendor/WebSRT/crates/ts-muxer-wasm/src/lib.rs:3` — muxer is built around
**"H.264 NAL units (Annex B) + Opus packets (one per PES)"**. Single
program, video+audio PMT. Tests (`h264_default_has_opus_descriptor`,
`av1_has_av01_and_opus_descriptors`) confirm the video-oriented design.
**Audio-only (no video PID) is NOT currently supported** — a real WebSRT
extension.

### Receive path (SRT → browser)
WebSRT worker decodes Opus → `AudioData` → posted to consumer. SlopShady's
`src/js/ui/streaming-input.ts:542` (`_handleAudioData`) is the reference
tap point — it currently feeds `MediaStreamTrackGenerator` → Web Audio
graph. The mixer will instead `copyTo()` Float32 out to mixer inputs.
**Single-program assumption** — multi-program (MPTS) is a WebSRT extension.

### Agreed WebSRT extension work items (user does these, mixer bumps pin)
1. **Audio-only mux** — TS with audio PIDs only, no video elementary stream.
2. **MPTS** — multi-program TS (PAT cycling multiple PMTs). Both
   `ts-muxer-wasm` (mux N programs) and `mpeg2ts-wasm` (per-program
   grouping demux API) get extended.
3. **Multi-PID-per-program** — a program carries N audio elementary
   streams, each tagged with a per-PID channel count (mono/stereo/arbitrary,
   broadcaster-style: 16ch = 16×mono or 8×stereo).

---

## 5. Transport architecture (decided this session)

- **MPTS** for dense/local sources — one SRT connection, one TS carrying
  multiple programs. Phase alignment within the stream is free (shared PCR).
- **Multiple WebSRT sessions** for distributed sources (different origins).
  Phase alignment across sessions is **protocol-level**: matched SRT
  `latency` setting + SRT's playout buffer delivers aligned streams. **Not
  a mixer subsystem** — the mixer just consumes each receiver's paced
  output. (Mirrors AES67/Dante distributed-alignment practice.)
- 128 mixer input strips max. Sources arrive as programs; each program's
  audio PIDs become strips (1 mono PID → 1 strip, 1 stereo PID → 2 strips
  or 1 stereo strip, per routing config).

---

## 6. SlopShady reference files (read-only, do not modify)

SlopShady is the working reference for WebSRT audio I/O in a browser.
Three files are the blueprint:

| File | What it shows |
|------|---------------|
| `src/js/ui/streaming-input.ts:542` (`_handleAudioData`) | Where decoded `AudioData` arrives from the WebSRT worker; current tap into `MediaStreamTrackGenerator`. The mixer's *input* tap point. |
| `src/js/features/stream-audio-worklet.js` | AudioWorklet tapping a source → 960-frame Float32 → transferred `MessagePort`. The mixer *output* capture pattern. 48kHz/960 = 20ms Opus frame. |
| `src/js/features/stream-worker.js` | Worker owning Opus `AudioEncoder` + `ts-muxer-wasm` + SRT publish. Reuse this pipeline for the mixer's master-out publish. |

Key design choice to copy: **transferred `MessagePort` for PCM handoff,
not SharedArrayBuffer.** SlopShady deliberately avoids COOP/COEP. The
mixer must do the same — no `SharedArrayBuffer`, no cross-origin isolation
requirement on host apps.

---

## 7. Useful URLs for the next agent

oximedia-mixer source (raw):
- https://raw.githubusercontent.com/cool-japan/oximedia/master/crates/oximedia-mixer/Cargo.toml
- https://raw.githubusercontent.com/cool-japan/oximedia/master/crates/oximedia-mixer/src/lib.rs  ← **read first to map the API**
- Per-module: `https://raw.githubusercontent.com/cool-japan/oximedia/master/crates/oximedia-mixer/src/<module>.rs`
- Directory listing (JSON): `https://api.github.com/repos/cool-japan/oximedia/contents/crates/oximedia-mixer/src`

oximedia-wasm (the WASM surface, for reference on binding patterns):
- https://raw.githubusercontent.com/cool-japan/oximedia/master/oximedia-wasm/Cargo.toml
- https://raw.githubusercontent.com/cool-japan/oximedia/master/oximedia-wasm/TODO.md  ← honesty notes
- https://raw.githubusercontent.com/cool-japan/oximedia/master/oximedia-wasm/src/mixer_wasm.rs  ← the toy (do not copy)

SlopShady references (local, in this repo):
- `../src/js/ui/streaming-input.ts`
- `../src/js/features/stream-audio-worklet.js`
- `../src/js/features/stream-worker.js`
- `../vendor/WebSRT/crates/ts-muxer-wasm/src/lib.rs`
- `../vendor/WebSRT/web/src/shared/viewer.ts` (receive-side audio pipeline)
