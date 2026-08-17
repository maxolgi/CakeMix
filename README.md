# CakeMix 🍰

Professional WASM audio mixer with WebSRT I/O. Built on the real `oximedia-mixer` DSP engine, compiled to `wasm32-unknown-unknown`.

## Status

**M1 complete.** PCM audio arrives over WebSRT (SRT/WebTransport), is mixed by
the WASM engine in the browser's AudioWorklet, and is re-published over
WebSRT. DSP proven with 57 native + 35 WASM tests (plus a 13-test JS runner).

| Milestone | Status |
|-----------|--------|
| M0 — WASM build + DSP tests | ✅ |
| M1 — PCM end-to-end via WebSRT | ✅ |
| M2 — Multi-PID (8 stereo / 16 mono) | ⏳ Needs WebSRT MPTS |
| M3 — Multi-session sum | ⏳ |
| M4 — Pro DSP (EQ, dynamics, metering) | 🔨 In progress |
| M5 — Bus architecture + scenes | 🔨 8 buses shipped; scenes pending |

## Quick start

```bash
# Build for wasm32
make build-wasm

# Run native DSP tests (no browser needed)
make test-native

# Run WASM tests
make test-wasm

# Run everything
make test-all
```

## Running the server

```bash
make serve      # builds wasm + UI, serves http://localhost:8200 (no TLS)
make serve-tls  # same, HTTPS with an auto-generated self-signed cert
```

`web/` is generated and gitignored (Vite output + worklet bundle). The
server (`crates/cakemix-server`) embeds `web/`, the wasm-pack output
(`crates/mixer-wasm/pkg`) and the reference web app
(`vendor/WebSRT/web/dist`) **at compile time** — `make serve` builds all
three first. The reference WebSRT web app is served on its own port
(`--web-port`, default 8201).

## Architecture

```
WebSRT (SRT over WebTransport)
  ↓ MPEG2-TS with SMPTE 302M audio PIDs
mpeg2ts-wasm demuxer (WebSRT repo)
  ↓ Float32 interleaved per PID
MixerWasm (this crate, in an AudioWorklet or Worker)
  ↓ elastic FIFOs → staging per strip: gate → expander → comp → EQ (raw source)
  ↓ 9 engine instances (1 main + 8 buses) — all summing via process_mix_rt
  ↓ bus tail (gain/mute → publish / feed master) → master gain → limiter
Web Audio / WebSRT publish (master stereo, Nch direct-out taps, or any bus 1-8)
```

### Console

128 input strips + 8 summing buses (16 slots each). Bus slots tap their
source's **raw** signal — the source's fader/mute/EQ/dynamics never affect
the bus path. Two routing toggles: **MAIN** per strip (off = the strip
reaches master only via its bus slots) and **FEEDS MAIN** per bus (off =
independent bus, still publishable). Live per-strip COMP GR meters.

### PCM-only

No codecs anywhere. Raw linear PCM (Float32) end-to-end. Audio is transported inside MPEG2-TS using SMPTE 302M (s302m) encapsulation over WebSRT/SRT (registration descriptor "BSSD").

### PCM handoff contract (CakeMix ↔ WebSRT)

Browser-only integration — no compile-time dependency; pure `MessagePort` /
`postMessage`:

- **worker → worklet** (transferred `MessagePort`, zero main-thread hops):
  `{type:'batch', msgs:[{type:'pcm', pid, channelCount, samples: Float32Array, pts}]}`
- **worklet → main**: `{type:'pid-mapped', pid, chStart, channelCount}`
  (chStart −1 = past the cap) and `{type:'pcm-dropped', total}` (cumulative).
- A PID is mapped on its **first pcm** — the channelCount detected from the
  AES3 frame header is authoritative, not the PMT. PIDs pack consecutively
  from mixer channel 0, capped at 128 strips; overflow PCM is dropped+counted.
- `map_pid` re-anchors the strip's elastic playout FIFOs (a remap = new stream).
- ffmpeg publisher: `./fixtures/stream_pcm_real.sh` (30 stereo s302m PIDs,
  looped; `WEBSTRT_SRT_URL=…` overrides the target).

### Per-channel input architecture

The upstream engine's `process()` feeds the same input to every channel. We
solved this in the engine fork: `ProcessingEngine::process_mix_rt` takes
per-channel input slices, sums into caller-provided buffers with zero
steady-state allocation, and is step-identical to `process_mix` (pinned by
parity tests in the fork). The binding's staging layer resolves each strip's
audio and drives nine engine instances (1 main + 8 buses) per block — the
engine performs all summing.

### Forks

- **`maxolgi/oximedia`** — engine fork on top of upstream 0.2.1:
  `process_mix_rt` (per-channel-input, alloc-free real-time mixing),
  `ChannelId::nil()`, `Copy` on `ChannelProcessParams`,
  `Compressor::last_gain_reduction_db()`, rayon optional behind `parallel`
  (off for wasm), `std::time`/scene fixes for wasm32, `linear_to_db` −200 dB
  floor (envelope followers poisoned permanently on digital silence without
  it). Pinned via `[patch.crates-io]` at the pushed fork master.

## Performance / DSP capacity

Measured driving the real `process_block` (`glitch_sim`, native release).
Budget: **2667 µs/block** (128 frames @ 48 kHz).

| Load | avg | p99 | headroom |
|---|---|---|---|
| Realistic — 60 strips + 2 buses × 4 slots, 4 comps, 16-ch tap | 253 µs | **275 µs** | **9.7×** (CI-enforced) |
| Full console — 128 strips + 8×16 bus slots, comp+gate on 32 strips | 1074 µs | **1161 µs** | **2.3×** |

- ~3.8 µs/strip/block, scaling ~linearly (staging + 6-band EQ dominates;
  the nine engine sums add little). 256 strips ≈ 44% of budget — no room
  for 2× without slimmer staging.
- Zero starvation, zero NaN/Inf, FIFO depth bounded under 300 ppm clock
  drift (elastic slip reconciliation).
- Known: limiter transient inter-sample overshoot up to +0.6 dBFS at
  full-load sums (steady-state ceiling holds; see `limiter_test`).
- Repro: `cargo test -p mixer-wasm --release --test glitch_sim`
  (full load: `-- --ignored --nocapture`). Steady state is alloc-free —
  pinned by `alloc_test` (counting global allocator, 0 bytes/block).

## DSP modules (M4 progress)

All verified with known-answer honesty tests (per AGENTS.md honesty rule):

| Module | Real DSP? | Tests | Wired into chain? |
|--------|-----------|-------|-------------------|
| Mixer (gain/pan/sum) | ✅ | parity + alloc pinned in fork; 9 native + 5 WASM here | ✅ engine-driven (`process_mix_rt`, 9 instances/block) |
| ParametricEq | ✅ (RBJ biquads) | 5 honesty + 3 integration + 4 six-band | ✅ staging layer |
| Compressor | ✅ | 16 honesty + 3 integration | ✅ staging layer + GR meter |
| Expander | ✅ | (in honesty suite) | ✅ staging layer |
| Gate | ✅ | (in honesty suite) | ✅ staging layer |
| OversampledLimiter | ✅ | 3 | ✅ master bus |

## License

AGPL-3.0 (see `LICENSE`). The repo contains code derived from
[Eyevinn audio-mixer](https://github.com/Eyevinn/audio-mixer) (AGPL-3.0),
which sets the license for the combined work. Third-party components:

| Component | License |
|-----------|---------|
| `maxolgi/oximedia` (DSP engine fork) | Apache-2.0 |
| `vendor/WebSRT` (submodule) | MPL-2.0 |
| `maxolgi/srt-rs` (SRT transport) | Apache-2.0 |
| `maxolgi/mpeg2ts` (TS demuxer) | MIT |
