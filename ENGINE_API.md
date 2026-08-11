# ENGINE_API.md — Verbatim API map of `oximedia-mixer`

> Source of truth for the engine being ported into CakeMix. Every signature,
> struct, and code snippet below is quoted **verbatim** from the actual source
> at `/tmp/oximedia/crates/oximedia-mixer/src/` (oximedia v0.2.0). Line numbers
> are relative to that snapshot.
>
> Read alongside `AGENTS.md` (rules), `plan.md` (decisions), and `FINDINGS.md`
> (original investigation). Where this document and `FINDINGS.md` disagree,
> this document is authoritative — it was re-verified line-by-line.

---

## 0. The two types that matter

The engine has exactly two types on the real-time path:

| Type | File:line | Role |
|------|-----------|------|
| `AudioMixer` | `lib.rs:348` | Public top-level façade. Owns channels, buses, the `ProcessingEngine`, solo bus, automation player, and master limiters. |
| `ProcessingEngine` | `processing.rs:396` | The DSP graph. Holds per-channel effects, routing, aux sends, VCA groups, PDC delays, and bus accumulators. `process_mix()` is the one real-time DSP entry point. |

`ProcessingEngine` **is publicly exported** — both the module and the type:

```rust
/// DSP processing pipeline with bus routing, effects, sends, VCA, and PDC.
pub mod processing;                      // lib.rs:159
```
```rust
pub use processing::{                    // lib.rs:259-262
    ChannelOutputTarget, ChannelProcessParams, PanLawType, ProcessAuxSend, ProcessingEngine,
    RuntimeEffectSlot, RuntimeEffectsChain, VcaGroupState,
};
```

`ProcessingEngine` is a `pub struct` (`processing.rs:396`) and is reachable as
`oximedia_mixer::ProcessingEngine`. Its constructor `ProcessingEngine::new`
(`processing.rs:431`) and the DSP method `process_mix` (`processing.rs:538`)
are both `pub`.

---

## 1. VERBATIM SIGNATURES

All code below is copied unchanged from the source.

### 1.1 `AudioMixer` (`lib.rs`)

#### Struct definition (`lib.rs:348-379`)

```rust
/// Professional audio mixer.
pub struct AudioMixer {
    config: MixerConfig,
    channels: HashMap<ChannelId, Channel>,
    /// Maintains stable insertion-order index mapping for solo operations.
    channel_order: Vec<ChannelId>,
    buses: HashMap<BusId, Bus>,
    master_bus: Bus,
    session: MixerSession,
    sample_count: u64,
    /// Runtime processing engine for bus routing, effects, sends, VCA, and PDC.
    engine: ProcessingEngine,
    /// Solo bus for SIP, AFL, and PFL solo management.
    solo_bus: SoloBus,
    /// Automation player: renders per-block parameter values from automation lanes.
    automation_player: AutomationPlayer,
    /// Left-channel oversampled lookahead limiter for the master bus.
    master_limiter_l: oversampled_limiter::OversampledLimiter,
    /// Right-channel oversampled lookahead limiter for the master bus.
    master_limiter_r: oversampled_limiter::OversampledLimiter,
    /// Whether the master lookahead limiter is active.
    ///
    /// When `false` (default) the legacy `soft_clip()` is used to match the
    /// previous behaviour.  Enable via [`AudioMixer::set_limiter_enabled`].
    limiter_enabled: bool,
    /// Pool of pre-allocated `f32` buffers sized to `config.buffer_size`.
    ///
    /// Used by `extract_f32_samples` to avoid a fresh `Vec` allocation on
    /// every mix cycle.  The RAII [`PooledBuffer`] guard ensures each buffer
    /// is returned to the pool after the processing pass.
    sample_pool: buffer_pool::AudioBufferPool,
}
```

Note: `engine` is a private field, but `engine()` / `engine_mut()` expose it.

#### Constructor — `new()` (`lib.rs:394-437`)

```rust
    /// Create a new audio mixer.
    #[must_use]
    pub fn new(config: MixerConfig) -> Self {
        let master_bus = Bus::new(
            "Master".to_string(),
            BusType::Master,
            ChannelLayout::Stereo,
            config.sample_rate,
            config.buffer_size,
        );

        let engine = ProcessingEngine::new(config.buffer_size);

        // Build a stereo pair of oversampled lookahead limiters (default: −0.3 dBFS,
        // 50 ms release, 4× oversampling).  They are only exercised when
        // `limiter_enabled` is true.
        #[allow(clippy::cast_precision_loss)]
        let sample_rate_f32 = config.sample_rate as f32;
        let master_limiter_l =
            oversampled_limiter::OversampledLimiter::new(-0.3, 50.0, 4, sample_rate_f32);
        let master_limiter_r =
            oversampled_limiter::OversampledLimiter::new(-0.3, 50.0, 4, sample_rate_f32);

        // Pre-warm 4 buffers so the first few `process()` calls never need to
        // allocate.  Each buffer holds exactly `buffer_size` f32 samples.
        let sample_pool = buffer_pool::AudioBufferPool::new(config.buffer_size, 4);

        Self {
            config,
            channels: HashMap::new(),
            channel_order: Vec::new(),
            buses: HashMap::new(),
            master_bus,
            session: MixerSession::new(),
            sample_count: 0,
            engine,
            solo_bus: SoloBus::default(),
            automation_player: AutomationPlayer::new(),
            master_limiter_l,
            master_limiter_r,
            limiter_enabled: false,
            sample_pool,
        }
    }
```

#### Real-time entry — `process()` (`lib.rs:624`)

```rust
    pub fn process(&mut self, frame: &AudioFrame) -> MixerResult<AudioFrame> {
```
Returns `Result<AudioFrame, MixerError>`. Internally calls
`self.engine.process_mix(&channel_params, &pooled_input)` (see §1.2 and §2).

#### Channel management

```rust
    pub fn add_channel(                                      // lib.rs:521
        &mut self,
        name: String,
        channel_type: ChannelType,
        layout: ChannelLayout,
    ) -> MixerResult<ChannelId> {
```

```rust
    pub fn remove_channel(&mut self, id: ChannelId) -> MixerResult<()> {   // lib.rs:550
```
```rust
    pub fn get_channel(&self, id: ChannelId) -> MixerResult<&Channel> {    // lib.rs:563
```
```rust
    pub fn get_channel_mut(&mut self, id: ChannelId) -> MixerResult<&mut Channel> {  // lib.rs:574
```
```rust
    pub fn channels(&self) -> &HashMap<ChannelId, Channel> {               // lib.rs:582
```

#### Gain / pan — `set_channel_gain`, `set_channel_pan` (`lib.rs:591`, `lib.rs:602`)

```rust
    pub fn set_channel_gain(&mut self, id: ChannelId, gain: f32) -> MixerResult<()> {
        let channel = self.get_channel_mut(id)?;
        channel.set_gain(gain);
        Ok(())
    }
```
```rust
    pub fn set_channel_pan(&mut self, id: ChannelId, pan: f32) -> MixerResult<()> {
        let channel = self.get_channel_mut(id)?;
        channel.set_pan(pan);
        Ok(())
    }
```

#### Mute / solo — **naming correction**

There is **no** `set_channel_mute()` or `set_channel_solo()` convenience method
on `AudioMixer`. The actual API is:

- **Mute** is set via the channel accessor, e.g.
  `mixer.get_channel_mut(id)?.set_muted(bool)` — `Channel::set_muted`
  (`channel.rs:349`):
  ```rust
      pub fn set_muted(&mut self, muted: bool) {
          self.state.muted = muted;
      }
  ```
- **Solo** uses a separate bus-based API that takes an **insertion-order index
  (`usize`), not a `ChannelId`** (`lib.rs:929-980`):
  ```rust
      pub fn solo_channel(&mut self, channel_id: usize, mode: SoloMode) -> MixerResult<()> {
  ```
  ```rust
      pub fn unsolo_channel(&mut self, channel_id: usize) -> MixerResult<()> {
  ```
  ```rust
      pub fn is_soloed(&self, channel_id: usize) -> bool {
  ```
  ```rust
      pub fn set_solo_mode(&mut self, mode: SoloMode) {
  ```
  ```rust
      pub fn solo_mode(&self) -> SoloMode {
  ```
  ```rust
      pub fn any_channel_soloed(&self) -> bool {
  ```

  The index maps to `channel_order: Vec<ChannelId>` position; SIP muting is
  applied in `process()` (see §2).

#### Routing — `route_channel_to_bus`, `route_channel_to_master` (`lib.rs:784`, `lib.rs:806`)

```rust
    pub fn route_channel_to_bus(
        &mut self,
        channel_id: ChannelId,
        bus_id: BusId,
    ) -> MixerResult<()> {
```
```rust
    pub fn route_channel_to_master(&mut self, channel_id: ChannelId) -> MixerResult<()> {
```
Both write into `self.engine.channel_routing` (a `HashMap<ChannelId,
ChannelOutputTarget>`).

#### Aux send — `add_aux_send` (`lib.rs:822`)

```rust
    pub fn add_aux_send(
        &mut self,
        channel_id: ChannelId,
        bus_id: BusId,
        level: f32,
        pre_fader: bool,
    ) -> MixerResult<()> {
```
Pushes a `ProcessAuxSend { bus_id, level: level.clamp(0.0, 1.0), pre_fader, active: true }`
into `self.engine.channel_sends`.

#### VCA — `add_vca_group` (`lib.rs:886`)

```rust
    pub fn add_vca_group(&mut self, name: String, channels: Vec<ChannelId>) -> usize {
```
Returns the new group's **index** (pushes onto `self.engine.vca_groups`).
Related: `set_vca_gain(group_index: usize, gain: f32)` (`lib.rs:894`),
`set_vca_muted(group_index: usize, muted: bool)` (`lib.rs:901`).

#### Bus — `add_bus` (`lib.rs:1062`)

```rust
    pub fn add_bus(
        &mut self,
        name: String,
        bus_type: BusType,
        layout: ChannelLayout,
    ) -> MixerResult<BusId> {
```
Calls `self.engine.register_bus(id, bus_type)` to create a `BusAccumulator`.

#### Engine accessors — `engine()`, `engine_mut()` (`lib.rs:768`, `lib.rs:774`)

```rust
    /// Get a reference to the processing engine.
    #[must_use]
    pub fn engine(&self) -> &ProcessingEngine {
        &self.engine
    }

    /// Get a mutable reference to the processing engine.
    #[must_use]
    pub fn engine_mut(&mut self) -> &mut ProcessingEngine {
        &mut self.engine
    }
```

#### Parallel (offline) path — `process_parallel` (`lib.rs:731`)

```rust
    pub fn process_parallel(&mut self, block_size: usize) -> MixerResult<Vec<f32>> {
```
> Rayon-based fast path (channel-strip only; no aux/bus/PDC/VCA). Uses
> `parallel_mix.rs` → the **only** file in the crate that imports `rayon`.
> Not used on the real-time WASM path.

---

### 1.2 `ProcessingEngine` (`processing.rs`)

#### Struct definition (`processing.rs:396-426`)

```rust
pub struct ProcessingEngine {
    /// Buffer size in samples.
    buffer_size: usize,

    /// Per-channel runtime effects chains.
    pub channel_effects: HashMap<ChannelId, RuntimeEffectsChain>,

    /// Per-bus runtime effects chains (for group / aux buses).
    pub bus_effects: HashMap<BusId, RuntimeEffectsChain>,

    /// Channel output routing (which bus each channel feeds).
    pub channel_routing: HashMap<ChannelId, ChannelOutputTarget>,

    /// Per-channel aux sends.
    pub channel_sends: HashMap<ChannelId, Vec<ProcessAuxSend>>,

    /// VCA groups.
    pub vca_groups: Vec<VcaGroupState>,

    /// PDC delay lines (one per channel).
    pub pdc_delays: HashMap<ChannelId, PdcDelayLine>,

    /// Maximum PDC latency across all channels (recomputed on demand).
    max_latency: usize,

    /// Bus accumulators keyed by BusId (group and aux buses).
    bus_accumulators: HashMap<BusId, BusAccumulator>,

    /// Which buses are group buses vs aux buses.
    pub bus_types: HashMap<BusId, BusType>,
}
```

Most fields are `pub` (mutable from `engine_mut()`). `buffer_size`,
`max_latency`, and `bus_accumulators` are private.

#### Constructor (`processing.rs:431`)

```rust
    /// Create a new processing engine for the given buffer size.
    #[must_use]
    pub fn new(buffer_size: usize) -> Self {
```

#### DSP entry — `process_mix()` (`processing.rs:538-542`) — **verbatim signature**

```rust
    pub fn process_mix(
        &mut self,
        channels: &[(ChannelId, ChannelProcessParams)],
        input_samples: &[f32],
    ) -> (Vec<f32>, Vec<f32>) {
```

Returns `(master_left, master_right)`, each a `Vec<f32>` of length
`buffer_size`. See §2 for how `input_samples` is used (the per-channel gap).

#### Other `pub` methods on `ProcessingEngine` (`processing.rs`)

```rust
    pub fn register_bus(&mut self, bus_id: BusId, bus_type: BusType)              // :447
    pub fn set_bus_gain(&mut self, bus_id: BusId, gain: f32)                      // :455
    pub fn set_bus_muted(&mut self, bus_id: BusId, muted: bool)                   // :462
    pub fn recompute_pdc(&mut self)                                               // :472
    pub fn max_latency(&self) -> usize                                            // :495
    pub fn vca_gain_for_channel(&self, channel_id: ChannelId) -> f32              // :504
    pub fn process_channel_effects(&mut self, channel_id: ChannelId, working: &mut [f32])  // :520
```

---

### 1.3 Supporting types (verbatim)

#### `ChannelProcessParams` (`processing.rs:723-738`)

```rust
/// Parameters for processing a single channel through the mix pipeline.
#[derive(Debug, Clone)]
pub struct ChannelProcessParams {
    /// Fader gain (linear, 0.0–2.0).
    pub fader_gain: f32,
    /// Pan position (-1.0 to 1.0).
    pub pan: f32,
    /// Whether the channel is muted.
    pub muted: bool,
    /// Input gain in dB.
    pub input_gain_db: f32,
    /// Phase inverted.
    pub phase_inverted: bool,
    /// Pan law to apply.
    pub pan_law: PanLawType,
}
```

```rust
/// Pan law selection for the processing engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanLawType {                 // processing.rs:742
    Linear,
    Minus3dB,
    Minus4Dot5dB,
    Minus6dB,
}
impl Default for PanLawType { fn default() -> Self { Self::Minus3dB } }   // :753
```

#### `ChannelOutputTarget` (`processing.rs:177-188`)

```rust
pub enum ChannelOutputTarget {
    /// Route directly to master bus (default).
    Master,
    /// Route to a group bus.
    GroupBus(BusId),
}
impl Default for ChannelOutputTarget { fn default() -> Self { Self::Master } }
```

#### `ProcessAuxSend` (`processing.rs:190-201`)

```rust
/// Aux send descriptor used during processing.
#[derive(Debug, Clone)]
pub struct ProcessAuxSend {
    /// Target aux bus ID.
    pub bus_id: BusId,
    /// Send level (linear, 0.0–1.0).
    pub level: f32,
    /// Whether the send taps pre-fader.
    pub pre_fader: bool,
    /// Whether the send is active.
    pub active: bool,
}
```

#### `VcaGroupState` (`processing.rs:207-218`)

```rust
/// A VCA group that applies multiplicative gain to linked channels.
#[derive(Debug, Clone)]
pub struct VcaGroupState {
    /// Group name.
    pub name: String,
    /// VCA fader gain (linear, 0.0–2.0).
    pub gain: f32,
    /// Whether the VCA group is muted.
    pub muted: bool,
    /// Channel IDs belonging to this VCA group.
    pub channels: Vec<ChannelId>,
}
```

#### `ChannelId` (`channel.rs:13-14`)

```rust
/// Unique channel identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ChannelId(pub Uuid);
```
Newtype over `uuid::Uuid` (v4 generated in `add_channel`).

#### `ChannelType` (`channel.rs:22-55`)

```rust
/// Channel type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelType {
    Mono,
    Stereo,
    Surround51,
    Surround71,
    AmbisonicsFirstOrder,
    AmbisonicsSecondOrder,
    AmbisonicsThirdOrder,
}
impl ChannelType {
    pub fn channel_count(&self) -> usize { /* Mono→1, Stereo→2, 5.1→6, 7.1→8, … */ }
}
```

> **Note:** `ChannelType` is metadata only. `process_mix` treats every channel
> as a mono source and pans it into a stereo master (see §2). The
> `channel_count()` is never consulted on the live path.

#### `Channel` (`channel.rs:203-261`) — full struct

```rust
/// Mixer channel (track or bus).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    name: String,
    #[allow(clippy::struct_field_names)]
    channel_type: ChannelType,
    #[serde(skip)]
    layout: ChannelLayout,
    sample_rate: u32,
    buffer_size: usize,
    state: ChannelState,
    gain: f32,
    pan: f32,
    pan_mode: PanMode,
    pan_law: PanLaw,
    link: ChannelLink,
    effects: Vec<EffectSlot>,
    sends: HashMap<usize, SendConfig>,
    input: InputRouting,          // ← metadata; see §2
    output: Option<OutputRouting>,
    direct_monitoring: DirectMonitoring,
    color: Option<String>,
    icon: Option<String>,
}
```

Constructor (`channel.rs:266`) and the accessors used by `process()`'s
param-builder:

```rust
    pub fn new(
        name: String,
        channel_type: ChannelType,
        layout: ChannelLayout,
        sample_rate: u32,
        buffer_size: usize,
    ) -> Self
```
```rust
    pub fn gain(&self) -> f32                       // channel.rs:388
    pub fn pan(&self) -> f32                        // channel.rs:418
    pub fn is_muted(&self) -> bool                  // channel.rs:344
    pub fn is_phase_inverted(&self) -> bool         // channel.rs:377
    pub fn pan_law(&self) -> PanLaw                 // channel.rs:440
    pub fn input(&self) -> &InputRouting            // channel.rs:528
```

`InputRouting` (`channel.rs:137-147`) — the metadata that is **read but only
for `gain_db`** on the live path:

```rust
pub struct InputRouting {
    pub source: InputSource,        // Hardware(u32) | Bus(BusId) | Virtual | None  — NEVER read by process_mix
    pub gain_db: f32,               // ← the only field used on the live path
    pub highpass_enabled: bool,     // ← never applied on the live path
    pub highpass_freq: f32,         // ← never applied on the live path
}
```

#### `MixerConfig` (`lib.rs:305-346`)

```rust
/// Mixer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MixerConfig {
    pub sample_rate: u32,
    pub buffer_size: usize,
    pub max_channels: usize,
    pub max_buses: usize,
    pub max_effects_per_channel: usize,
    pub enable_automation: bool,
    pub enable_metering: bool,
    pub metering_rate: u32,
}

impl Default for MixerConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            buffer_size: 512,
            max_channels: 128,
            max_buses: 32,
            max_effects_per_channel: 8,
            enable_automation: true,
            enable_metering: true,
            metering_rate: 30,
        }
    }
}
```

#### `BusType` (`bus.rs:23-34`) and `BusId` (`bus.rs:14-15`)

```rust
/// Bus type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BusType {
    Master,
    Group,
    Auxiliary,
    Matrix,
}
```
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct BusId(pub Uuid);
```

#### `AudioEffect` trait (`effects_chain.rs:8-14`) — the effects-chain interface

The runtime effects chain (`RuntimeEffectSlot`, `RuntimeEffectsChain`) holds
boxed `dyn AudioEffect`, **not** the `dynamics.rs`/`eq_band.rs` structs (see §3):

```rust
/// A single audio effect that can process a mono sample buffer in-place.
pub trait AudioEffect: Send + Sync {
    /// Process the sample buffer in-place.
    fn process(&mut self, samples: &mut [f32]);
    /// Human-readable effect name.
    fn name(&self) -> &str;
}
```

---

## 2. ⚠️ THE PER-CHANNEL INPUT GAP (load-bearing)

`AudioMixer::process()` extracts **one** `&[f32]` from the entire input frame
and feeds that **same** slice to **every** channel. There is no per-channel
input wiring. For a 128-strip live mixer where each strip carries distinct
WebSRT audio, this is the single biggest gap between the crate's marketing and
its runtime behavior.

### Proof (3 steps, verbatim)

**Step 1 — one buffer is extracted from the frame** (`lib.rs:635`):

```rust
        let pooled_input = extract_f32_samples(frame, buffer_size, &self.sample_pool);
```

`extract_f32_samples` (`lib.rs:1119-1164`) decodes the raw interleaved/planar
bytes of the **whole** `AudioFrame` into a single contiguous `PooledBuffer`
(`Deref<Target = [f32]>`). It is called exactly once per `process()` call.

**Step 2 — that single buffer is passed, unmodified, to the engine**
(`lib.rs:674-675`):

```rust
        let (mut master_left, mut master_right) =
            self.engine.process_mix(&channel_params, &pooled_input);
```

`channel_params` carries per-channel **gain/pan/mute** state, but **no audio
data** — `ChannelProcessParams` has no audio field (see §1.3). The audio is the
single `&pooled_input`.

**Step 3 — inside `process_mix`, every channel reads the same `input_samples`**
(`processing.rs:562-567`, inside `for (channel_id, params) in channels { … }`):

```rust
            let mut working: Vec<f32> = input_samples
                .iter()
                .take(bs)
                .map(|&s| s * input_gain_linear * phase_mult)
                .collect();
            working.resize(bs, 0.0);
```

`input_samples` is the function parameter `input_samples: &[f32]` — the same
slice for all N channels. Each channel only scales it by its own
`input_gain_db`/phase; the **source samples are identical across channels**.

### `Channel.input` exists but does not select audio

`Channel.input: InputRouting` (`channel.rs:248`) carries an `InputSource`
(`Hardware(u32)` / `Bus(BusId)` / `Virtual` / `None`, `channel.rs:162-171`). On
the live path, `process()` reads **only** `ch.input().gain_db`
(`lib.rs:664`, `input_gain_db: ch.input().gain_db`) to build
`ChannelProcessParams`. The `InputSource` discriminant is **never inspected** to
route distinct audio to distinct channels — nothing maps `InputSource::Hardware(n)`
to a per-strip audio buffer. `highpass_enabled`/`highpass_freq` are likewise
never applied in `process_mix`.

### What this means for CakeMix

`process_mix` cannot be used as-is for a multi-source mixer via `AudioMixer::process()`.
The resolution is the **integration-path decision** in §5: bypass
`AudioMixer::process()` and call `ProcessingEngine::process_mix()` (or a thin
per-channel variant) directly with distinct per-channel slices. This is possible
because `ProcessingEngine` is `pub` and `AudioMixer::engine_mut()` exposes it.

---

## 3. DEAD MODULES LIST (the `#![allow(dead_code)]` five)

Five modules carry `#![allow(dead_code)]` (or item-level `#[allow(dead_code)]`)
and contain DSP that is **not reachable from `process()` / `process_mix()`**.
Per the AGENTS.md honesty rule, each must pass its own known-answer test before
being trusted — "compiles and is `pub`" is not evidence it runs.

The **live** real-time path is: `lib.rs::AudioMixer` → `processing.rs::ProcessingEngine::process_mix`.
That path uses the `AudioEffect` trait (`effects_chain.rs`), `ChannelProcessParams`,
pan laws, VCA, PDC, and bus accumulators — **none** of which come from the five
modules below.

### 3.1 `channel_strip.rs` — whole-module `#![allow(dead_code)]` (line 1)

A standalone `ChannelStrip` struct modeling an analog console strip
(input-gain → HPF → EQ → dynamics → fader → pan). **Not constructed by
`AudioMixer` or `process_mix`.**

- **Dead:** `StripEq::process` (`channel_strip.rs:155-171`) is a **fake EQ**.
  It does **not** filter — it multiplies the whole-band sample by a flat linear
  gain derived from each band's `gain_db`, ignoring `frequency` and `q`
  entirely:
  ```rust
      pub fn process(&self, input: f32) -> f32 {
          if !self.enabled { return input; }
          let mut output = input;
          for band in &[&self.low, &self.low_mid, &self.high_mid, &self.high] {
              if band.enabled && band.gain_db.abs() > 0.01 {
                  let gain_linear = 10.0_f32.powf(band.gain_db / 20.0);
                  output *= gain_linear;
              }
          }
          output
      }
  ```
  A real biquad `ParametricEqBand` exists in the same module (uses
  `eq_band::BiquadCoeffs`) but is also not wired into the live path.
- **Dead:** `StripDynamics` (compressor+gate), `HighPassFilter`, the full
  `ChannelStrip::process_sample` chain.
- **Live:** none on the RT path. The only external caller is
  `parallel_mix::process_channel_strip` (the rayon offline fast path, §1.1),
  which is itself not on the WASM real-time path. Also exercised by unit tests
  in `pan_matrix.rs`.

### 3.2 `eq_band.rs` — whole-module `#![allow(dead_code)]` (line 1)

Real biquad-based parametric EQ: `EqFilterType`, `BiquadCoeffs` (Direct Form I),
`ParametricEqBand` with Audio EQ Cookbook coefficient math.

- **Dead:** the entire module on the RT path. `ParametricEqBand` is consumed
  only by `channel_strip.rs` (§3.1, itself dead on the RT path).
- **Live RT path uses a different mechanism:** the effects chain is trait-based
  (`Box<dyn AudioEffect>`, `effects_chain.rs:8`), holding its own effect types.
  `effects.rs` defines a separate `EqBand`/`EqBandType`/`ParametricEqParams`
  (`effects.rs:399`, `:428`, `:377`) plus many other `Effect` variants — these are
  the EQ parameter types on the engine's own effects surface, **not**
  `eq_band::ParametricEqBand`. The two EQ implementations are independent.

### 3.3 `dynamics.rs` — **item-level** `#[allow(dead_code)]` (lines 5, 12)

> Correction to the task brief: `dynamics.rs` does **not** carry a whole-module
> `#![allow(dead_code)]`. Only two free functions are individually annotated:
> ```rust
>     #[allow(dead_code)]
>     pub fn db_to_linear(db: f32) -> f32 { 10.0_f32.powf(db / 20.0) }   // :5
>     #[allow(dead_code)]
>     pub fn linear_to_db(x: f32) -> f32 { … }                          // :12
> ```

Contains `Compressor`/`CompressorConfig`, `Expander`, `Gate`, `Limiter` (all
`pub struct` with `process_sample(&mut self, sample: f32, sample_rate: u32) -> f32`).

- **Dead:** `Compressor`, `Expander`, `Gate`, and this module's own `Limiter`/
  `LimiterConfig` are **not referenced anywhere** outside `dynamics.rs` (no
  external `use`, no construction in `process_mix` or `effects.rs`). The
  `Limiter::new` hits in the codebase are `limiter.rs::Limiter` and
  `oversampled_limiter.rs::OversampledLimiter` — different types.
- **Dead:** `db_to_linear`/`linear_to_db` here (note: `processing.rs` has its
  **own** `db_to_linear` (`processing.rs:766`), which is the one `process_mix`
  actually calls).
- **Live:** nothing from this module is on the RT path. RT dynamics/limiting is
  done via `Box<dyn AudioEffect>` plugin slots and the master
  `OversampledLimiter` (`oversampled_limiter.rs`), driven from `lib.rs`.

### 3.4 `delay_line.rs` — whole-module `#![allow(dead_code)]` (line 1)

Configurable delay line (`DelayLine`, `DelayLineConfig`, fractional-sample
interpolation `InterpolationMode`, feedback, modulation for chorus/flanger).

- **Dead:** the entire module. `delay_line` is referenced **only** by its own
  `pub mod` declaration in `lib.rs:128` — zero external `use` statements.
- **Live:** the PDC delay lines used in `process_mix` are `PdcDelayLine`
  (defined privately inside `processing.rs:252`), **not** `delay_line::DelayLine`.

### 3.5 `routing.rs` — **item-level** `#[allow(dead_code)]` (8 sites)

> Correction to the task brief: `routing.rs` does **not** carry a whole-module
> `#![allow(dead_code)]`. The annotation is per-item: on the `RoutingMatrix`
> struct (`routing.rs:9`) and on 7 of its methods (`:23, :36, :47, :64, :96,
> :116, :165`).

A gain-based `RoutingMatrix { inputs, outputs, connections: Vec<Vec<f32>> }`
with preset configs and validation.

- **Dead:** `RoutingMatrix` and all its methods. Nothing in the engine
  constructs it. The `routing` string hits across the codebase are comments or
  **different** types (`BusRouting`, `ChannelOutputTarget`,
  `mix_bus::MixMatrix`, etc.).
- **Live:** the RT path routes channels via
  `ProcessingEngine::channel_routing: HashMap<ChannelId, ChannelOutputTarget>`
  (`processing.rs:407`) and `ChannelOutputTarget::{Master, GroupBus(BusId)}`
  (`processing.rs:177`) — **not** via `routing::RoutingMatrix`.

> **Additional dead-path caveat (not one of the five):** `MixerConfig::enable_metering`
> / `metering_rate` are **no-ops** on the RT path. `process()` does not update
> any meter; the `metering.rs`/`analysis_meter.rs` types are not exercised by
> `process_mix`. Metering is metadata-only in the live path.

---

## 4. THE WASM BLOCKER SUMMARY

Target is `wasm32-unknown-unknown`, single-threaded, **no** `SharedArrayBuffer`
/ COOP-COEP (per AGENTS.md). Two dependencies pull in `rayon`, which requires OS
threads and will not compile on that target.

### Blocker 1 — `rayon` is an **unconditional** dep of `oximedia-mixer`

`crates/oximedia-mixer/Cargo.toml`:

```toml
[features]
default = []

[dependencies]
oximedia-core.workspace = true
oximedia-audio.workspace = true
scirs2-core.workspace = true
rayon.workspace = true          # ← unconditional; no feature gate
serde = { workspace = true }
serde_json = { workspace = true }
uuid = { workspace = true }
thiserror.workspace = true
bytes.workspace = true
```

There is **no** `parallel` feature; `rayon` is always compiled. It is used in
exactly one file:

```rust
// parallel_mix.rs:16
use rayon::prelude::*;
// parallel_mix.rs:134
        .par_iter()
```

(`offline_bounce.rs` does **not** use rayon — the only `use rayon` /
`par_iter` in the crate is `parallel_mix.rs`.)

**Patch:** fork `oximedia-mixer` (or `[patch.crates.io]` it) and make rayon
optional + feature-gated:
```toml
parallel = ["dep:rayon"]
rayon = { workspace = true, optional = true }
```
and cfg-gate `parallel_mix.rs` behind `#[cfg(feature = "parallel")]` (and the
`pub mod parallel_mix;` in `lib.rs:162`). Real-time audio is single-threaded by
design (Web Audio's 128-frame render quantum) — no audio-thread parallelism is
lost. Pin the fork in CakeMix's `Cargo.toml` via `[patch.crates.io]`.

### Blocker 2 — `oxifft` ships `threading` (= `rayon`) in its **default** features

`oximedia-mixer` does **not** depend on `oxifft` directly, but its dep
`oximedia-audio` does, unconditionally:

`crates/oximedia-audio/Cargo.toml`:
```toml
[dependencies]
oximedia-core.workspace = true
bytes.workspace = true
thiserror.workspace = true
oxifft.workspace = true      # ← unconditional; pulls oxifft default features
```

`oxifft 0.3.2` (verified from the registry `Cargo.toml`) has:
```toml
[features]
default = ["std", "threading"]
threading = ["std", "dep:rayon"]
wasm = ["dep:wasm-bindgen", "dep:js-sys"]
```

Because `default` includes `threading` and `threading` pulls `dep:rayon`, and
because Cargo **feature-unifies** the default, an external crate cannot turn
`threading` off — `oximedia-audio` asks for the defaults, so rayon is always
enabled transitively. This is the **real** wasm blocker (the deeper one; blocker
1 above is the direct one).

**Patch:** a one-line `[patch.crates.io]` pointing `oxifft` at a fork whose
only diff is `default = ["std"]` (drop `"threading"`). Real-time audio never
wants oxifft's threading on any target. The mixer's own `spectrum_analyzer.rs`
uses a **pure-Rust** Cooley-Tukey FFT and does not call oxifft at all, so
disabling oxifft threading loses nothing the mixer uses.

### Combined patch posture

```toml
# CakeMix Cargo.toml (workspace)
[patch.crates-io]
oxifft         = { git = "<fork>", … }   # default = ["std"] only
oximedia-mixer = { git = "<fork>", … }   # rayon optional + parallel feature
```
Escalate to a fuller fork only if either patch grows beyond a few lines
(per `plan.md` locked decision #2).

### (Non-blocker) other wasm adjustments

- `scirs2-core` is a phantom dep on the RT path (`rg scirs` → 0 hits in the
  mixer) and is itself wasm/no_std-capable — not a blocker.
- `oximedia-core` needs its `wasm` feature; `getrandom 0.4` needs
  `features = ["wasm_js"]`; `uuid` needs `js` (already in the workspace).

---

## 5. INTEGRATION PATH DECISION

**We can call `ProcessingEngine::process_mix()` directly with per-channel
slices — and we must, because `AudioMixer::process()` cannot.**

This works because:

1. `ProcessingEngine` **is** `pub`-exported as `oximedia_mixer::ProcessingEngine`
   (`lib.rs:159`, `lib.rs:259-262`, struct at `processing.rs:396`).
2. `AudioMixer::engine_mut(&mut self) -> &mut ProcessingEngine` (`lib.rs:774`)
   hands out a mutable reference to it, so a binding can populate
   `channel_routing`, `channel_sends`, `vca_groups`, `channel_effects`, etc.
   directly.
3. `process_mix(&mut self, channels: &[(ChannelId, ChannelProcessParams)],
   input_samples: &[f32]) -> (Vec<f32>, Vec<f32>)` (`processing.rs:538`) is
   `pub` and is the single real-time DSP entry point.

**The problem** (§2): `process_mix` takes **one** `input_samples: &[f32]` and
reads it for **every** channel. It has no per-channel audio input. The
`AudioMixer::process()` wrapper does not fix this — it derives that single slice
from the whole frame. So a naive `AudioMixer::process(frame)` call mixes N copies
of the same audio, not N distinct sources.

**The resolution — locked decision (b) in `plan.md`: per-channel input at the
binding layer.** The CakeMix WASM binding (`crates/mixer-wasm/`) keeps a
per-channel `Vec<f32>` input store (a `HashMap<ChannelId, Vec<f32>>` or a slab),
populated by per-strip setters (each WebSRT strip writes its own buffer). Each
render quantum, the binding either:

- **(Option A — call per-channel, leave `process_mix` untouched):** loop over
  channels, and for each, build a `ChannelProcessParams` and run the channel
  DSP on **that channel's own** slice, accumulating into the master. This keeps
  the upstream engine signature unchanged (pure `[patch]` for wasm, no API
  fork). The simplest correct form reproduces `process_mix`'s per-channel loop
  (steps 1–7: input-gain/phase → effects → VCA×fader → pan → PDC → sends →
  routing) but with `input_samples` indexed per channel.
- **(Option B — fork a per-channel-input `process_mix`):** add a variant that
  takes `&[(ChannelId, ChannelProcessParams, &[f32])]` (per-channel audio) in
  the engine fork. Cleaner caller code, but it is an API change to the engine.

Decision (b) / Option A is the M0 path: `process_mix`'s signature stays
untouched, the engine fork is wasm-only (`rayon` gating + `oxifft` patch), and
the per-channel-input logic lives entirely in the CakeMix binding. This matches
SlopShady's discrete-arrival pattern (one `Float32Array` per strip) and makes
the M0 known-answer test (two distinct sine tones → correct stereo mix)
straightforward.

**What we keep from the engine as-is:** the DSP primitives are real and
verified end-to-end (`plan.md` honesty check): input-gain/phase, the
`AudioEffect` effects chain, VCA×fader, the four stereo pan laws, PDC delay,
aux sends, and bus routing/accumulation all run real numeric math in
`process_mix`. The binding reuses `ProcessingEngine` for all of that; it only
replaces the **input fan-out** that `AudioMixer::process()` gets wrong.
