# CakeMix 🍰

Professional WASM audio mixer with QuicSRT I/O. Built on the real `oximedia-mixer` DSP engine, compiled to `wasm32-unknown-unknown`.

## Status

Complete through M5: multi-session QuicSRT PCM input, 128-strip console
(EQ, dynamics, 8 buses, scenes with cross-fade), QuicSRT publish.

| Date | Milestone |
|---|---|
| 2026-10-20 | Per-pair guitarix FX insert chains: the real faust C++ DSP compiled freestanding from `vendor/guitarix` (`crates/mixer-wasm/gxfx/`), 11 effects honesty-tested and live (wah, overdrive, distortion — bossds1 via the gx_resampler/zita oversampling TU — compressor, chorus, flanger, phaser, tremolo, delay, echo, reverb), 11 slots per stereo pair post-EQ, FX rack UI, scene snap, engine-sourced catalog |
| 2026-10-19 | QuicSRT migration: replaced the Rust WebSRT stack with the C++ QuicSRT engine (`vendor/QuicSRT-wasm` submodule, emcc + pthreads), COOP/COEP on every response, embedded gateway removed in favor of the external QuicSRT-gateway |
| 2026-10-19 | Stream-health gate: per-channel delivery-pathology protection (below) — lossy/bursty streams (any latency; jitter-tail events at 120 ms+ included) fade their strips to silence (~40 ms worst-case blip) instead of blasting constant full-scale granular static |

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

Prerequisites: the pinned Rust toolchain with `wasm32-unknown-unknown`.
Building mixer-wasm compiles the gxfx guitarix C++ DSP freestanding —
that needs `clang++` and an archiver on PATH (`ar`, else `llvm-ar`, else
`$AR`) plus the vendored sources:
`git submodule update --init vendor/guitarix`. The QuicSRT engine step
(`make serve`) additionally needs emsdk (below).

## Running the server

```bash
make serve      # builds wasm + emcc engine + UI + worklet, serves http://localhost:8200 (no TLS)
make serve-tls  # same, HTTPS with an auto-generated self-signed cert
```

`web/` is generated and gitignored (Vite output + worklet bundle + the
QuicSRT engine staged at `web/build/`). The server
(`crates/cakemix-server`) embeds only `web/` and the wasm-pack output
(`crates/mixer-wasm/pkg`) **at compile time** — `make serve` builds
everything first, including the emcc engine (`make build-quicsrt-engine`,
which needs emsdk and inits the submodule's own deps).

cakemix-server embeds **no gateway**. The external QuicSRT-gateway must
be running (on the dev machine it is, under supervisord: web/player
HTTPS :8443, WebTransport ingest :4435, SRT ingest :9102, health :9292);
CakeMix subscribes/publishes through it via the drawer's URL field
(default `https://127.0.0.1:8443`).

Every response carries `Cross-Origin-Opener-Policy: same-origin` +
`Cross-Origin-Embedder-Policy: require-corp`: the QuicSRT engine
(emscripten + pthreads → `SharedArrayBuffer`) requires a
cross-origin-isolated page, and `crossOriginIsolated` needs a secure
context — with `--no-tls` plain HTTP that means localhost only; remote
LAN clients need TLS mode.

The WebTransport identity belongs to the gateway (self-signed mode: hash
published at its `cert-hash.js`, pinned client-side via
`serverCertificateHashes`). Its web origin is cross-origin to us, so the
hash is resolved through our `/api/cert-hash?url=` proxy, which applies
the same split: it accepts self-signed certs only from closed-network
hosts (private/loopback IPs, dotless or `.local`-style names) and
strictly validates public ones. Web TLS for cakemix-server itself:
self-signed by default, or `--cert-pem`/`--key-pem` (mkcert or a real
CA such as Let's Encrypt).

## Architecture

```
QuicSRT (SRT over WebTransport, via the external QuicSRT-gateway)
  ↓ MPEG2-TS with SMPTE 302M audio PIDs
QuicSRT engine demuxer (vendor/QuicSRT-wasm — C++/emscripten, in a Worker)
  ↓ Float32 interleaved per PID
MixerWasm (this crate, in an AudioWorklet or Worker)
  ↓ elastic FIFOs → staging per strip: gate → expander → comp → EQ → FX pair chains (bus slots tap the raw source)
  ↓ 9 engine instances (1 main + 8 buses) — all summing via process_mix_rt
  ↓ bus tail (gain/mute → publish / feed master) → master gain → limiter
Web Audio / QuicSRT publish (master stereo, Nch direct-out taps, or any bus 1-8)
```

### Console

128 input strips + 8 summing buses (16 slots each). Bus slots tap their
source's **raw** signal — the source's fader/mute/EQ/dynamics never affect
the bus path. Two routing toggles: **MAIN** per strip (off = the strip
reaches master only via its bus slots) and **FEEDS MAIN** per bus (off =
independent bus, still publishable). Live per-strip COMP GR meters.
**Scenes** capture the entire console (strips, buses, master); recall is
instant or cross-faded (dB-domain gains, audio-rate fade inside the mixer
block — any user set cancels the fade). Multiple QuicSRT receive sessions
run concurrently — PIDs are namespaced per session (keyed mapping), so
different gateways/streams may reuse PID numbers.

**FX chains** insert in the staging pass post-EQ, pre-meter/fader. The
128 input strips form 64 stereo pairs; pair *t* (strips 2t/2t+1, the
even strip owning) carries one 11-slot chain — any effect per slot or
empty, per-slot IN/BYP, params in engine units. Stereo effects run
native stereo across the pair, mono effects dual-mono. Eleven guitarix
effects are live (wah, overdrive, distortion, compressor, chorus,
flanger, phaser, tremolo, delay, echo, reverb). The effect catalog is
delivered once from the engine (`fx_catalog_json`, single source of
truth); DSP is created/activated eagerly at message time, never inside
`process()`. Delay/echo/reverb allocate MB-scale tapes at create, so
the engine caps concurrently created memory-class instances at 96
(~384 MiB worst case); rejections surface in the UI (fx-error toast +
store rollback). Scene recall snaps FX state — never interpolates,
even mid-cross-fade — and muted chains keep running so tails ring out.

### PCM-only

No codecs anywhere. Raw linear PCM (Float32) end-to-end. Audio is transported inside MPEG2-TS using SMPTE 302M (s302m) encapsulation over QuicSRT/SRT (registration descriptor "BSSD").

### PCM handoff contract (CakeMix ↔ QuicSRT)

Browser-only integration — the boundary is pure `MessagePort` /
`postMessage`; the receive worker is vendored at source level from the
submodule (`vendor/QuicSRT-wasm/page/vendor/worker.ts`), the publish
worker drives the engine's C-ABI `sendMessage` (`js/srt_engine.js` +
`js/muxer_engine.js`). The `WorkerCmd`/`WorkerMsg` shapes are unchanged
from the previous stack, and the engine's demux/mux byte parity is pinned
upstream by QuicSRT-wasm's parity gates:

- **worker → worklet** (transferred `MessagePort`, zero main-thread hops):
  `{type:'batch', msgs:[{type:'pcm', pid, channelCount, samples: Float32Array, pts}]}`
- **worklet → main**: `{type:'pid-mapped', pid, chStart, channelCount}`
  (chStart −1 = past the cap) and `{type:'pcm-dropped', total}` (cumulative).
- A PID is mapped on its **first pcm** — the channelCount detected from the
  AES3 frame header is authoritative, not the PMT. PIDs pack consecutively
  from mixer channel 0, capped at 128 strips; overflow PCM is dropped+counted.
- `map_pid` re-anchors the strip's elastic playout FIFOs (a remap = new stream).
- ffmpeg publisher: `./fixtures/stream_pcm_real.sh` (30 stereo s302m PIDs,
  looped; `WEBSTRT_SRT_URL=srt://127.0.0.1:9102` overrides the target —
  the script's baked-in default still points at the old port, so set the
  gateway's SRT ingest explicitly).

### Per-channel input architecture

The upstream engine's `process()` feeds the same input to every channel. We
solved this in the engine fork: `ProcessingEngine::process_mix_rt` takes
per-channel input slices, sums into caller-provided buffers with zero
steady-state allocation, and is step-identical to `process_mix` (pinned by
parity tests in the fork). The binding's staging layer resolves each strip's
audio and drives nine engine instances (1 main + 8 buses) per block — the
engine performs all summing.

### Stream-health gate (per-channel delivery protection)

The elastic playout buffer reconciles clock *drift* (hundreds of ppm), but
pathological delivery cannot be patched over, from either side: net-slow
(TSBPD latency below the network's jitter tail → SRT drop storms) starves
the FIFOs — every starvation a hole, every insert-slip a repeat — and
net-fast (a post-stall backlog flush) pins the queue at the drop-oldest
cap and chops audio from the head with no starvation at all. At high rates
either "repair" becomes constant full-scale granular static (the "loud
static at low latency" failure; reproduced live with `rxDropped` climbing
and the starve/insert/overflow rates pegged). Each elastic FIFO therefore
tracks its own delivery health (`crates/mixer-wasm/src/fifo.rs`):

- **Degraded** = >8 starved blocks or >8 insert slips per ~1 s
  statistics window; >12 trim slips or >256 cap-dropped frames per
  window; or the fast paths — ≥12 starved blocks (≈32 ms of holes),
  ≥12 trims, or ≥1024 cap-dropped frames mid-window — which engage the
  fade immediately instead of at the window boundary.
- While degraded the channel **fades out over ≈5 ms** (per-sample ramp on
  the FIFO drain) and concealment inserts stop; it **fades back in over
  ≈11 ms** after two consecutive clean windows. The drain keeps running,
  so the FIFO stays rate-balanced and recovery is sample-accurate.
- Telemetry rides the per-strip meters JSON (`dg`, `sb`, `ins`, `ov` —
  the last counting drop-oldest overflow frames); the UI shows a red
  `GATED` badge on affected strips (`ChannelDetailPanel`) with a tooltip
  pointing at the latency knob.

Pinned by `tests/stream_health_test.rs`: bursty (net-slow) delivery gates
fast and recovers; net-fast backlog churn gates with zero starvation
(proving the trim/overflow paths); paced delivery never gates; 200 ppm
drift reconciliation never gates. The remaining fix belongs in the
QuicSRT repos (enforce a sane minimum TSBPD latency / surface the
effective value), not here.

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
| Realistic + FX — realistic load + 16 enabled chains (reverb×4, delay×4, chorus×4, overdrive×4) | 275–283 µs | **305–313 µs** | **~8.6×** (CI-enforced) |
| Full console — 128 strips + 8×16 bus slots, comp+gate on 32 strips | 1074 µs | **1161 µs** | **2.3×** |

- ~3.8 µs/strip/block, scaling ~linearly (staging + 6-band EQ dominates;
  the nine engine sums add little). 256 strips ≈ 44% of budget — no room
  for 2× without slimmer staging.
- FX marginal cost ≈ 2 µs per enabled chain per block (reverb/delay/
  chorus/overdrive alike; measured over multiple 1-min native-release
  runs) — extrapolated, all 64 pairs running one effect each still
  clear the budget with room to spare.
- Zero starvation, zero NaN/Inf, FIFO depth bounded under 300 ppm clock
  drift (elastic slip reconciliation).
- Known: limiter transient inter-sample overshoot up to +0.6 dBFS at
  full-load sums (steady-state ceiling holds; see `limiter_test`).
- Repro: `cargo test -p mixer-wasm --release --test glitch_sim`
  (full load: `-- --ignored --nocapture`). Steady state is alloc-free —
  pinned by `alloc_test` (counting global allocator, 0 bytes/block).

## DSP modules

All verified with known-answer honesty tests (per AGENTS.md honesty rule):

| Module | Real DSP? | Tests | Wired into chain? |
|--------|-----------|-------|-------------------|
| Mixer (gain/pan/sum) | ✅ | parity + alloc pinned in fork; 9 native + 5 WASM here | ✅ engine-driven (`process_mix_rt`, 9 instances/block) |
| ParametricEq | ✅ (RBJ biquads) | 5 honesty + 3 integration + 4 six-band | ✅ staging layer |
| Compressor | ✅ | 16 honesty + 3 integration | ✅ staging layer + GR meter |
| Expander | ✅ | (in honesty suite) | ✅ staging layer |
| Gate | ✅ | (in honesty suite) | ✅ staging layer |
| OversampledLimiter | ✅ | 3 | ✅ master bus |
| gxfx insert FX — guitarix faust DSP, 11 effects (wah — a manual pedal wah, band set by HOTPOTZ, not envelope-driven — overdrive, distortion — bossds1, oversampled via gx_resampler/zita; its faust model drains onto a documented ~-130 dBFS control-voltage DC floor, not exact zero — compressor, chorus, flanger, phaser, tremolo, delay, echo, reverb) | ✅ | 11 known-answer honesty (9 in `gxfx_honesty_test` + rack-level delay impulse + console-level echo e2e) + unit/console suites | ✅ staging pass post-EQ, per-pair 11-slot chains |

## License

AGPL-3.0 (see `LICENSE`). The repo contains code derived from
[Eyevinn audio-mixer](https://github.com/Eyevinn/audio-mixer) (AGPL-3.0),
which sets the license for the combined work. Third-party components:

| Component | License |
|-----------|---------|
| `maxolgi/oximedia` (DSP engine fork) | Apache-2.0 |
| `vendor/QuicSRT-wasm` (submodule; itself carries libquicsrt + the srt fork as deps) | MPL-2.0 |
