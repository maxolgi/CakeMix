# CakeMix 🍰

Professional WASM audio mixer with WebSRT I/O. Built on the real `oximedia-mixer` DSP engine, compiled to `wasm32-unknown-unknown`.

## Status

**M0 complete.** Mixer runs in WASM, DSP proven with 42 native + 9 WASM tests.

| Milestone | Status |
|-----------|--------|
| M0 — WASM build + DSP tests | ✅ |
| M1 — PCM end-to-end via WebSRT | Blocked on WebSRT changes |
| M4 — Pro DSP (EQ, dynamics, metering) | In progress |

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
- **`vendor/oxifft/`** — oxifft 0.3.2 with `default = ["std"]` (no threading/rayon)

## DSP modules (M4 progress)

All verified with known-answer honesty tests (per AGENTS.md honesty rule):

| Module | Real DSP? | Tests | Wired into chain? |
|--------|-----------|-------|-------------------|
| Mixer (gain/pan/sum) | ✅ | 9 native + 5 WASM | ✅ |
| ParametricEq | ✅ (RBJ biquads) | 5 honesty + 3 integration | ✅ |
| Compressor | ✅ | 16 honesty + 3 integration | ✅ |
| Expander | ✅ | (in honesty suite) | ✅ |
| Gate | ✅ | (in honesty suite) | ✅ |
| OversampledLimiter | ✅ | 3 | ✅ (in `process()`) |

## License

Apache-2.0
