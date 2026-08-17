# ENGINE_API.md — Verbatim API map of `oximedia-mixer`

> Source of truth for the engine being ported into CakeMix. Every signature,
> struct, and code snippet below is quoted **verbatim** from the actual source
> at `/home/flibb/oximedia/crates/oximedia-mixer/src/` — the CakeMix fork
> (`maxolgi/oximedia`, upstream 0.2.1 merge base `aa9e68af` plus the fork
> commits listed in §6, HEAD `3283cb3f`, **not yet pushed**). Line numbers
> are relative to that fork checkout.
>
> Read alongside `AGENTS.md` (rules), `plan.md` (decisions), and `FINDINGS.md`
> (original investigation). Where this document and `FINDINGS.md` disagree,
> this document is authoritative — it was re-verified line-by-line.

---

## 0. The two types that matter

The engine has exactly two types on the real-time path:

| Type | File:line | Role |
|------|-----------|------|
| `AudioMixer` | `lib.rs:351` | Public top-level façade. Owns channels, buses, the `ProcessingEngine`, solo bus, automation player, and master limiters. |
| `ProcessingEngine` | `processing.rs:396` | The DSP graph. Holds per-channel effects, routing, aux sends, VCA groups, PDC delays, and bus accumulators. `process_mix()` is the buffered entry point; the fork adds `process_mix_rt()` (`processing.rs:765`), the real-time multi-source entry point CakeMix drives per block (§6). |

`ProcessingEngine` **is publicly exported** — both the module and the type:

```rust
/// DSP processing pipeline with bus routing, effects, sends, VCA, and PDC.
pub mod processing;                      // lib.rs:159
```
```rust
pub use processing::{                    // lib.rs:261-263
    ChannelOutputTarget, ChannelProcessParams, PanLawType, ProcessAuxSend, ProcessingEngine,
    RuntimeEffectSlot, RuntimeEffectsChain, VcaGroupState,
};
```

`ProcessingEngine` is a `pub struct` (`processing.rs:396`) and is reachable as
`oximedia_mixer::ProcessingEngine`. Its constructor `ProcessingEngine::new`
(`processing.rs:446`) and the DSP methods `process_mix` (`processing.rs:559`)
and `process_mix_rt` (`processing.rs:765`, fork) are all `pub`.

---

## 1. VERBATIM SIGNATURES

All code below is copied unchanged from the source.

### 1.1 `AudioMixer` (`lib.rs`)

#### Struct definition (`lib.rs:351-381`)

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

#### Constructor — `new()` (`lib.rs:398-439`)

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

#### Real-time entry — `process()` (`lib.rs:626`)

```rust
    pub fn process(&mut self, frame: &AudioFrame) -> MixerResult<AudioFrame> {
```
Returns `Result<AudioFrame, MixerError>`. Internally calls
`self.engine.process_mix(&channel_params, &pooled_input)` (see §1.2 and §2).

#### Channel management

```rust
    pub fn add_channel(                                      // lib.rs:523
        &mut self,
        name: String,
        channel_type: ChannelType,
        layout: ChannelLayout,
    ) -> MixerResult<ChannelId> {
```

```rust
    pub fn remove_channel(&mut self, id: ChannelId) -> MixerResult<()> {   // lib.rs:552
```
```rust
    pub fn get_channel(&self, id: ChannelId) -> MixerResult<&Channel> {    // lib.rs:565
```
```rust
    pub fn get_channel_mut(&mut self, id: ChannelId) -> MixerResult<&mut Channel> {  // lib.rs:576
```
```rust
    pub fn channels(&self) -> &HashMap<ChannelId, Channel> {               // lib.rs:584
```

#### Gain / pan — `set_channel_gain`, `set_channel_pan` (`lib.rs:593`, `lib.rs:604`)

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
  (`channel.rs:358`):
  ```rust
      pub fn set_muted(&mut self, muted: bool) {
          self.state.muted = muted;
      }
  ```
- **Solo** uses a separate bus-based API that takes an **insertion-order index
  (`usize`), not a `ChannelId`** (`lib.rs:932-987`):
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

#### Routing — `route_channel_to_bus`, `route_channel_to_master` (`lib.rs:787`, `lib.rs:809`)

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

#### Aux send — `add_aux_send` (`lib.rs:825`)

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

#### VCA — `add_vca_group` (`lib.rs:889`)

```rust
    pub fn add_vca_group(&mut self, name: String, channels: Vec<ChannelId>) -> usize {
```
Returns the new group's **index** (pushes onto `self.engine.vca_groups`).
Related: `set_vca_gain(group_index: usize, gain: f32)` (`lib.rs:897`),
`set_vca_muted(group_index: usize, muted: bool)` (`lib.rs:904`).

#### Bus — `add_bus` (`lib.rs:1065`)

```rust
    pub fn add_bus(
        &mut self,
        name: String,
        bus_type: BusType,
        layout: ChannelLayout,
    ) -> MixerResult<BusId> {
```
Calls `self.engine.register_bus(id, bus_type)` to create a `BusAccumulator`.

#### Engine accessors — `engine()`, `engine_mut()` (`lib.rs:771`, `lib.rs:777`)

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

#### Parallel (offline) path — `process_parallel` (`lib.rs:734`)

```rust
    pub fn process_parallel(&mut self, block_size: usize) -> MixerResult<Vec<f32>> {
```
> Rayon-based fast path (channel-strip only; no aux/bus/PDC/VCA). Uses
> `parallel_mix.rs` → the **only** file in the crate that imports `rayon`.
> Not used on the real-time WASM path. **In the fork** the whole module is
> cfg-gated: `#[cfg(feature = "parallel")] pub mod parallel_mix;`
> (`lib.rs:163`) — the `parallel` feature is off for wasm builds (§4).

---

### 1.2 `ProcessingEngine` (`processing.rs`)

#### Struct definition (`processing.rs:396-441`)

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

    // Pre-allocated scratch for [`Self::process_mix_rt`] so the real-time
    // path performs zero steady-state allocation. Sized to `buffer_size` at
    // construction and never resized.
    /// Per-channel working buffer (input gain/phase → effects → fader × VCA).
    rt_working: Vec<f32>,
    /// Pre-fader tap copy for pre-fader aux sends.
    rt_prefader: Vec<f32>,
    /// Panned stereo pair (post-pan, pre-PDC).
    rt_ch_left: Vec<f32>,
    rt_ch_right: Vec<f32>,
    /// Bus-id scratch for the bus-processing tail (aux first, then group —
    /// the same ordering `process_mix` uses).
    rt_aux_ids: Vec<BusId>,
    rt_group_ids: Vec<BusId>,
}
```

Most fields are `pub` (mutable from `engine_mut()`). `buffer_size`,
`max_latency`, `bus_accumulators`, and the six `rt_*` scratch fields are
private. The scratch fields exist solely for `process_mix_rt` (§6) and are
sized once in the constructor (`vec![0.0; buffer_size]`, `processing.rs:458-463`).

#### Constructor (`processing.rs:446`)

```rust
    /// Create a new processing engine for the given buffer size.
    #[must_use]
    pub fn new(buffer_size: usize) -> Self {
```

#### DSP entry — `process_mix()` (`processing.rs:559-563`) — **verbatim signature**

```rust
    pub fn process_mix(
        &mut self,
        channels: &[(ChannelId, ChannelProcessParams)],
        input_samples: &[f32],
    ) -> (Vec<f32>, Vec<f32>) {
```

Returns `(master_left, master_right)`, each a `Vec<f32>` of length
`buffer_size`. See §2 for how `input_samples` is used (the per-channel gap)
and §6 for the fork's real-time per-channel variant `process_mix_rt`.

#### Other `pub` methods on `ProcessingEngine` (`processing.rs`)

```rust
    pub fn register_bus(&mut self, bus_id: BusId, bus_type: BusType)              // :468
    pub fn set_bus_gain(&mut self, bus_id: BusId, gain: f32)                      // :476
    pub fn set_bus_muted(&mut self, bus_id: BusId, muted: bool)                   // :483
    pub fn recompute_pdc(&mut self)                                               // :493
    pub fn max_latency(&self) -> usize                                            // :516
    pub fn vca_gain_for_channel(&self, channel_id: ChannelId) -> f32              // :525
    pub fn process_channel_effects(&mut self, channel_id: ChannelId, working: &mut [f32])  // :541
```

The fork adds `process_mix_rt` (`:765`, §6).

---

### 1.3 Supporting types (verbatim)

#### `ChannelProcessParams` (`processing.rs:930-945`)

```rust
/// Parameters for processing a single channel through the mix pipeline.
#[derive(Debug, Clone, Copy)]
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

> **Fork change (`14e623c8`):** the derive is now `Copy` (upstream 0.2.0/0.2.1
> had `Clone` only). This is what lets the CakeMix binding keep its engine
> call lists in fixed-size stack arrays of `(ChannelId, ChannelProcessParams,
> &[f32])` tuples (§6).

```rust
/// Pan law selection for the processing engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanLawType {                 // processing.rs:949
    Linear,
    Minus3dB,
    Minus4Dot5dB,
    Minus6dB,
}
impl Default for PanLawType { fn default() -> Self { Self::Minus3dB } }   // :960
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

#### `ProcessAuxSend` (`processing.rs:192-201`)

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

#### `VcaGroupState` (`processing.rs:209-218`)

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
Newtype over `uuid::Uuid` (v4 generated in `add_channel`). **Fork addition
(`14e623c8`):** `ChannelId::nil()` (`channel.rs:20`) returns the all-zero
UUID — a placeholder that never collides with v4 ids, used by the CakeMix
binding to initialize its fixed-size call-list arrays.

#### `ChannelType` (`channel.rs:33-48`)

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

#### `Channel` (`channel.rs:214-270`) — full struct

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
    input: InputRouting,          // ← metadata; see §2 (channel.rs:257)
    output: Option<OutputRouting>,
    direct_monitoring: DirectMonitoring,
    color: Option<String>,
    icon: Option<String>,
}
```

Constructor (`channel.rs:275`) and the accessors used by `process()`'s
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
    pub fn gain(&self) -> f32                       // channel.rs:397
    pub fn pan(&self) -> f32                        // channel.rs:427
    pub fn is_muted(&self) -> bool                  // channel.rs:353
    pub fn is_phase_inverted(&self) -> bool         // channel.rs:386
    pub fn pan_law(&self) -> PanLaw                 // channel.rs:449
    pub fn input(&self) -> &InputRouting            // channel.rs:537
```

`InputRouting` (`channel.rs:147-156`) — the metadata that is **read but only
for `gain_db`** on the live path:

```rust
pub struct InputRouting {
    pub source: InputSource,        // Hardware(u32) | Bus(BusId) | Virtual | None  — NEVER read by process_mix (channel.rs:171-180)
    pub gain_db: f32,               // ← the only field used on the live path
    pub highpass_enabled: bool,     // ← never applied on the live path
    pub highpass_freq: f32,         // ← never applied on the live path
}
```

#### `MixerConfig` (`lib.rs:309-348`)

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

#### `BusType` (`bus.rs:25-34`) and `BusId` (`bus.rs:15`)

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

#### `AudioEffect` trait (`effects_chain.rs:8-13`) — the effects-chain interface

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

**Step 1 — one buffer is extracted from the frame** (`lib.rs:637`):

```rust
        let pooled_input = extract_f32_samples(frame, buffer_size, &self.sample_pool);
```

`extract_f32_samples` (`lib.rs:1122-1169`) decodes the raw interleaved/planar
bytes of the **whole** `AudioFrame` into a single contiguous `PooledBuffer`
(`Deref<Target = [f32]>`). It is called exactly once per `process()` call.

**Step 2 — that single buffer is passed, unmodified, to the engine**
(`lib.rs:677-678`):

```rust
        let (mut master_left, mut master_right) =
            self.engine.process_mix(&channel_params, &pooled_input);
```

`channel_params` carries per-channel **gain/pan/mute** state, but **no audio
data** — `ChannelProcessParams` has no audio field (see §1.3). The audio is the
single `&pooled_input`.

**Step 3 — inside `process_mix`, every channel reads the same `input_samples`**
(`processing.rs:583-588`, inside `for (channel_id, params) in channels { … }`):

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

`Channel.input: InputRouting` (`channel.rs:257`) carries an `InputSource`
(`Hardware(u32)` / `Bus(BusId)` / `Virtual` / `None`, `channel.rs:171-180`). On
the live path, `process()` reads **only** `ch.input().gain_db`
(`lib.rs:666`, `input_gain_db: ch.input().gain_db`) to build
`ChannelProcessParams`. The `InputSource` discriminant is **never inspected** to
route distinct audio to distinct channels — nothing maps `InputSource::Hardware(n)`
to a per-strip audio buffer. `highpass_enabled`/`highpass_freq` are likewise
never applied in `process_mix`.

### What this means for CakeMix

`process_mix` cannot be used as-is for a multi-source mixer via `AudioMixer::process()`.
The gap is now closed in the fork itself: `process_mix_rt` (§6) takes
per-channel audio slices directly. The binding drives it per block through
`AudioMixer::engine_mut()` (§5 addendum) — `AudioMixer::process()` remains
bypassed.

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

- **Dead:** `StripEq::process` (`channel_strip.rs:159-171`) is a **fake EQ**.
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
>     pub fn db_to_linear(db: f32) -> f32 { 10.0_f32.powf(db / 20.0) }   // :6
>     #[allow(dead_code)]
>     pub fn linear_to_db(x: f32) -> f32 { … }                          // :13
> ```

Contains `Compressor`/`CompressorConfig`, `Expander`, `Gate`, `Limiter` (all
`pub struct` with `process_sample(&mut self, sample: f32, sample_rate: u32) -> f32`).

- **Dead:** `Compressor`, `Expander`, `Gate`, and this module's own `Limiter`/
  `LimiterConfig` are **not referenced anywhere** in the engine outside
  `dynamics.rs` (no external `use`, no construction in `process_mix` or
  `effects.rs`). The `Limiter::new` hits in the codebase are
  `limiter.rs::Limiter` and `oversampled_limiter.rs::OversampledLimiter` —
  different types. (Outside the engine crate, the CakeMix binding *does* use
  these types directly — its staging layer runs `Compressor`/`Expander`/`Gate`
  per strip ahead of the engine call; fork commit `3283cb3f` added
  `Compressor::last_gain_reduction_db()` (`dynamics.rs:133`) for the binding's
  GR meters.)
- **Dead:** `db_to_linear`/`linear_to_db` here (note: `processing.rs` has its
  **own** `db_to_linear` (`processing.rs:973`), which is the one `process_mix`
  actually calls).
- **Live:** nothing from this module is on the engine's RT path. RT
  dynamics/limiting inside the engine is done via `Box<dyn AudioEffect>`
  plugin slots and the master `OversampledLimiter`
  (`oversampled_limiter.rs`), driven from `lib.rs`.

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

### Blocker 1 — `rayon` is an **unconditional** dep of `oximedia-mixer` — ✅ RESOLVED in fork (`24a35975`)

Upstream (pre-fork) `crates/oximedia-mixer/Cargo.toml`:

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

There was **no** `parallel` feature; `rayon` was always compiled. It is used in
exactly one file:

```rust
// parallel_mix.rs:16
use rayon::prelude::*;
// parallel_mix.rs:134
        .par_iter()
```

(`offline_bounce.rs` does **not** use rayon — the only `use rayon` /
`par_iter` in the crate is `parallel_mix.rs`.)

**Applied in the fork** — `crates/oximedia-mixer/Cargo.toml` now reads:

```toml
[features]
default = []
parallel = ["dep:rayon"]
rayon = { workspace = true, optional = true }
```

and `parallel_mix` is cfg-gated: `#[cfg(feature = "parallel")] pub mod
parallel_mix;` (`lib.rs:163`), with `process_parallel()` gated the same way.
The `parallel` feature stays off for wasm builds; no audio-thread parallelism
is lost (Web Audio's 128-frame render quantum is single-threaded by design).
The fork commit also dropped the phantom `scirs2-core` dependency (it had zero
uses in the mixer crate).

### Blocker 2 — `oxifft` ships `threading` (= `rayon`) in its **default** features — ✅ RESOLVED (no fork needed)

`oximedia-mixer` does **not** depend on `oxifft` directly, but its dep
`oximedia-audio` does. In oxifft 0.3.x `default = ["std", "threading"]` and
`threading` pulls `dep:rayon`; because Cargo **feature-unifies** the default,
an external crate could not turn `threading` off — rayon was always enabled
transitively.

**Resolution (current state):** oxifft 0.4.2 makes `threading` a hard
`compile_error!` on wasm32, and the fork's workspace declares:

```toml
oxifft = { version = "0.4.2", default-features = false, features = ["std"] }
```

with `oximedia-audio` re-adding `features = ["threading"]` on non-wasm targets
only (target-specific dependency table). Real-time audio never wants oxifft's
threading on any target; the mixer's own `spectrum_analyzer.rs` uses a
**pure-Rust** Cooley-Tukey FFT and does not call oxifft at all. No oxifft
fork, patch, or `vendor/` copy is needed.

### Combined patch posture (as shipped)

```toml
# CakeMix Cargo.toml (workspace)
[patch.crates-io]
# LOCAL DEV: path patch into ~/oximedia (has process_mix_rt, unpushed).
# When maxolgi/oximedia master is pushed, revert these three to:
#   { git = "https://github.com/maxolgi/oximedia", branch = "master" }
oximedia-mixer = { path = "/home/flibb/oximedia/crates/oximedia-mixer" }
oximedia-core  = { path = "/home/flibb/oximedia/crates/oximedia-core" }
oximedia-audio = { path = "/home/flibb/oximedia/crates/oximedia-audio" }
```

The three-crate path patch exists because the fork's `process_mix_rt` commits
are **not yet pushed**; after the push it reverts to the git form (§6).

### (Non-blocker) other wasm adjustments

- `scirs2-core` was a phantom dep on the RT path (`rg scirs` → 0 hits in the
  mixer); the fork removed it outright in `24a35975`. Not a blocker.
- `oximedia-core` needs its `wasm` feature; `getrandom 0.4` needs
  `features = ["wasm_js"]`; `uuid` needs `js` (already in the workspace).

---

## 5. INTEGRATION PATH DECISION

> **⚠️ SUPERSEDED — see the dated addendum at the end of this section.**
> The M0 plan below (Option A: call `process_mix` once per channel from the
> binding) shipped and passed its known-answer tests, but has since been
> replaced by the fork's `process_mix_rt` (§6) driven per block. The analysis
> is kept for the record.

**We can call `ProcessingEngine::process_mix()` directly with per-channel
slices — and we must, because `AudioMixer::process()` cannot.**

This works because:

1. `ProcessingEngine` **is** `pub`-exported as `oximedia_mixer::ProcessingEngine`
   (`lib.rs:159`, `lib.rs:261-263`, struct at `processing.rs:396`).
2. `AudioMixer::engine_mut(&mut self) -> &mut ProcessingEngine` (`lib.rs:777`)
   hands out a mutable reference to it, so a binding can populate
   `channel_routing`, `channel_sends`, `vca_groups`, `channel_effects`, etc.
   directly.
3. `process_mix(&mut self, channels: &[(ChannelId, ChannelProcessParams)],
   input_samples: &[f32]) -> (Vec<f32>, Vec<f32>)` (`processing.rs:559`) is
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

- **(Option A — call per-channel, leave `process_mix` untouched):** ~~loop over
  channels, and for each, build a `ChannelProcessParams` and run the channel
  DSP on **that channel's own** slice, accumulating into the master.~~
  *(This was the shipped M0 path; superseded — see addendum.)*
- **(Option B — fork a per-channel-input `process_mix`):** add a variant that
  takes `&[(ChannelId, ChannelProcessParams, &[f32])]` (per-channel audio) in
  the engine fork. Cleaner caller code, but it is an API change to the engine.
  *(This is what shipped — `process_mix_rt`, §6.)*

Decision (b) / Option A was the M0 path: `process_mix`'s signature stayed
untouched, the engine fork was wasm-only (`rayon` gating + `oxifft` patch), and
the per-channel-input logic lived entirely in the CakeMix binding. This matched
SlopShady's discrete-arrival pattern (one `Float32Array` per strip) and made
the M0 known-answer test (two distinct sine tones → correct stereo mix)
straightforward.

**What we keep from the engine as-is:** the DSP primitives are real and
verified end-to-end (`plan.md` honesty check): input-gain/phase, the
`AudioEffect` effects chain, VCA×fader, the four stereo pan laws, PDC delay,
aux sends, and bus routing/accumulation all run real numeric math in
`process_mix`. The binding reuses `ProcessingEngine` for all of that; it only
replaces the **input fan-out** that `AudioMixer::process()` gets wrong.

### Addendum (2026-08-16) — what actually shipped

Option A held until the per-channel `process_mix` loop's allocation cost and
the bus architecture forced a rethink. The shipped integration (CakeMix commit
`84077ea`, engine fork `0a517fb0`):

- **`AudioMixer::process()` is bypassed entirely.** The binding
  (`crates/mixer-wasm/`) drives **nine `ProcessingEngine` instances** per
  block via `AudioMixer::engine_mut().process_mix_rt(...)`: one main console
  (input strips 0-127) + eight 16-slot bus consoles. The `AudioMixer` objects
  are used only as engine containers + channel/param storage; the engine
  performs **all** summing.
- **Per-channel inputs are resolved at the binding's staging layer.** Each
  block, the binding drains its elastic input FIFOs, runs per-strip
  gate → expander → compressor → EQ "staging" on the raw source, and builds
  fixed-size **stack** call lists of `(ChannelId, ChannelProcessParams,
  &[f32])` pointing into the staged rows (possible because the fork made
  `ChannelProcessParams` `Copy` and added `ChannelId::nil()` for
  initialization) — zero steady-state heap allocation.
- **Buses are whole mixers, not engine bus/aux constructs.** Each bus engine
  sums its 16 slots; the binding's "bus tail" then applies bus gain/mute,
  publishes the bus's own stereo output (`take_bus_output`), and accumulates
  into master when the bus feeds main. Bus slots stage the **raw** row of
  their assigned source (parallel-tap semantics, pinned by
  `tests/bus_parallel_test.rs`).
- **Two console toggles cover the group-routing and independent-aux
  semantics** the engine's `ChannelOutputTarget`/aux-send machinery would
  otherwise provide: `set_channel_main_assign(ch, on)` removes a strip from
  the main engine's call list (strip still reaches master via its bus slots —
  group-bus routing), and `set_bus_feeds_main(bus, on)` detaches a bus from
  master while `take_bus_output` keeps publishing it (independent aux/monitor
  bus).

Full `process_mix_rt` API details in §6.

---

## 6. FORK ADDITIONS: `process_mix_rt` and friends

The CakeMix fork (`maxolgi/oximedia`) extends the engine beyond the wasm
build fixes (§4). Everything in this section is upstream-tracked API added on
top of the 0.2.1 merge base `aa9e68af`.

### 6.1 `process_mix_rt` (`processing.rs:765`) — verbatim

```rust
    /// Real-time multi-source variant of [`Self::process_mix`].
    ///
    /// Step-for-step identical DSP (input gain/phase → effects chain →
    /// fader × VCA → pan → PDC → aux sends → route to master/group bus →
    /// bus processing), with two changes required by real-time multi-source
    /// mixing (the live WASM audio case):
    ///
    /// 1. **Per-channel inputs.** Each entry carries its own input slice.
    ///    `process_mix` feeds one shared buffer to every channel, which
    ///    cannot express a multi-source console.
    /// 2. **Zero steady-state allocation.** Outputs are summed into
    ///    caller-provided `out_left`/`out_right`, and all per-channel and
    ///    per-call scratch (working buffer, pre-fader tap, panned pair,
    ///    bus-id lists) is pre-allocated on the engine.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when `out_left`/`out_right` are shorter than
    /// `buffer_size`. In release the first `buffer_size` samples are used
    /// unconditionally — callers on the real-time path always pass exact-size
    /// buffers.
    ///
    /// Input slices shorter than `buffer_size` are zero-padded, matching
    /// `process_mix`. Keep the DSP in sync with `process_mix` when editing
    /// either method.
    #[allow(clippy::too_many_lines)]
    pub fn process_mix_rt(
        &mut self,
        channels: &[(ChannelId, ChannelProcessParams, &[f32])],
        out_left: &mut [f32],
        out_right: &mut [f32],
    ) {
```

No return value: outputs are **summed into** `out_left`/`out_right`
(zero-filled first, so an empty call list yields silence), which the caller
reuses block after block.

### Differences from `process_mix`

| | `process_mix` (`:559`) | `process_mix_rt` (`:765`) |
|---|---|---|
| Audio input | one `&[f32]` shared by **every** channel (§2) | per-channel slice in each tuple |
| Output | allocates and returns `(Vec<f32>, Vec<f32>)` | sums into caller-provided `&mut [f32]` |
| Per-channel scratch | `Vec<f32>` allocated per channel per call (working buffer + unconditional pre-fader clone + stereo pair) | engine fields `rt_working`/`rt_prefader`/`rt_ch_left`/`rt_ch_right` (see §1.2) |
| Bus tail | fresh `Vec`s for bus-id collection | `rt_aux_ids`/`rt_group_ids` cleared + refilled per call (aux first, then group — same ordering) |
| Steady-state allocs | ~4 Vecs/channel + per-call master/bus Vecs | **zero** (pinned by `tests/process_mix_rt_alloc.rs`: counting global allocator, 128ch × 200 blocks = 0 bytes) |
| DSP steps | input gain/phase → effects → VCA×fader → pan → PDC → sends → routing → bus tail | **identical, step for step** (pinned by parity tests) |

The muted-channel `continue`, the zero-pad of short inputs, the
mono-send→equal-L/R aux contribution, and the bus-not-found→master fallback
all match `process_mix` exactly.

### 6.2 Other fork API additions

- `ChannelId::nil()` (`channel.rs:20`) — all-zero UUID placeholder, never
  collides with `add_channel`'s v4 ids. Used to initialize fixed-size
  call-list arrays.
- `Copy` on `ChannelProcessParams` (`processing.rs:931`) — see §1.3.
- `Compressor::last_gain_reduction_db(&self) -> f32` (`dynamics.rs:133`) —
  gain reduction applied by the most recent `process_sample` call, in dB.
  Read by the CakeMix binding's GR meters.

### 6.3 Fork commit state (as of 2026-08-16)

Six commits beyond upstream, **not yet pushed**; CakeMix's root `Cargo.toml`
temporarily path-patches `oximedia-{mixer,core,audio}` to `~/oximedia`
(revert to the git patch after pushing — §4):

| Commit | What |
|--------|------|
| `24a35975` | rayon optional behind `parallel` feature; phantom `scirs2-core` dep removed |
| `b2b065af` | `std::time` patched for `wasm32-unknown-unknown` (session.rs, offline_bounce.rs) |
| `0e900b20` | scene_recall.rs `Scene::new` fixed for wasm32 |
| `aa9e68af` | merge base: upstream/master 0.2.1 (`5263510`) merged in |
| `0a517fb0` | **`process_mix_rt`** — per-channel-input, alloc-free mixing (+ engine tests + alloc integration test) |
| `14e623c8` | `ChannelId::nil()` + `Copy` on `ChannelProcessParams` |
| `3283cb3f` | `Compressor::last_gain_reduction_db()` |

> Note: the pre-merge fork commit `b88cf333` (floor `linear_to_db(0.0)` at
> −200 dB) did **not** survive the 0.2.1 merge — upstream `linear_to_db`
> returns `−inf` again and a test pins that (`dynamics.rs`, `test_linear_to_db_zero`).

### 6.4 Fork test coverage

`process_mix_rt` is pinned in-engine (all in `processing.rs` `#[cfg(test)]`,
`:1821+`): distinct-inputs known answer, exact parity with `process_mix`
(single-channel **and** a buses/sends/VCA scenario), muted/fader behavior,
short-input zero-padding; plus the integration test
`tests/process_mix_rt_alloc.rs` (counting global allocator: 128 channels ×
200 blocks, zero steady-state allocation). Fork suite: 993 lib tests + 1
alloc integration test.
