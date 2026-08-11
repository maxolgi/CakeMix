# Plan — CakeMix WASM Audio Mixer

Execution plan for a standalone professional live-broadcast audio mixer
compiled to `wasm32-unknown-unknown`, with audio arriving/departing over
WebSRT. Built on `oximedia-mixer` (real engine) + WebSRT (transport).
Aspirational feature target: Blackmagic Fairlight Live (live broadcast
console, software-based, audio-only, multi-bus, scenes, talkback, etc.).

Read alongside: `AGENTS.md` (rules), `FINDINGS.md` (original investigation),
`IMPLEMENTATION_PLAN.md` (original milestone plan — superseded by this
document where they conflict).

---

## The honest picture (from research, supersedes FINDINGS.md where they disagree)

### Corrections to FINDINGS.md
- **`scirs2-core` is NOT the wasm risk.** The mixer never imports it
  (`rg scirs` → 0 hits across 64 files). It's a phantom dep — drop it.
  scirs2-core is itself wasm/no_std-capable anyway.
- **`rayon` is NOT already cfg-gated.** No `parallel` feature exists.
  `rayon` is an unconditional hard dep, used in exactly one file
  (`parallel_mix.rs`, one `.par_iter()`). `offline_bounce.rs` does NOT
  use rayon.
- **`oximedia-audio` is NOT wasm-clean by default.** It pulls `oxifft`
  with default `threading` (rayon) feature on — this is the real wasm
  blocker.

### The real wasm blocker
**`oxifft 0.3.2`** (transitive via `oximedia-audio`) ships
`default = ["std","threading"]` and `threading = ["rayon"]`. Feature
unification means we can't disable it from outside.

**Fix:** a one-line `[patch.crates.io]` pointing `oxifft` at a fork whose
only diff is `default = ["std"]`. Real-time audio is single-threaded by
design — we never want oxifft's threading on any target.

### Engine API essentials (full map to be captured in `ENGINE_API.md`)

Main type: `AudioMixer` (`lib.rs:348-379`). Constructor:
```rust
pub fn new(config: MixerConfig) -> Self          // lib.rs:394
```

Real-time entry:
```rust
pub fn process(&mut self, frame: &AudioFrame) -> MixerResult<AudioFrame>   // lib.rs:624
```
Internally delegates to:
```rust
pub fn process_mix(
    &mut self,
    channels: &[(ChannelId, ChannelProcessParams)],
    input_samples: &[f32],
) -> (Vec<f32>, Vec<f32>)   // (master_left, master_right)   // processing.rs:537
```

### ⚠️ The per-channel input gap (load-bearing)
The public `process()` feeds the **same** input buffer to **every**
channel. There is no per-channel input wiring — `Channel.input` exists
as metadata but is never read to select audio. For a 128-strip mixer
where each strip carries distinct WebSRT audio, this is the single
biggest gap between the crate's marketing and its behavior.

**Decision (b) — per-channel input setter** resolves this at the binding
layer, leaving `process_mix` untouched.

### Honesty check on the live DSP path — PASS (with caveats)
Traced `process()` → `process_mix()` end-to-end. Real DSP confirmed:
input gain/phase → effects chain → VCA×fader → stereo pan (4 laws) →
PDC delay → aux sends → bus routing → master accumulate. Numeric unit
tests assert real values.

**5 modules carry `#![allow(dead_code)]` and contain DSP not reachable
from `process()`:**
- `channel_strip.rs` — `StripEq::process` is a **fake EQ** (flat per-band
  gain, ignores `frequency`/`q`). Real biquad (`ParametricEqBand`) exists
  in the same dead module.
- `eq_band.rs`, `dynamics.rs`, `delay_line.rs`, `routing.rs` — real-looking
  DSP, never wired.
- **Metering is dead in the RT path** (`enable_metering` is a no-op).

Every one of these needs its own known-answer test before being trusted
(per the AGENTS.md honesty rule).

### Allocation concern (perf, not correctness)
`process_mix` allocates ~516 `Vec`s per block at 128 channels.
`buffer_pool` infrastructure exists but isn't wired into `process_mix`.
M4 concern, not M0.

### Environment (ready)
cargo/rustc 1.95, wasm-pack 0.15, `wasm32-unknown-unknown` target,
node 23.11, npm 11.4 — all present. `index.crates.io` returns 200
(crates.io website 403s on curl are cosmetic Cloudflare bot-block;
cargo itself unaffected).

### Reference patterns (SlopShady — read-only)
- Input tap: `streaming-input.ts:542` — WebCodecs `AudioData` arrives;
  replace `audioWriter.write(audioData)` with
  `audioData.allocationSize({format:'f32-planar', planeIndex})` →
  `Float32Array` → `audioData.copyTo(buf, {format, planeIndex})` →
  `postMessage({ts, data}, [buf.buffer])`.
- Output capture: `stream-audio-worklet.js` — planar `Float32Array(960*2)`
  (20ms @ 48k), transferred via `[buf.buffer]`. Buffers 128→960 because
  the Web Audio quantum doesn't divide 960.
- Publish pipeline: `stream-worker.js` — MessagePort → `AudioData` wrap →
  Opus `AudioEncoder` (48k/2ch/128kbps) → `chunk.copyTo` →
  `muxer.push_audio` → `muxer.poll` → `rx.sendMessage`. Wasm instantiated
  via wasm-bindgen-generated JS (parallel `init()` calls).
- **ts-muxer-wasm audio-only restriction is enforced in 3 places**
  (PCR updated only in `push_video`, PAT/PMT emitted only on video
  keyframe, `PCR_PID` hardcoded to video PID) — all in `lib.rs`. Localized
  but non-trivial. WebSRT-side edit, not CakeMix.
- **wasm-bindgen lesson:** the toy `mixer_wasm.rs` uses copy-heavy
  `&[f32]`/`Vec<f32>` (alloc per call both directions). The real binding
  uses `js_sys::Float32Array` views (`.copy_to` on input,
  `Float32Array::view` or caller-supplied buffer on output).

---

## Locked decisions

1. **Per-channel input design → (b) per-channel input setter.** Thin WASM
   adapter; engine's `process_mix` signature untouched. Matches SlopShady's
   discrete-arrival pattern. Simplifies the M0 known-answer test.
2. **Source path → crates.io + `[patch]` first**, not fork-and-strip.
   Preserves upstream tracking; escalate to selective fork only if patches
   grow beyond a few lines.
3. **Audio-only scope** (confirmed via Fairlight Live reference). Not a
   media mixer; no video DSP.
4. **Hybrid codecs** — WebSRT/WebCodecs handles Opus decode/encode;
   OxiMedia's value is the mixing DSP engine, not its codecs.

## Deferred decisions (not blocking M0)

- **WebSRT canonical path** — `/home/flibb/WebSRT` vs.
  `SlopShady/vendor/WebSRT`. Needed at M1.
- **Channel-count target for v1** — `MixerConfig.max_channels=128`
  default. Real ceiling measured at M4.
- **VST/AU substitute** — native Rust plugin library vs. WAM format vs.
  AudioWorklet chaining. Affects M4+ plugin work. (VST/AU hosting is
  impossible in WASM — must pick a substitute.)
- **Block-size policy on the live path** — M0 uses 128 throughout;
  128↔960 buffering is an M1 concern when wiring Opus publish.

---

## M0 — mixer runs in WASM, DSP proven

**Scope:** `crates/mixer-wasm/` compiles on `wasm32-unknown-unknown`;
`process()` runs with per-channel inputs; known-answer test green.
No UI, no network.

### Pre-implementation
1. **Write `ENGINE_API.md`** from the API-map research — verbatim
   signatures for `AudioMixer::new`, `process`, `add_channel`,
   `set_channel_gain/pan`, `add_bus`, `route_channel_to_*`,
   `add_aux_send`, `add_vca_group`; the dead-modules list; the
   per-channel-input gap note; the wasm-blocker summary. One source of
   truth so the binding work isn't guessing. **Verify whether
   `ProcessingEngine` is publicly exported** — this determines which
   integration path step 6 takes.

### Scaffold
2. `cargo new --lib` workspace + `crates/mixer-wasm/` (cdylib + rlib),
   `.gitignore`, `rust-toolchain.toml` (1.95), git init.
3. Wire deps:
   - `oximedia-mixer`, `oximedia-core` (with `wasm` feature),
     `oximedia-audio` (default-features = false) from crates.io.
   - `[patch.crates.io]` for `oxifft` → fork with `default = ["std"]`.
   - `[patch.crates.io]` for `oximedia-mixer` → fork with `rayon`
     optional + `parallel = ["dep:rayon"]` feature; `parallel_mix`
     gated behind it (sequential fallback already written).
   - Drop `scirs2-core` (phantom dep).
   - `getrandom = "0.4" features=["wasm_js"]`, `uuid` with `js`
     feature.
4. `cargo build --target wasm32-unknown-unknown` — **first gate**.
   If something nastier than rayon/oxifft surfaces, stop and report
   before escalating.

### Binding (`crates/mixer-wasm/src/lib.rs`)
5. `#[wasm_bindgen]` struct `MixerWasm` wrapping
   `oximedia_mixer::AudioMixer`. `#[wasm_bindgen(constructor)] fn
   new(sample_rate, block_size, max_channels) -> Result<MixerWasm,
   JsValue>`.
6. Per-channel input setter (decision b):
   `set_channel_input(ch: u32, data: &js_sys::Float32Array)`.
   Maintain `HashMap<ChannelId, Vec<f32>>` of pending per-channel
   input; on `process()`, push each channel's slice into the engine.
   **Integration point depends on step 1's finding:**
   - If `ProcessingEngine` is publicly exported → call `process_mix`
     directly with per-channel slices.
   - Else → thin adapter in our crate that loops per-channel, calls
     the engine's per-channel processing, sums manually.
   Use `Float32Array::copy_to` into a pre-allocated Rust buffer per
   channel — no per-call allocation on the steady-state path.
7. Channel add/remove, `set_channel_gain/pan/mute/solo` — thin
   wrappers over engine methods.
8. `process(block_size: u32) -> js_sys::Float32Array` — master stereo
   interleaved. Start with a fresh `Float32Array` per call (fine for
   M0; switch to `Float32Array::view` into internal buffer when wiring
   real-time AudioWorklet path in M1).
9. `console_error_panic_hook::set_once()` in the constructor.

### Known-answer test (`tests/known_answer.rs`)
10. Construct mixer at 48kHz, block 128, 2 channels.
11. Synthesize two sines (220Hz @ 0.5 linear, 330Hz @ 0.5 linear),
    128 samples each.
12. `set_channel_input(0, sine_a)`, `set_channel_input(1, sine_b)`,
    `process(128)`.
13. Direct-computed reference:
    `out[n] = 0.5*sin(2π·220·n/48000) + 0.5*sin(2π·330·n/48000)`.
14. Assert `|actual − reference| < 1e-5` for all n.
15. **Honesty gate:** if output is silence, all-zeros, or only one
    channel summed, STOP. Suspect fake-stub or wiring error;
    investigate before proceeding.

### Done criteria
- `wasm-pack test --node --release` green.
- `cargo build --target wasm32-unknown-unknown` clean.
- `ENGINE_API.md` committed.

---

## Milestone roadmap (after M0)

### M1 — single stereo program in/out (PCM audio-only end-to-end)
- [WebSRT] ts-muxer-wasm changes for PCM (see WEBSRT_CHANGES.md);
  bump pin when merged.
- Receive: WebSRT delivers MPEG2-TS → demux audio PES → raw PCM
  bytes → Float32Array → mixer `set_channel_input`.
- Publish: master Float32Array → raw PCM bytes → `ts-muxer-wasm`
  `push_audio` → SRT datagrams.
- No codec encode/decode step — PCM goes straight in/out of the TS muxer.
- 128↔960 buffering on the publish side (SlopShady's approach, minus Opus).
- **Verify:** publish a tone from a browser → mixer passes through →
  receive in a second browser, audio present at correct level.
  No video PID in the output TS.

### M2 — MPTS, one program with 8 stereo / 16 mono PIDs
- [WebSRT] MPTS mux + demux + multi-PID-per-program land; bump pin.
- Mixer input model: program → strips (1 mono PID → 1 strip; 1 stereo
  PID → 2 strips or 1 stereo strip per config).
- UI: multiple strips with independent gain/pan/mute/solo.
- **Verify:** source publishes a program with 8 stereo Opus PIDs;
  mixer shows 16 channels; solo/mute/pan each work; master sums
  correctly (known-answer: N unity-gain identical signals → +N× level
  pre-master).

### M3 — multi-session sum at master
- Per-session SRT `latency` config surfaced and documented.
- Mixer consumes multiple receivers' paced outputs into the master sum.
- Trivial bounded queue per input to absorb residual within-window skew
  (implementation detail, not a subsystem).
- **Verify:** two origins publish simultaneously with matched latency;
  master output is the correct sum; drop one session, confirm the other
  continues cleanly. Measure inter-session skew vs. configured latency.

### M4 — per-strip pro DSP (each module ships with its own known-answer test)
- Wire `eq_band::ParametricEq` (live, dead-code today). Extend to 6-band
  to match Fairlight. Honesty test: band gain at center freq.
- Wire `dynamics.rs` (comp/gate/expander/limiter). Honesty test:
  steady-state gain reduction, attack/release curves.
- Per-strip metering (peak/RMS/true-peak). The `enable_metering` no-op
  must become a real integration.
- Wire `limiter.rs` / `oversampled_limiter.rs` as the master bus
  finalizer (already live in `process()` if `limiter_enabled`).
- Measure real CPU at 128 strips in a browser tab — confirm v1 channel
  ceiling.

### M5 — broadcast bus architecture + scenes
- Full main/sub/aux/mix-minus/matrix with format conversion
  (mono↔stereo↔5.1↔ambisonics). Verify `BusType::Matrix` supports
  mix-minus cleanly.
- Scene recall with **cross-fade transitions** (Fairlight "smooth fades
  between snapshots"). Verify or extend `mix_scene`/`scene_recall`.
- Ambisonics bus (`ambisonics.rs`) — honesty-test first.

### M6 — live production subsystems
- Talkback (multi-group + ducking) — routing/ducking overlay using
  existing bus infrastructure.
- Cue player (16 audio + 16 MIDI cues via Web MIDI).
- Signal generators (oscillator/noise/tones/slate) — trivial DSP, new
  subsystem.
- Virtual soundcheck (record via File System Access API or IndexedDB,
  replay through input strips).

### M7 — control surfaces & AFV
- Web MIDI for Mackie MCU-style controllers.
- OSC over WebSocket for tablet apps.
- Audio Follows Video event hook (subscribe to a video switcher's
  camera-cut events; apply pre-configured audio reactions).
- Verify `midi_control.rs` honesty before reuse.

### M8 — reliability & multi-operator (large; may warrant its own design doc)
- On-Air mode (lock critical functions during live broadcast).
- Prep mode (offline show building, save/load show files).
- Failsafe mirror (two mixer instances with state sync).
- Remote control (multi-operator, assigned-channel scopes).

---

## Honest non-goals (out of scope for the WASM port)

- **VST/AU plugin hosting.** Impossible in WASM (native shared libs).
  Substitutes considered: native Rust plugin library (lean), WAM
  format, AudioWorklet chaining.
- **Hardware control surfaces (Fairlight Audio Panel 10/20/40).**
  Browser-native equivalents only (Web MIDI / WebHID / OSC).
- **"Thousands of channels."** Realistic browser-tab ceiling at
  single-threaded WASM is likely 128 (configurable up to ~256 with the
  buffer-pool rework). Don't promise Fairlight-scale channel density.
- **SMPTE-2110.** Replaced by WebSRT. PTP sync → matched SRT `latency`
  + playout buffer (protocol-level, not a mixer subsystem).

---

## Risk register (carried over + updated)

| Risk | Mitigation |
|------|------------|
| First wasm build surfaces nastier transitive issues | Stop and report; escalate to selective fork only if patches exceed a few lines. |
| A ported module is "compiles-but-fake" | Per-module known-answer tests (honesty rule). Investigate, don't paper over. |
| `ProcessingEngine` not publicly exported | Fallback: thin adapter that loops per-channel and sums manually (slightly more code, fine for M0). |
| Per-channel input setter hits engine-internal friction | Resolved at M0 step 1 (ENGINE_API.md); surface immediately if it changes the plan. |
| WebSRT audio-only / MPTS work is larger than expected | M1/M2 don't require all of it; sequence behind M0. Dummy-video fallback unblocks M1. |
| 128-strip decode load in one worker | Measure at M4; if decode-bound, split Opus decode across workers, postMessage PCM to the single mix worker. |
| ~516 Vec allocations per block at 128 channels | Wire `buffer_pool` into `process_mix` before M4. Engine has the infra; it's just not connected. |

---

## First concrete action

Write `ENGINE_API.md` from the API-map research (M0 step 1). That
single document resolves the one remaining implementation ambiguity
(whether `ProcessingEngine` is publicly exported, which determines the
step-6 integration path) and gives the binding work a verbatim source
of truth. Then scaffold the workspace and attempt the first wasm build.
