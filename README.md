# CakeMix 🍰

Professional WASM audio mixer with WebSRT I/O. Built on the real `oximedia-mixer` DSP engine, compiled to `wasm32-unknown-unknown`.

## Status

**M1 complete.** PCM audio arrives over WebSRT (SRT/WebTransport), is mixed by
the WASM engine in the browser's AudioWorklet, and is re-published over
WebSRT. DSP proven with 57 native + 44 WASM known-answer tests.

| Milestone | Status |
|-----------|--------|
| M0 — WASM build + DSP tests | ✅ |
| M1 — PCM end-to-end via WebSRT | ✅ |
| M2 — Multi-PID (8 stereo / 16 mono) | ⏳ Needs WebSRT MPTS |
| M3 — Multi-session sum | ⏳ |
| M4 — Pro DSP (EQ, dynamics, metering) | 🔨 In progress |

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

The server (`crates/cakemix-server`) embeds the built UI (`web/`) and the
wasm-pack output (`crates/mixer-wasm/pkg`) **at compile time** — both are
built by `make serve` first. The reference WebSRT web app is served on its
own port (`--web-port`, default 8201) after `make build-websrt-web` + a
server rebuild.

## Architecture

```
WebSRT (SRT over WebTransport)
  ↓ MPEG2-TS with SMPTE 302M audio PIDs
mpeg2ts-wasm demuxer (WebSRT repo)
  ↓ Float32 interleaved per PID
MixerWasm (this crate, in an AudioWorklet or Worker)
  ↓ per-channel DSP: gain → EQ → dynamics → pan → sum
  ↓ master bus: limiter → stereo out
Web Audio / WebSRT publish
```

### PCM-only

No codecs anywhere. Raw linear PCM (Float32) end-to-end. Audio is transported inside MPEG2-TS using SMPTE 302M (s302m) encapsulation over WebSRT/SRT.

See:
- `audioplan.md` — full PCM transport design and phased plan
- `WEBSRT_CHANGES.md` — exact edits needed in WebSRT's ts-muxer-wasm
- `plan.md` — overall milestone roadmap
- `ENGINE_API.md` — verbatim API map of the oximedia-mixer engine

### Per-channel input architecture

The engine's `process()` feeds the same input to every channel. We resolve this at the binding layer by calling `engine.process_mix()` once per channel with that channel's own input, then summing the master outputs.

### Forks

- **`maxolgi/oximedia`** — rayon optional behind `parallel` feature, `std::time` patched for wasm32, dynamics zero-crossing bug fixed

## DSP modules (M4 progress)

All verified with known-answer honesty tests (per AGENTS.md honesty rule):

| Module | Real DSP? | Tests | Wired into chain? |
|--------|-----------|-------|-------------------|
| Mixer (gain/pan/sum) | ✅ | 9 native + 5 WASM | ✅ |
| ParametricEq | ✅ (RBJ biquads) | 5 honesty + 3 integration | ✅ |
| Compressor | ✅ | 16 honesty + 3 integration | ✅ |
| Expander | ✅ | (in honesty suite) | ⚠️ adapter tested, not yet wired |
| Gate | ✅ | (in honesty suite) | ✅ |
| OversampledLimiter | ✅ | 3 | ✅ (in `process()`) |

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
