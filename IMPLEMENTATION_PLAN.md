# IMPLEMENTATION PLAN — WASM Audio Mixer

A standalone professional audio mixer compiled to WebAssembly, with audio
carried in and out of the browser over WebSRT. Built on a port of
`oximedia-mixer`; consumes WebSRT for transport.

Read first: `FINDINGS.md` (verified technical facts) and `AGENTS.md`
(repo rules). This document is the milestone-by-milestone execution plan.

---

## Goal

Up to 128 input strips, full console (EQ, dynamics, buses, sends/returns,
scenes/snapshots, automation, metering), running in the browser as WASM,
fed by WebSRT (MPTS for local density, multi-session for distributed
sources), publishing a mixed master back over WebSRT. Host apps need no
COOP/COEP headers.

---

## Architecture

```
                       Browser (one or more WebSRT sessions)
 ┌──────────────────────────────────────────────────────────────────┐
 │                                                                  │
 │  Web Worker(s)                                                   │
 │  ┌───────────────┐   PCM    ┌─────────────────┐   PCM   ┌──────┐ │
 │  │ WebSRT        │─────────▶│ oximedia-mixer  │────────▶│WebSRT│ │
 │  │ receiver(s)   │ Float32  │ (WASM, 1 thread)│ Float32 │sender│ │
 │  │ + TS demux    │          │ process()       │         │+TSmux│ │
 │  │ + Opus decode │          │                 │         │+Opus │ │
 │  └───────────────┘          └─────────────────┘         └──────┘ │
 │         ▲ SRT/WebTransport         ▲ control/meters        │      │
 │         │                          │                       │      │
 │  ┌──────┴──────────────────────────┴───────────────────────┴────┐ │
 │  │ Main thread / UI: channel strips, faders, EQ, scenes, meters │ │
 │  └─────────────────────────────────────────────────────────────┘ │
 │                                                                  │
 │  Optional: AudioWorklet for local-monitor speaker output         │
 └──────────────────────────────────────────────────────────────────┘
                          ▲ SRT/WebTransport   ▲
                          │                     │
                   (sources: OBS,       (downstream: viewer,
                    other browsers)      re-broadcast, record)
```

### Data flow
1. One or more WebSRT sessions deliver MPEG-TS over WebTransport.
2. `mpeg2ts-wasm` demuxes → per-program audio PIDs → Opus packets.
3. Opus decode (WebCodecs `AudioDecoder` or `oximedia-audio`) → Float32 PCM.
4. PCM posted to the mixer worker via transferred `MessagePort`.
5. `mixer.process(inputs, block)` every 128–960 samples → master Float32.
6. Master → Opus encode → `ts-muxer-wasm` → SRT publish via WebTransport.

### Threading model
- Mixer WASM is **single-threaded** on the audio path (correct by design —
  matches Web Audio's render quantum).
- `rayon` cfg-gated behind `parallel` (off on wasm); used only by offline
  bounce, which isn't on the real-time path.
- PCM handoff = transferred `MessagePort`, never `SharedArrayBuffer`. No
  COOP/COEP requirement on host apps.

---

## Repo structure (target)

```
audio-mixer-wasm/
├── Cargo.toml                    # workspace
├── crates/
│   └── mixer-wasm/               # cdylib + rlib; wasm-bindgen over oximedia-mixer
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs            # wasm-bindgen exports (the real binding layer)
│           └── ...               # glue, block framing, meter readbacks
├── vendor/
│   ├── oximedia-mixer/           # the engine (dep, fork, or submodule — decide at M0)
│   └── WebSRT/                   # submodule: srt-wasm, ts-muxer-wasm, mpeg2ts-wasm
├── web/                          # TS frontend
│   ├── worker.ts                 # owns mixer WASM + WebSRT rx/tx workers
│   ├── mixer-bridge.ts           # main ↔ worker postMessage protocol
│   ├── audio/                    # decode tap, capture, encode pipeline
│   └── ui/                       # channel strips, faders, meters, EQ, scenes
├── tests/
│   └── known_answer.rs           # wasm-bindgen-test: synthetic PCM mix
├── scripts/
│   └── build-wasm.sh             # wasm-pack invocation
└── README.md
```

---

## Engine port strategy (M0 focus)

### Source decision
Pick one at M0 kickoff (cleanest first):
1. **crates.io dep** — `oximedia` is published (111 crates). Cleanest if it
   builds on wasm after feature selection. Try first.
2. **git dep** on `cool-japan/oximedia` — if crates.io version lags or
   features need tweaking.
3. **Fork-and-strip** — copy `oximedia-mixer/` + minimal deps into this
   repo, remove `rayon`/`scirs2-core` where they block. Last resort; most
   maintenance.

### Port tasks
1. **Read `lib.rs` (66KB) first** — map the public API: the top-level
   mixer/console type, channel/bus construction, and the `process`/`render`
   entry-point signature. Everything else flows from this.
   URL: https://raw.githubusercontent.com/cool-japan/oximedia/master/crates/oximedia-mixer/src/lib.rs
2. **Build on wasm** — `cargo build --target wasm32-unknown-unknown`. Resolve
   `scirs2-core` (verify/stub). Gate `rayon` behind `parallel` (off).
3. **Write `mixer-wasm/src/lib.rs`** — wasm-bindgen exports:
   - `new(sample_rate, block_size, max_channels)`
   - channel add/remove, per-channel gain/pan/mute/solo
   - `process(inputs: Float32Array[], block_size) -> Float32Array` (master)
   - meter readbacks (engine has `metering.rs`, `analysis_meter.rs`)
4. **Known-answer test** — two synthetic sine tones at known frequencies,
   sum them, verify the output matches a direct-computed reference within
   a tolerance. This is the honesty gate (AGENTS.md "honesty rule").

---

## WebSRT extension work (happens in WebSRT, user applies, then bump pin)

These are prerequisites for later milestones. Track them; don't block M0.

| Extension | File (WebSRT) | Why |
|-----------|---------------|-----|
| Audio-only TS mux (no video PID) | `crates/ts-muxer-wasm/src/lib.rs` | M1 publish path; current muxer assumes video+audio |
| MPTS mux (multiple PMTs/programs) | `crates/ts-muxer-wasm/src/lib.rs` | M2 dense local transport |
| MPTS demux (per-program grouping API) | `crates/mpeg2ts-wasm/src/lib.rs` | M2 receive side |
| Multi-PID-per-program (N audio streams, per-PID channel count) | both mux + demux | M2 (16ch = 16×mono or 8×stereo) |

When a WebSRT change lands: `git submodule update --remote vendor/WebSRT`
+ rebuild wasm + commit the pin bump. Never edit WebSRT from this repo.

---

## Milestones

Each milestone has a **verifiable success criterion**. Do not advance until
the criterion passes.

### M0 — mixer runs in WASM, DSP proven
**Scope:** `crates/mixer-wasm/` compiles on `wasm32-unknown-unknown`;
`process()` runs; known-answer test green. No UI, no network.
**Tasks:**
- Source decision (crates.io → git → fork).
- Read `oximedia-mixer/src/lib.rs`; map API.
- Resolve wasm build (scirs2-core, rayon cfg-gate).
- Write `mixer-wasm` binding (constructor, channel config, `process`).
- Write `tests/known_answer.rs` (two sines → summed reference match).
**Verify:** `wasm-pack test --node` passes the known-answer test; output
matches reference within tolerance; `cargo build --target wasm32-unknown-unknown` clean.
**Blocker to watch:** if `process()` output is wrong/silent, suspect the
"compiles-but-fake" pattern — investigate before proceeding.

### M1 — single stereo program in/out (audio-only end to end)
**Scope:** one WebSRT source (stereo Opus) decoded → one mixer strip →
master out → Opus encode → audio-only TS → SRT publish. Proves the
audio-only mux path.
**Tasks:**
- [WebSRT] audio-only mux path lands; bump pin.
- Receive: tap decoded `AudioData` → Float32 → mixer input (mirror
  `streaming-input.ts:542`, but to the mixer instead of MediaStreamTrackGenerator).
- Publish: master Float32 → Opus encode → `ts-muxer-wasm` → SRT (mirror
  `stream-worker.js` with the Webamp tap replaced by mixer master).
**Verify:** publish a tone from OBS (or another browser via WebSRT) →
mixer passes it through → receive the published stream in a test viewer
and confirm audio present at correct level. No video PID in the output TS.

### M2 — MPTS, one program with 8 stereo / 16 mono PIDs
**Scope:** one WebSRT connection delivers a multi-PID program → 16 strips
on the mixer. Proves multi-PID demux + program grouping + flexible
per-PID channel count.
**Tasks:**
- [WebSRT] MPTS mux + demux + multi-PID-per-program land; bump pin.
- Mixer input model: program → strips (1 mono PID → 1 strip; 1 stereo PID
  → 2 strips or 1 stereo strip per config).
- UI: multiple strips with independent gain/pan/mute/solo.
**Verify:** source publishes a program with 8 stereo Opus PIDs; mixer
shows 16 channels; solo/mute/pan each work; master sums correctly
(known-answer: N unity-gain identical signals → +N× level pre-master).

### M3 — multi-session sum at master
**Scope:** a second WebSRT origin (separate session) contributes another
program; both sum at the master. Cross-session alignment comes from
matched SRT `latency` (protocol-level — no mixer alignment code).
**Tasks:**
- Per-session SRT `latency` config surfaced and documented.
- Mixer consumes multiple receivers' paced outputs into the master sum.
- Trivial bounded queue per input to absorb residual within-window skew
  (implementation detail, not a subsystem).
**Verify:** two origins publish simultaneously with matched latency;
master output is the correct sum; drop one session and confirm the other
continues cleanly. Measure inter-session skew vs. configured latency.

### M4 — scale to 128 strips + layer pro features
**Scope:** reach full console by layering `oximedia-mixer` modules onto
strips and buses, validating each with a DSP known-answer test.
**Feature roadmap (from engine module inventory):**
- Channel EQ (`eq_band.rs`) — known-answer: band gain at center freq.
- Dynamics (`dynamics.rs`, `gain_computer.rs`, `limiter.rs`,
  `oversampled_limiter.rs`) — known-answer: steady-state gain reduction.
- Buses & sends (`bus.rs`, `group_bus.rs`, `mix_bus.rs`, `aux_send.rs`,
  `send_return.rs`) — known-answer: signal reaches the right bus.
- Scenes/snapshots (`mix_scene.rs`, `scene_recall.rs`) — state round-trip.
- Automation (`automation.rs` + family) — parameter schedule playback.
- Metering (`metering.rs`, `analysis_meter.rs`, `meter_bridge.rs`) —
  peak/RMS/EBU R128 vs. reference.
- Ambisonics (`ambisonics.rs`) — optional, for spatial output.
- MIDI control (`midi_control.rs`) — optional, for control surfaces.
**Verify:** each feature ships with its own known-answer test. No feature
is declared done on "it compiles" alone.

---

## Key decisions (rationale)

- **Port oximedia-mixer rather than wrap the existing `mixer_wasm.rs`:**
  the existing WASM mixer is a 250-line toy that doesn't touch the engine
  (FINDINGS.md §2). There is nothing to wrap.
- **Single-threaded wasm, no COOP/COEP:** real-time audio is inherently
  single-threaded; `MessagePort` transfer (SlopShady's model) avoids
  infecting host apps with cross-origin-isolation requirements.
- **Alignment is protocol-level, not a mixer subsystem:** SRT's `latency`
  setting + playout buffer align multi-session streams. The mixer just
  sums aligned inputs.
- **Treat oximedia as a black-box release:** single-author, release-snapshot
  repo, documented "fake module" history. Trust only what you test.

---

## Risks & mitigations

| Risk | Mitigation |
|------|------------|
| `scirs2-core` (or other dep) blocks wasm build | Stub/strip; or fork-and-strip the engine. Decide at M0. |
| A ported module is "compiles-but-fake" | Per-module known-answer tests (honesty rule). Investigate, don't paper over. |
| `oximedia-mixer` API is awkward for real-time `process()` | The API mapping at M0 step 1 surfaces this early; worst case, write a thin real-time-friendly adapter over the engine's types. |
| WebSRT audio-only / MPTS work is larger than expected | M1/M2 don't require all of it; sequence behind M0. Keep the dummy-video fallback (send a tiny black video stream) as a way to unblock publish without the audio-only muxer change. |
| 128-strip decode load in one worker | Measure at M4; if decode-bound, split Opus decode across workers, postMessage PCM to the single mix worker. Architecture allows it. |

---

## Open questions (resolve at the indicated milestone)

- **M0:** crates.io vs git dep vs fork-and-strip — pick after a build attempt.
- **M0:** what is `oximedia-mixer`'s real top-level `process`/`render` API?
  (read `lib.rs`.)
- **M1:** audio-only TS publish, or dummy-video fallback first? (dummy-video
  unblocks M1 without waiting on WebSRT.)
- **M2:** does the existing `mpeg2ts-wasm` demux already expose per-PID
  grouping that MPTS can build on, or is it single-program throughout?

---

## First concrete steps (for the continuation agent)

1. Read `FINDINGS.md` and `AGENTS.md` fully.
2. Fetch and read
   `https://raw.githubusercontent.com/cool-japan/oximedia/master/crates/oximedia-mixer/src/lib.rs`
   — map the public API. Record the result in a new `ENGINE_API.md`.
3. Scaffold the repo: workspace `Cargo.toml`, `crates/mixer-wasm/`, git
   init, `.gitignore`, `rust-toolchain.toml`.
4. Wire `oximedia-mixer` as a dep; attempt `cargo build --target
   wasm32-unknown-unknown`; resolve `scirs2-core` and gate `rayon`.
5. Write the minimal `mixer-wasm` binding + the known-answer test.
6. `wasm-pack test --node`. Green = M0 done.
