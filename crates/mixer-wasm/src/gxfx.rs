//! Per-strip guitarix FX chains (gxfx): Rust chain state driving the
//! C++ DSP compiled from `gxfx/` by build.rs.
//!
//! The C++ side (`gxfx_dsp_eleven.cpp`) is a stateless factory + per-
//! instance dispatcher over the guitarix faust `PluginLV2` classes; ALL
//! param storage, chaining, pair logic and strip state live here. The
//! slot model mirrors OctOBX's `wasm/obxd/gxfx_host.cpp` engine exactly:
//!
//! * The console's 128 input strips are 64 stereo pairs. The EVEN
//!   channel (2t) owns the pair's FX chain; 2t+1 is the R half. Pair p
//!   covers channels 2p / 2p+1.
//! * 11 slots per chain; the slot index IS the chain position. A slot
//!   holds any effect id (duplicates allowed) or -1 (empty), 48 param
//!   floats in ENGINE units (ttl ranges, not 0..1), and an enabled flag.
//! * `set_slot` eagerly CREATES the DSP at message time (`gxfx_create`
//!   calls set_samplerate internally, whose init() computes the
//!   rate-dependent constants; the megabyte-scale delay/reverb tapes
//!   are new'd at first activate) and connects every param port to the
//!   slot's param storage BEFORE activation. `set_slot` also resets the
//!   slot's params to the catalog defaults and clears enabled.
//! * `set_enabled` eagerly activates/deactivates (message time).
//! * `move_slot` array-moves the whole slot content — id, params,
//!   enabled, DSP handles — and re-points every shifted slot's DSP at
//!   its (moved) param row, exactly like OctOBX's `fx_move_slot`.
//! * DSP creation/destruction NEVER happens inside the processing path.
//! * MONO effects get TWO instances (L and R, dual-mono); STEREO
//!   effects get ONE instance run across the pair.
//!
//! The `MixerWasm` integration lives in `lib.rs` (construction, the
//! FX pass inside `process_block`, the `fx_*` wasm setters), with
//! scene capture/recall in `scene.rs`/`lib.rs` and the console-JSON
//! fx block in `console_json.rs`; this module only owns the state and
//! the FFI.

use std::ffi::{c_char, c_int};
use std::fmt;

use crate::console_json::json_escape;

/// Stereo pairs on the console (128 input strips / 2).
pub const CHAIN_PAIRS: usize = 64;
/// FX slots per chain (chain position = slot index).
pub const CHAIN_SLOTS: usize = 11;
/// Param floats per slot, in ENGINE units (padded row width).
pub const SLOT_PARAMS: usize = 48;

/// Cap on concurrently CREATED instances of the memory-class effects
/// (catalog `memory` flag — see [`is_memory_class`]). Sizing: the worst
/// offender among the live effects (stereodelay) new's 2 x 524288 f32 =
/// 4 MiB of tape per instance (stereoecho 2 MiB; stereoverb's state is
/// inline in the instance), so the cap bounds the worst-case tape at
/// 96 x 4 MiB ≈ 384 MiB — big, but a ceiling, not an allocation: it lets
/// every one of the 64 pairs run a delay AND leave 50% headroom
/// (96 = 1.5 x 64) while a pathological all-memory-FX console fails
/// CLEANLY at `set_slot` instead of OOM-crashing the wasm heap.
pub const MEMORY_CLASS_CAP: u32 = 96;

/// Whether `fx`'s DSP allocates megabyte-scale delay/reverb buffers at
/// create/first-activate — read from the generated catalog's per-effect
/// `memory` flag through the C ABI (delay/time family, reverb family,
/// convolver/looper; everything else false). For the live eleven this
/// is exactly the old hardcoded set: ids 8 (delay), 9 (echo),
/// 10 (reverb).
fn is_memory_class(fx: i16) -> bool {
    effect_info(fx).is_some_and(|i| i.memory != 0)
}

// ── FFI: exact mirror of gxfx/gxfx_dsp_eleven.cpp's extern "C" ──────

mod ffi {
    use super::{GxFxEffectInfo, GxFxInstance, GxFxParamDef};
    use std::ffi::c_int;

    extern "C" {
        pub fn gxfx_effect_count() -> c_int;
        pub fn gxfx_param_count(fx: c_int) -> c_int;
        pub fn gxfx_is_stereo(fx: c_int) -> c_int;
        pub fn gxfx_param_default(fx: c_int, param: c_int) -> f32;
        /// Pointee is static storage (gxfx_catalog.h), null when out of range.
        pub fn gxfx_param_def(fx: c_int, param: c_int) -> *const GxFxParamDef;
        /// Fills the effect-level info block (catalog columns + the
        /// availability flag); 0 on an unknown id.
        pub fn gxfx_effect_info(fx: c_int, out: *mut GxFxEffectInfo) -> c_int;
        /// Null when the id is unknown or allocation fails. Calls
        /// set_samplerate BEFORE any port connection.
        pub fn gxfx_create(fx: c_int, sample_rate: u32) -> *mut GxFxInstance;
        pub fn gxfx_connect_port(
            handle: *mut GxFxInstance,
            port: c_int,
            storage: *mut f32,
        ) -> c_int;
        pub fn gxfx_activate(handle: *mut GxFxInstance, start: c_int) -> c_int;
        pub fn gxfx_run_mono(handle: *mut GxFxInstance, buf: *mut f32, n: c_int);
        pub fn gxfx_run_stereo(handle: *mut GxFxInstance, l: *mut f32, r: *mut f32, n: c_int);
        pub fn gxfx_destroy(handle: *mut GxFxInstance);
    }
}

/// Mirror of `GxFxParamDef` in gxfx/gxfx_catalog.h (keep in lockstep).
#[repr(C)]
struct GxFxParamDef {
    port: c_int,
    symbol: *const c_char,
    name: *const c_char,
    def: f32,
    min: f32,
    max: f32,
    step: f32,
    is_integer: c_int,
}

/// Mirror of `GxFxEffectInfo` in gxfx/gxfx_catalog.h (keep in lockstep):
/// one effect's catalog columns plus the runtime availability flag.
#[derive(Clone, Copy)]
#[repr(C)]
struct GxFxEffectInfo {
    key: *const c_char,
    label: *const c_char,
    category: *const c_char,
    stereo: c_int,
    memory: c_int,
    param_count: c_int,
    available: c_int,
}

/// Opaque C++ DSP instance (`struct GxFxInstance { PluginLV2* plugin; }`).
#[repr(C)]
pub struct GxFxInstance {
    _private: [u8; 0],
}

// ── Safe wrappers over the static catalog queries ───────────────────

/// Number of catalog effects (row index = effect id).
pub fn effect_count() -> i16 {
    unsafe { ffi::gxfx_effect_count() as i16 }
}

/// Live param count of `fx` (0 for an unknown id). `pub(crate)`: the
/// binding needs it to bound scene-recall param writes and the JSON
/// serializer to emit only the live params.
pub(crate) fn param_count(fx: i16) -> usize {
    unsafe { ffi::gxfx_param_count(fx as c_int).max(0) as usize }
}

/// Whether `fx` runs the native stereo path (else dual-mono).
fn is_stereo(fx: i16) -> bool {
    unsafe { ffi::gxfx_is_stereo(fx as c_int) != 0 }
}

/// Effect-level info (catalog columns + availability); None for an
/// unknown id. The string pointers inside are static catalog storage.
fn effect_info(fx: i16) -> Option<GxFxEffectInfo> {
    let mut info = GxFxEffectInfo {
        key: std::ptr::null(),
        label: std::ptr::null(),
        category: std::ptr::null(),
        stereo: 0,
        memory: 0,
        param_count: 0,
        available: 0,
    };
    let ok = unsafe { ffi::gxfx_effect_info(fx as c_int, &mut info) };
    (ok != 0).then_some(info)
}

/// Catalog default of `(fx, param)` in ENGINE units (0.0 out of range).
fn param_default(fx: i16, param: usize) -> f32 {
    unsafe { ffi::gxfx_param_default(fx as c_int, param as c_int) }
}

/// LV2 PortIndex of param ordinal `param` (0 out of range).
fn param_port(fx: i16, param: usize) -> c_int {
    match param_def(fx, param) {
        Some(d) => d.port,
        None => 0,
    }
}

/// Full param definition; the pointee is static storage.
fn param_def(fx: i16, param: usize) -> Option<&'static GxFxParamDef> {
    let p = unsafe { ffi::gxfx_param_def(fx as c_int, param as c_int) };
    (!p.is_null()).then(|| unsafe { &*p })
}

/// Read a static C string without CStr (whose `from_ptr` pulls in
/// `strlen`, an unwelcome link-time guest on wasm32-unknown-unknown).
/// Catalog strings are static ASCII; anything else degrades to "".
unsafe fn static_cstr(ptr: *const c_char) -> &'static str {
    if ptr.is_null() {
        return "";
    }
    let mut len = 0usize;
    while unsafe { *ptr.add(len) } != 0 {
        len += 1;
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) };
    std::str::from_utf8(bytes).unwrap_or("")
}

// ── Handle: one C++ DSP instance ────────────────────────────────────

/// Move-only RAII wrapper over one C++ DSP instance: Drop runs the C++
/// teardown (deactivate → delete → free), so a slot's handles are
/// destroyed exactly when the slot state is. Never wraps a null pointer.
struct GxFxHandle(*mut GxFxInstance);

impl GxFxHandle {
    /// Create (set_samplerate included). None = effect unavailable
    /// (null factory / allocation failure).
    fn create(fx: i16, sample_rate: u32) -> Option<Self> {
        let h = unsafe { ffi::gxfx_create(fx as c_int, sample_rate) };
        (!h.is_null()).then_some(Self(h))
    }
    fn activate(&self, start: bool) {
        unsafe { ffi::gxfx_activate(self.0, start as c_int) };
    }
    fn run_mono(&self, buf: *mut f32, n: usize) {
        unsafe { ffi::gxfx_run_mono(self.0, buf, n as c_int) };
    }
    fn run_stereo(&self, l: *mut f32, r: *mut f32, n: usize) {
        unsafe { ffi::gxfx_run_stereo(self.0, l, r, n as c_int) };
    }
}

impl Drop for GxFxHandle {
    fn drop(&mut self) {
        // gxfx_destroy deactivates first (freeing the lazily-new'd
        // tapes) then deletes — safe whatever the active state.
        unsafe { ffi::gxfx_destroy(self.0) };
    }
}

// ── Errors ──────────────────────────────────────────────────────────

/// Clean failures of the chain-state API (all recoverable: the rack
/// keeps its previous state and audio never sees a torn slot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GxFxError {
    /// Pair or slot index out of range.
    BadIndex,
    /// Effect id outside the catalog (-1 ..= effect_count()-1 for
    /// `set_slot`).
    BadEffect,
    /// Effect exists in the catalog but its DSP could not be created —
    /// create returned null (null factory or allocation failure).
    Unavailable,
    /// [`MEMORY_CLASS_CAP`] concurrently created memory-class instances
    /// would be exceeded.
    Budget,
    /// Param operation on an empty slot.
    NoEffect,
    /// Param ordinal outside the slot effect's live param count.
    BadParam,
}

impl fmt::Display for GxFxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            GxFxError::BadIndex => "pair/slot index out of range",
            GxFxError::BadEffect => "effect id outside the catalog",
            GxFxError::Unavailable => "effect unavailable (DSP create failed)",
            GxFxError::Budget => "memory-class instance cap would be exceeded",
            GxFxError::NoEffect => "slot holds no effect",
            GxFxError::BadParam => "param index outside the effect's param count",
        };
        f.write_str(s)
    }
}

impl std::error::Error for GxFxError {}

// ── Chain state ─────────────────────────────────────────────────────

/// One chain slot: effect id (-1 = empty), enabled flag, param storage,
/// the eagerly-created C++ DSP handle(s) and the active flag. MONO
/// effects hold TWO handles (L in `dsp`, R in `dsp_r`, dual-mono);
/// STEREO effects hold ONE (in `dsp`).
struct FxSlot {
    fx: i16,
    enabled: bool,
    active: bool,
    params: [f32; SLOT_PARAMS],
    dsp: Option<GxFxHandle>,
    dsp_r: Option<GxFxHandle>,
}

impl FxSlot {
    const fn empty() -> Self {
        Self {
            fx: -1,
            enabled: false,
            active: false,
            params: [0.0; SLOT_PARAMS],
            dsp: None,
            dsp_r: None,
        }
    }

    fn handle_count(&self) -> u32 {
        self.dsp.is_some() as u32 + self.dsp_r.is_some() as u32
    }
}

/// One stereo pair's FX chain: 11 slots in chain order.
struct GxFxChain {
    slots: [FxSlot; CHAIN_SLOTS],
}

/// Plain-data copy of one pair's whole chain (effect ids, enabled
/// flags, padded param rows) — the scene-capture read returned by
/// [`GxFxRack::chain_snapshot`].
pub struct FxChainState {
    pub fx: [i16; CHAIN_SLOTS],
    pub enabled: [bool; CHAIN_SLOTS],
    pub params: [[f32; SLOT_PARAMS]; CHAIN_SLOTS],
}

/// The whole board: 64 pair-owned chains plus the memory budget.
pub struct GxFxRack {
    sample_rate: u32,
    chains: Vec<GxFxChain>,
    /// Scratch row for the single-channel degrade path of
    /// [`GxFxRack::process_pair`] (pre-allocated; never grown on the
    /// audio path).
    stereo_scratch: Vec<f32>,
    /// Currently created memory-class DSP instances (see
    /// [`MEMORY_CLASS_CAP`]); incremented per handle at create,
    /// decremented at teardown.
    mem_class_instances: u32,
}

impl GxFxRack {
    /// `max_block` sizes the degrade-path scratch: the largest block
    /// `process_pair` will be asked to run single-channel.
    pub fn new(sample_rate: u32, max_block: usize) -> Self {
        Self {
            sample_rate,
            chains: (0..CHAIN_PAIRS)
                .map(|_| GxFxChain {
                    slots: std::array::from_fn(|_| FxSlot::empty()),
                })
                .collect(),
            stereo_scratch: vec![0.0; max_block.max(1)],
            mem_class_instances: 0,
        }
    }

    fn chain_mut(&mut self, pair: usize) -> Result<&mut GxFxChain, GxFxError> {
        if pair >= self.chains.len() {
            return Err(GxFxError::BadIndex);
        }
        Ok(&mut self.chains[pair])
    }

    fn chain(&self, pair: usize) -> Result<&GxFxChain, GxFxError> {
        if pair >= self.chains.len() {
            return Err(GxFxError::BadIndex);
        }
        Ok(&self.chains[pair])
    }

    /// Deactivate + destroy the slot's DSP and reset it to empty
    /// (params zeroed, like OctOBX's empty-slot memset). Budget is
    /// released per destroyed memory-class handle.
    fn slot_teardown(&mut self, pair: usize, slot: usize) {
        let s = &mut self.chains[pair].slots[slot];
        let mem = is_memory_class(s.fx);
        if let Some(h) = s.dsp.take() {
            if mem {
                self.mem_class_instances -= 1;
            }
            drop(h);
        }
        if let Some(h) = s.dsp_r.take() {
            if mem {
                self.mem_class_instances -= 1;
            }
            drop(h);
        }
        *s = FxSlot::empty();
    }

    /// Point every param port of the slot's DSP handle(s) at the slot's
    /// param storage. Called after create and after the array-move in
    /// [`GxFxRack::move_slot`] (the moved C++ instances keep the
    /// connect_ports pointers they were created with — they still
    /// reference their OLD param rows until re-pointed; pointer stores
    /// only, no processing state is touched).
    fn connect_slot_params(&mut self, pair: usize, slot: usize) {
        let fx = self.chains[pair].slots[slot].fx;
        if fx < 0 {
            return;
        }
        let hl = self.chains[pair].slots[slot].dsp.as_ref().map(|h| h.0);
        let hr = self.chains[pair].slots[slot].dsp_r.as_ref().map(|h| h.0);
        let n = param_count(fx);
        for i in 0..n {
            let storage: *mut f32 = &mut self.chains[pair].slots[slot].params[i];
            if let Some(h) = hl {
                unsafe { ffi::gxfx_connect_port(h, param_port(fx, i), storage) };
            }
            if let Some(h) = hr {
                unsafe { ffi::gxfx_connect_port(h, param_port(fx, i), storage) };
            }
        }
    }

    /// Set the effect in chain slot `slot` of pair `pair` (OctOBX
    /// `fx_set_slot` semantics):
    ///
    /// * Tears down whatever the slot held, resets the slot's params to
    ///   the new effect's catalog defaults and clears its enabled flag.
    /// * `-1` empties the slot.
    /// * Eagerly CREATES the DSP now (message time, never the render
    ///   path): `gxfx_create` runs set_samplerate (whose init computes
    ///   the rate-dependent constants), then every param port is
    ///   connected to the slot's param storage BEFORE any activation —
    ///   the activation ordering OctOBX proved in production.
    /// * Errors leave the slot EMPTY (never a torn half-state):
    ///   [`GxFxError::Unavailable`] when the id's factory is null,
    ///   [`GxFxError::Budget`] when the memory-class cap would be
    ///   exceeded (checked BEFORE any teardown, so a rejected set
    ///   leaves the previous effect running).
    pub fn set_slot(&mut self, pair: usize, slot: usize, fx: i16) -> Result<(), GxFxError> {
        self.chain_mut(pair)?;
        if slot >= CHAIN_SLOTS {
            return Err(GxFxError::BadIndex);
        }
        if fx < -1 || fx >= effect_count() {
            return Err(GxFxError::BadEffect);
        }
        // Budget pre-check: teardown releases the old slot's memory-class
        // instances first, so the post-operation count is
        // current - old + new.
        let old = &self.chains[pair].slots[slot];
        let old_mem = if is_memory_class(old.fx) {
            old.handle_count()
        } else {
            0
        };
        let new_mem = if is_memory_class(fx) {
            u32::from(!is_stereo(fx)) + 1
        } else {
            0
        };
        if self.mem_class_instances - old_mem + new_mem > MEMORY_CLASS_CAP {
            return Err(GxFxError::Budget);
        }
        self.slot_teardown(pair, slot);
        if fx < 0 {
            return Ok(());
        }
        // Params → catalog defaults (live ordinals; the padding stays
        // zero — same padded-row content OctOBX memcpy's from FX_DEFAULTS).
        let n = param_count(fx);
        for i in 0..n {
            self.chains[pair].slots[slot].params[i] = param_default(fx, i);
        }
        // Eager create: L always, R too for mono effects (dual-mono).
        let Some(l) = GxFxHandle::create(fx, self.sample_rate) else {
            return Err(GxFxError::Unavailable);
        };
        let r = if is_stereo(fx) {
            None
        } else {
            match GxFxHandle::create(fx, self.sample_rate) {
                Some(h) => Some(h),
                None => {
                    drop(l);
                    return Err(GxFxError::Unavailable);
                }
            }
        };
        self.mem_class_instances += new_mem;
        let s = &mut self.chains[pair].slots[slot];
        s.fx = fx;
        s.enabled = false;
        s.active = false;
        s.dsp = Some(l);
        s.dsp_r = r;
        self.connect_slot_params(pair, slot);
        Ok(())
    }

    /// Effect id in `(pair, slot)`; -1 = empty.
    pub fn slot_effect(&self, pair: usize, slot: usize) -> Result<i16, GxFxError> {
        Ok(self
            .chain(pair)?
            .slots
            .get(slot)
            .ok_or(GxFxError::BadIndex)?
            .fx)
    }

    /// Created DSP instance count of `(pair, slot)` (2 for a mono
    /// effect's dual-mono pair, 1 for stereo, 0 for empty).
    pub fn slot_instances(&self, pair: usize, slot: usize) -> Result<u32, GxFxError> {
        Ok(self
            .chain(pair)?
            .slots
            .get(slot)
            .ok_or(GxFxError::BadIndex)?
            .handle_count())
    }

    /// Currently created memory-class DSP instances (budget readback).
    pub fn memory_class_instances(&self) -> u32 {
        self.mem_class_instances
    }

    /// Whether any pair's chain holds a created+enabled effect — the
    /// exact complement of `process_pair`'s per-slot skip condition
    /// (`fx < 0 || !enabled`), so the whole FX pass can be skipped at
    /// zero cost when no chain is active.
    pub fn any_active(&self) -> bool {
        self.chains
            .iter()
            .any(|c| c.slots.iter().any(|s| s.fx >= 0 && s.enabled))
    }

    /// Whole-chain state read for scene capture (see
    /// `MixerWasm::console_snapshot`): effect ids, enabled flags and
    /// the full padded param rows, copied out of pair `pair`.
    /// Padded entries beyond a slot's live param count are zero
    /// (empty slots are zeroed by teardown; `set_slot` fills only the
    /// live ordinals).
    pub fn chain_snapshot(&self, pair: usize) -> Result<FxChainState, GxFxError> {
        let c = self.chain(pair)?;
        let mut st = FxChainState {
            fx: [-1; CHAIN_SLOTS],
            enabled: [false; CHAIN_SLOTS],
            params: [[0.0; SLOT_PARAMS]; CHAIN_SLOTS],
        };
        for s in 0..CHAIN_SLOTS {
            st.fx[s] = c.slots[s].fx;
            st.enabled[s] = c.slots[s].enabled;
            st.params[s] = c.slots[s].params;
        }
        Ok(st)
    }

    /// Enable/disable chain slot `(pair, slot)` (OctOBX `fx_set_enabled`
    /// semantics plus this engine's eager DEactivation): the flag is
    /// stored, and the DSP is activated (first activate allocates the
    /// delay/reverb tapes) or deactivated (freeing them) immediately,
    /// at message time — never inside the processing path. Enabling an
    /// empty slot is a no-op (nothing to enable).
    pub fn set_enabled(
        &mut self,
        pair: usize,
        slot: usize,
        enabled: bool,
    ) -> Result<(), GxFxError> {
        let s = &mut self
            .chain_mut(pair)?
            .slots
            .get_mut(slot)
            .ok_or(GxFxError::BadIndex)?;
        if s.fx < 0 {
            return Ok(());
        }
        s.enabled = enabled;
        match (enabled, s.active) {
            (true, false) => {
                if let Some(h) = &s.dsp {
                    h.activate(true);
                }
                if let Some(h) = &s.dsp_r {
                    h.activate(true);
                }
                s.active = true;
            }
            (false, true) => {
                if let Some(h) = &s.dsp {
                    h.activate(false);
                }
                if let Some(h) = &s.dsp_r {
                    h.activate(false);
                }
                s.active = false;
            }
            _ => {}
        }
        Ok(())
    }

    /// Enabled flag of `(pair, slot)`.
    pub fn is_enabled(&self, pair: usize, slot: usize) -> Result<bool, GxFxError> {
        Ok(self
            .chain(pair)?
            .slots
            .get(slot)
            .ok_or(GxFxError::BadIndex)?
            .enabled)
    }

    /// Plain control-rate param write into the slot's storage (the DSP
    /// reads through its connected pointers each block). `param` is the
    /// param ORDINAL (catalog row order), not the LV2 port.
    pub fn set_param(
        &mut self,
        pair: usize,
        slot: usize,
        param: usize,
        value: f32,
    ) -> Result<(), GxFxError> {
        let s = &mut self
            .chain_mut(pair)?
            .slots
            .get_mut(slot)
            .ok_or(GxFxError::BadIndex)?;
        if s.fx < 0 {
            return Err(GxFxError::NoEffect);
        }
        if param >= param_count(s.fx) {
            return Err(GxFxError::BadParam);
        }
        s.params[param] = value;
        Ok(())
    }

    /// Param readback (ENGINE units) of `(pair, slot, param)`.
    pub fn param(&self, pair: usize, slot: usize, param: usize) -> Result<f32, GxFxError> {
        let s = self
            .chain(pair)?
            .slots
            .get(slot)
            .ok_or(GxFxError::BadIndex)?;
        if s.fx < 0 {
            return Err(GxFxError::NoEffect);
        }
        if param >= param_count(s.fx) {
            return Err(GxFxError::BadParam);
        }
        Ok(s.params[param])
    }

    /// Array-move (remove at `from`, insert at `to`) of a whole chain
    /// slot — id, params, enabled flag and DSP handles travel together
    /// (OctOBX `fx_move_slot`). Every shifted slot's DSP is then
    /// re-pointed at its (moved) param row.
    pub fn move_slot(&mut self, pair: usize, from: usize, to: usize) -> Result<(), GxFxError> {
        let chain = self.chain_mut(pair)?;
        if from >= CHAIN_SLOTS || to >= CHAIN_SLOTS {
            return Err(GxFxError::BadIndex);
        }
        if from == to {
            return Ok(());
        }
        // rotate moves the values (handles + params) in place; the DSP
        // instances' connect_ports pointers still address their old rows
        // until the reconnect pass below.
        if from < to {
            chain.slots[from..=to].rotate_left(1);
        } else {
            chain.slots[to..=from].rotate_right(1);
        }
        let (lo, hi) = (from.min(to), from.max(to));
        for s in lo..=hi {
            self.connect_slot_params(pair, s);
        }
        Ok(())
    }

    /// Run pair `pair`'s enabled chain across one block: `l` and `r`
    /// are the pair's L/R rows (in-place; typically 2t / 2t+1 of the
    /// staged block). Disabled and empty slots are skipped entirely.
    ///
    /// * STEREO effects run the native stereo path on both slices.
    /// * MONO effects run dual-mono: the L instance on `l`, the R
    ///   instance on `r`.
    /// * Single-channel degrade: when `r` is absent, stereo effects run
    ///   with L = R in and out — `l` is copied into an internal
    ///   scratch row that stands in for the missing R, both channels
    ///   are processed, and the L output is kept (the scratch row's
    ///   output is discarded). Mono effects run their L instance on
    ///   `l` only. At most `max_block` (the constructor argument)
    ///   samples run in this mode; longer slices are truncated to it.
    ///
    /// Allocation-free by construction: all DSP state is pre-created,
    /// the scratch row is pre-allocated, and there are no Vecs and no
    /// logging on this path. OctOBX parity: an ensure-active fallback
    /// (dead code given eager `set_enabled`) keeps a slot from ever
    /// running unactivated. Unknown pair indices are ignored.
    pub fn process_pair(&mut self, pair: usize, l: &mut [f32], mut r: Option<&mut [f32]>) {
        let Some(chain) = self.chains.get_mut(pair) else {
            return;
        };
        let scratch = &mut self.stereo_scratch;
        let n = match r.as_deref_mut() {
            Some(r) => l.len().min(r.len()),
            None => l.len().min(scratch.len()),
        };
        if n == 0 {
            return;
        }
        for slot in chain.slots.iter_mut() {
            if slot.fx < 0 || !slot.enabled {
                continue;
            }
            if !slot.active {
                if let Some(h) = &slot.dsp {
                    h.activate(true);
                }
                if let Some(h) = &slot.dsp_r {
                    h.activate(true);
                }
                slot.active = true;
            }
            if is_stereo(slot.fx) {
                let Some(h) = &slot.dsp else { continue };
                match r.as_deref_mut() {
                    Some(r) => h.run_stereo(l[..n].as_mut_ptr(), r[..n].as_mut_ptr(), n),
                    None => {
                        scratch[..n].copy_from_slice(&l[..n]);
                        h.run_stereo(l[..n].as_mut_ptr(), scratch[..n].as_mut_ptr(), n);
                    }
                }
            } else {
                if let Some(h) = &slot.dsp {
                    h.run_mono(l[..n].as_mut_ptr(), n);
                }
                if let (Some(h), Some(r)) = (&slot.dsp_r, r.as_deref_mut()) {
                    h.run_mono(r[..n].as_mut_ptr(), n);
                }
            }
        }
    }
}

// ── Catalog JSON ────────────────────────────────────────────────────

/// Serialize the effect CATALOG as JSON for the FX-chain UI (exposed
/// through wasm-bindgen as `MixerWasm::fx_catalog_json`; the frontend
/// builds its editor from this document). Hand-rolled string building
/// in the crate's console_json style — the catalog is static data, so
/// there is nothing to escape beyond the belt-and-braces `json_escape`.
/// EVERYTHING is read through the C ABI (the generated catalog header is
/// the single source of truth): per effect the key/label/category/
/// stereo/memory columns plus the runtime `available` flag (null
/// factory = DSP not ported yet — the frontend greys those rows out),
/// then the full param tables.
pub fn catalog_json() -> String {
    let mut j = String::from("{\"slots\":");
    j.push_str(&CHAIN_SLOTS.to_string());
    j.push_str(",\"slotParams\":");
    j.push_str(&SLOT_PARAMS.to_string());
    j.push_str(",\"effects\":[");
    let mut first = true;
    for fx in 0..effect_count() {
        let Some(info) = effect_info(fx) else {
            continue;
        };
        if !first {
            j.push(',');
        }
        first = false;
        // SAFETY: the info block's strings are static catalog storage.
        let (key, label, category) = unsafe {
            (
                static_cstr(info.key),
                static_cstr(info.label),
                static_cstr(info.category),
            )
        };
        j.push_str(&format!(
            "{{\"id\":{fx},\"key\":\"{}\",\"label\":\"{}\",\"category\":\"{}\",\"stereo\":{},\"memory\":{},\"available\":{},\"params\":[",
            json_escape(key),
            json_escape(label),
            json_escape(category),
            info.stereo != 0,
            info.memory != 0,
            info.available != 0,
        ));
        for p in 0..param_count(fx) {
            if p > 0 {
                j.push(',');
            }
            match param_def(fx, p) {
                Some(d) => {
                    let (symbol, name) = unsafe { (static_cstr(d.symbol), static_cstr(d.name)) };
                    j.push_str(&format!(
                        "{{\"port\":{},\"symbol\":\"{}\",\"name\":\"{}\",\"default\":{},\"min\":{},\"max\":{},\"step\":{},\"integer\":{}}}",
                        d.port,
                        json_escape(symbol),
                        json_escape(name),
                        d.def,
                        d.min,
                        d.max,
                        d.step,
                        d.is_integer != 0,
                    ));
                }
                None => j.push_str("null"),
            }
        }
        j.push_str("]}");
    }
    j.push_str("]}");
    j
}

// ── wasm-only C runtime floor ───────────────────────────────────────

/// The C runtime floor for the gxfx C++ objects on the freestanding
/// wasm32-unknown-unknown target (no libc): the malloc family over
/// std::alloc, and the libm surface. Natively none of this exists —
/// libc/libm serve the same objects unchanged.
///
/// The math shims delegate to the libm CRATE only. NEVER to std's
/// f32/f64 methods: on wasm32 those lower to LLVM intrinsics that
/// become calls to these very C symbols, so a shim calling e.g.
/// `x.sin()` is infinite recursion at runtime.
#[cfg(target_arch = "wasm32")]
mod c_floor {
    use std::alloc::{alloc as rust_alloc, alloc_zeroed, dealloc, Layout};
    use std::ptr;

    // ── C heap: malloc family over std::alloc ──────────────────────
    //
    // Every allocation carries a 16-byte header holding the REQUESTED
    // byte size, so free/realloc can reconstruct the Layout (alignment
    // is the compile-time constant below, so the size alone identifies
    // it). The header also keeps the returned user pointer 16-aligned
    // (max_align_t and SIMD-friendly).

    const ALIGN: usize = 16;
    const HEADER: usize = 16;

    #[no_mangle]
    pub extern "C" fn malloc(size: usize) -> *mut u8 {
        let Some(total) = size.checked_add(HEADER) else {
            return ptr::null_mut();
        };
        // total >= HEADER > 0, so the zero-size prohibition on
        // std::alloc::alloc can never bite (malloc(0) rides the header).
        let layout = Layout::from_size_align(total, ALIGN).expect("size + 16 is always valid");
        let base = unsafe { rust_alloc(layout) };
        if base.is_null() {
            return ptr::null_mut();
        }
        unsafe { ptr::write(base.cast::<usize>(), size) };
        unsafe { base.add(HEADER) }
    }

    /// # Safety
    ///
    /// `p` must be null or a pointer returned by [`malloc`],
    /// [`calloc`] or [`realloc`] in this module and not yet freed.
    #[no_mangle]
    pub unsafe extern "C" fn free(p: *mut u8) {
        if p.is_null() {
            return;
        }
        let base = unsafe { p.sub(HEADER) };
        let size = unsafe { ptr::read(base.cast::<usize>()) };
        let layout =
            Layout::from_size_align(size + HEADER, ALIGN).expect("header layout always valid");
        unsafe { dealloc(base, layout) };
    }

    #[no_mangle]
    pub extern "C" fn calloc(nmemb: usize, size: usize) -> *mut u8 {
        let Some(bytes) = nmemb.checked_mul(size) else {
            return ptr::null_mut();
        };
        if bytes == 0 {
            return ptr::null_mut();
        }
        let layout = Layout::from_size_align(bytes, ALIGN).expect("checked product is valid");
        let base = unsafe { alloc_zeroed(layout) };
        if base.is_null() {
            return ptr::null_mut();
        }
        unsafe { ptr::write(base.cast::<usize>(), bytes) };
        unsafe { base.add(HEADER) }
    }

    /// # Safety
    ///
    /// `p` must be null or a pointer returned by [`malloc`], [`calloc`]
    /// or [`realloc`] in this module and either null-usable or not yet
    /// freed.
    #[no_mangle]
    pub unsafe extern "C" fn realloc(p: *mut u8, size: usize) -> *mut u8 {
        if p.is_null() {
            return malloc(size);
        }
        if size == 0 {
            unsafe { free(p) };
            return ptr::null_mut();
        }
        let old_size = unsafe { ptr::read(p.sub(HEADER).cast::<usize>()) };
        let new = malloc(size);
        if new.is_null() {
            return ptr::null_mut(); // C semantics: old block untouched
        }
        unsafe { ptr::copy_nonoverlapping(p, new, old_size.min(size)) };
        unsafe { free(p) };
        new
    }

    // ── libm shims ─────────────────────────────────────────────────
    //
    // Defined for the symbols the wasm objects reference today
    // (sin/cos/exp/log/log10/pow and float variants) plus the rest of
    // the <cmath> subset gxfx/shim/cmath declares, so future gxfx TUs
    // link without floor surgery. All delegate to the libm crate.

    macro_rules! math_shim {
        ( $( $name:ident ( $ty:ty ) => $libm:path ; )* ) => {
            $(
                #[no_mangle]
                pub extern "C" fn $name(x: $ty) -> $ty {
                    $libm(x)
                }
            )*
        };
    }

    math_shim! {
        sin(f64) => libm::sin;   cos(f64) => libm::cos;   tan(f64) => libm::tan;
        asin(f64) => libm::asin; acos(f64) => libm::acos; atan(f64) => libm::atan;
        sinh(f64) => libm::sinh; cosh(f64) => libm::cosh; tanh(f64) => libm::tanh;
        exp(f64) => libm::exp;   log(f64) => libm::log;   log2(f64) => libm::log2;
        log10(f64) => libm::log10;
        fabs(f64) => libm::fabs; floor(f64) => libm::floor; ceil(f64) => libm::ceil;
        round(f64) => libm::round; sqrt(f64) => libm::sqrt;
        sinf(f32) => libm::sinf;   cosf(f32) => libm::cosf;   tanf(f32) => libm::tanf;
        asinf(f32) => libm::asinf; acosf(f32) => libm::acosf; atanf(f32) => libm::atanf;
        sinhf(f32) => libm::sinhf; coshf(f32) => libm::coshf; tanhf(f32) => libm::tanhf;
        expf(f32) => libm::expf;   logf(f32) => libm::logf;   log2f(f32) => libm::log2f;
        log10f(f32) => libm::log10f;
        fabsf(f32) => libm::fabsf; floorf(f32) => libm::floorf; ceilf(f32) => libm::ceilf;
        roundf(f32) => libm::roundf; sqrtf(f32) => libm::sqrtf;
    }

    macro_rules! math_shim2 {
        ( $( $name:ident ( $ty:ty ) => $libm:path ; )* ) => {
            $(
                #[no_mangle]
                pub extern "C" fn $name(x: $ty, y: $ty) -> $ty {
                    $libm(x, y)
                }
            )*
        };
    }

    math_shim2! {
        pow(f64) => libm::pow;   atan2(f64) => libm::atan2; fmod(f64) => libm::fmod;
        fmin(f64) => libm::fmin; fmax(f64) => libm::fmax;
        powf(f32) => libm::powf; atan2f(f32) => libm::atan2f; fmodf(f32) => libm::fmodf;
        fminf(f32) => libm::fminf; fmaxf(f32) => libm::fmaxf;
    }
}

// ── Native unit tests ───────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const FS: u32 = 48_000;
    const BS: usize = 128;

    /// Whether `fx`'s DSP factory is live (test-side read of the C ABI's
    /// availability flag; the engine itself rejects null factories at
    /// create inside `set_slot`).
    fn is_available(fx: i16) -> bool {
        effect_info(fx).is_some_and(|i| i.available != 0)
    }

    fn rack() -> GxFxRack {
        GxFxRack::new(FS, BS)
    }

    #[test]
    fn catalog_shape() {
        assert_eq!(effect_count(), 83);
        assert_eq!(CHAIN_PAIRS, 64);
        assert_eq!(CHAIN_SLOTS, 11);
        assert_eq!(SLOT_PARAMS, 48);
        // Param counts per gxfx_catalog.h (generator manifest, ids 0..82).
        let counts = [
            2, 1, 3, 5, 4, 6, 3, 4, 7, 7, 5, 2, 4, 3, 3, 2, 3, 3, 2, 2, 2, 2, 2, 1, 1, 1, 5, 2, 11,
            30, 5, 2, 6, 1, 5, 3, 0, 1, 8, 5, 5, 8, 10, 7, 3, 5, 8, 8, 8, 10, 10, 8, 5, 3, 3, 2, 2,
            11, 3, 5, 12, 6, 4, 12, 5, 2, 34, 19, 15, 14, 24, 3, 1, 1, 1, 7, 4, 9, 4, 4, 10, 2, 39,
        ];
        for (fx, &want) in counts.iter().enumerate() {
            assert_eq!(param_count(fx as i16), want, "fx {fx}");
        }
        // Stereo flags per the generated catalog (v1: chorus/delay/echo/
        // reverb, plus the natively-stereo Phase 1+ classes).
        let stereo: [u8; 83] = [
            0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 1, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 1,
            0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0,
        ];
        for (fx, &want) in stereo.iter().enumerate() {
            assert_eq!(is_stereo(fx as i16), want != 0, "fx {fx}");
        }
        // Availability: the live eleven only — ids 11..82 are catalog
        // rows whose DSP TUs have not been ported yet (null factories).
        for fx in 0..11 {
            assert!(is_available(fx), "fx {fx} must be available");
        }
        for fx in 11..83 {
            assert!(!is_available(fx), "fx {fx} must report unavailable");
        }
        // Memory-class flag (catalog `memory`): for the live eleven
        // exactly delay/echo/reverb (the old hardcoded set).
        for fx in 0..11 {
            assert_eq!(is_memory_class(fx as i16), matches!(fx, 8..=10), "fx {fx}");
        }
        // ...and the later-wave memory families carry the flag in the
        // catalog even though their DSP cannot be created yet (spot ids:
        // 45 duck_delay, 57 zita_rev1, 76 cabinet, 82 livelooper).
        for fx in [45, 57, 76, 82] {
            assert!(is_memory_class(fx), "fx {fx} must be memory-class");
        }
        assert!(
            !is_memory_class(80),
            "detune (sub-MB frames) is not memory-class"
        );
        // Unknown ids degrade cleanly.
        assert!(!is_available(-1));
        assert!(!is_memory_class(-1));
        assert!(!is_available(83));
    }

    #[test]
    fn unavailable_ids_are_rejected_and_leave_no_state() {
        let mut r = rack();
        // fuzzface (id 11) is in the catalog but its factory is null:
        // set_slot must fail Unavailable and leave the slot empty.
        assert_eq!(r.set_slot(0, 3, 11).unwrap_err(), GxFxError::Unavailable);
        assert_eq!(r.slot_effect(0, 3).unwrap(), -1);
        assert_eq!(r.slot_instances(0, 3).unwrap(), 0);
        assert_eq!(r.memory_class_instances(), 0);
        // Same at the catalog's end (livelooper, id 82).
        assert_eq!(r.set_slot(0, 3, 82).unwrap_err(), GxFxError::Unavailable);
        // Ids PAST the catalog are BadEffect, not Unavailable.
        assert_eq!(r.set_slot(0, 3, 83).unwrap_err(), GxFxError::BadEffect);
        assert_eq!(r.set_slot(0, 3, 3000).unwrap_err(), GxFxError::BadEffect);
        // The live eleven all set cleanly (distortion rides its own TU).
        for fx in 0..11i16 {
            r.set_slot(0, 0, fx).unwrap();
        }
    }

    #[test]
    fn set_slot_resets_params_and_clears_enabled() {
        let mut r = rack();
        // wah: params [0.0, 0.5]; MONO → two eagerly created instances.
        r.set_slot(0, 3, 0).unwrap();
        assert_eq!(r.slot_instances(0, 3).unwrap(), 2);
        r.set_param(0, 3, 0, -6.0).unwrap();
        r.set_enabled(0, 3, true).unwrap();
        assert_eq!(r.param(0, 3, 0).unwrap(), -6.0);
        assert!(r.is_enabled(0, 3).unwrap());

        r.set_slot(0, 3, 0).unwrap();
        assert_eq!(r.param(0, 3, 0).unwrap(), 0.0, "defaults restored");
        assert_eq!(r.param(0, 3, 1).unwrap(), 0.5);
        assert!(!r.is_enabled(0, 3).unwrap(), "enabled cleared");
        assert_eq!(r.slot_instances(0, 3).unwrap(), 2, "DSP eagerly re-created");

        // STEREO delay: one instance only.
        r.set_slot(0, 3, 8).unwrap();
        assert_eq!(r.slot_instances(0, 3).unwrap(), 1);
        // Padding beyond the live param count stays zero (padded row).
        assert_eq!(r.param(0, 3, 10).unwrap_err(), GxFxError::BadParam);
        // Duplicates are allowed.
        r.set_slot(0, 4, 8).unwrap();
        r.set_slot(0, 5, 8).unwrap();
        assert_eq!(r.memory_class_instances(), 3);

        // Emptying via -1 (and invalid indices/ids are clean errors).
        r.set_slot(0, 3, -1).unwrap();
        assert_eq!(r.slot_effect(0, 3).unwrap(), -1);
        assert_eq!(r.memory_class_instances(), 2);
        assert_eq!(r.set_slot(64, 0, 0).unwrap_err(), GxFxError::BadIndex);
        assert_eq!(r.set_slot(0, 11, 0).unwrap_err(), GxFxError::BadIndex);
        assert_eq!(r.set_slot(0, 0, 83).unwrap_err(), GxFxError::BadEffect);
        assert_eq!(r.set_slot(0, 0, -2).unwrap_err(), GxFxError::BadEffect);
        assert_eq!(r.set_param(0, 3, 0, 1.0).unwrap_err(), GxFxError::NoEffect);
    }

    #[test]
    fn distortion_loads_and_runs_like_any_available_effect() {
        let mut r = rack();
        // Distortion (id 2, bossds1) landed with gxfx_dsp_distortion.cpp:
        // set_slot must create it eagerly (mono → two instances for the
        // pair), accept its three params and run audio through it.
        r.set_slot(1, 2, 2).unwrap();
        assert_eq!(r.slot_effect(1, 2).unwrap(), 2);
        assert_eq!(r.slot_instances(1, 2).unwrap(), 2);
        assert_eq!(r.param(1, 2, 0).unwrap(), -2.0, "LEVEL default");
        assert_eq!(r.param(1, 2, 1).unwrap(), 0.5, "TONE default");
        assert_eq!(r.param(1, 2, 2).unwrap(), 0.5, "DRIVE default");
        r.set_param(1, 2, 2, 0.9).unwrap();
        r.set_enabled(1, 2, true).unwrap();
        let mut buf: Vec<f32> = (0..BS).map(|i| (i as f32 / 32.0).sin() * 0.5).collect();
        let expect = buf.clone();
        let mut rbuf = buf.clone();
        r.process_pair(1, &mut buf, Some(&mut rbuf));
        assert!(
            buf.iter()
                .zip(expect.iter())
                .any(|(o, e)| (o - e).abs() > 1e-4),
            "distortion must actually process audio (output differs from input)"
        );
        assert!(buf.iter().all(|v| v.is_finite()));
        // Not memory-class: its instances never count toward the budget.
        assert_eq!(r.memory_class_instances(), 0);
        r.set_slot(1, 2, -1).unwrap();
        assert_eq!(r.slot_effect(1, 2).unwrap(), -1);
    }

    #[test]
    fn move_slot_relocates_everything() {
        let mut r = rack();
        r.set_slot(3, 5, 8).unwrap();
        r.set_param(3, 5, 4, 250.0).unwrap();
        r.set_enabled(3, 5, true).unwrap();
        r.move_slot(3, 5, 0).unwrap();
        assert_eq!(r.slot_effect(3, 0).unwrap(), 8);
        assert_eq!(r.param(3, 0, 4).unwrap(), 250.0, "params moved");
        assert!(r.is_enabled(3, 0).unwrap(), "enabled moved");
        assert_eq!(r.slot_instances(3, 0).unwrap(), 1, "handle moved");
        // The slots between shifted up one position.
        assert_eq!(r.slot_effect(3, 5).unwrap(), -1);
        assert_eq!(r.move_slot(3, 0, 11).unwrap_err(), GxFxError::BadIndex);
        assert!(r.move_slot(3, 0, 0).is_ok(), "self-move is a no-op");
        // Budget is unaffected by moves.
        assert_eq!(r.memory_class_instances(), 1);
    }

    #[test]
    fn memory_budget_cap_enforced() {
        let mut r = rack();
        // Fill the cap with reverb (id 10 — memory-class, but its state
        // is inline in the instance, so the test stays memory-light).
        let mut created = 0;
        'fill: for pair in 0..CHAIN_PAIRS {
            for slot in 0..CHAIN_SLOTS {
                if created == MEMORY_CLASS_CAP as usize {
                    break 'fill;
                }
                r.set_slot(pair, slot, 10).unwrap();
                created += 1;
            }
        }
        assert_eq!(created, MEMORY_CLASS_CAP as usize);
        assert_eq!(r.memory_class_instances(), MEMORY_CLASS_CAP);
        // One more → clean error, nothing torn down.
        assert_eq!(r.set_slot(63, 10, 10).unwrap_err(), GxFxError::Budget);
        assert_eq!(r.slot_effect(63, 10).unwrap(), -1);
        assert_eq!(r.memory_class_instances(), MEMORY_CLASS_CAP);
        // Replacing a memory-class slot with another stays within budget.
        assert!(r.set_slot(0, 0, 8).is_ok());
        assert_eq!(r.memory_class_instances(), MEMORY_CLASS_CAP);
        // Freeing one releases budget for a new one.
        r.set_slot(1, 0, -1).unwrap();
        assert_eq!(r.memory_class_instances(), MEMORY_CLASS_CAP - 1);
        assert!(r.set_slot(63, 10, 10).is_ok());
        // Non-memory effects are never budget-gated.
        r.set_slot(63, 10, 0).unwrap();
    }

    #[test]
    fn empty_chain_and_disabled_slots_are_bit_exact_noop() {
        let mut r = rack();
        let pattern = |i: usize| ((i * 37) % 251) as f32 / 128.0 - 1.0;
        let mut l: Vec<f32> = (0..BS).map(pattern).collect();
        let mut rr = l.clone();
        let (cl, cr) = (l.clone(), rr.clone());
        // Empty chain: never touched.
        r.process_pair(7, &mut l, Some(&mut rr));
        assert_eq!(l, cl);
        assert_eq!(rr, cr);
        // Slots set (DSP created + connected) but ALL disabled.
        r.set_slot(7, 0, 0).unwrap(); // mono wah, disabled
        r.set_slot(7, 1, 8).unwrap(); // stereo delay, disabled
        r.set_param(7, 0, 0, 2.0).unwrap();
        r.process_pair(7, &mut l, Some(&mut rr));
        assert_eq!(l, cl, "disabled chain is a bit-exact no-op (L)");
        assert_eq!(rr, cr, "disabled chain is a bit-exact no-op (R)");
        // ... and single-channel degrade with everything disabled.
        r.process_pair(7, &mut l, None);
        assert_eq!(l, cl);
    }

    /// End-to-end DSP sanity: stereo delay proves the whole FFI +
    /// connect-ordering + activation path. The impulse's echo must come
    /// back at delay_ms × fs (stereodelay truncates to whole samples)
    /// at ~unity level (L_GAIN 0 dB; the level one-pole needs ~1000
    /// samples to settle, hence the silent lead-in). The slot is MOVED
    /// after being set, so the test also proves the move reconnect
    /// points the DSP at the relocated param row.
    #[test]
    fn stereo_delay_impulse_e2e() {
        let mut r = rack();
        r.set_slot(5, 7, 8).unwrap(); // delay, created at slot 7
                                      // ordinals: 3 = L_GAIN (dB), 4 = L_TIME (ms)
        r.set_param(5, 7, 3, 0.0).unwrap();
        r.set_param(5, 7, 4, 250.0).unwrap(); // 250 ms → 12000 samples @ 48 kHz
        r.set_enabled(5, 7, true).unwrap();
        r.move_slot(5, 7, 2).unwrap(); // now at slot 2

        let mut l = vec![0.0f32; BS];
        let mut rr = vec![0.0f32; BS];
        // Silent lead-in: settle the level one-pole and delay glide.
        for _ in 0..64 {
            r.process_pair(5, &mut l, Some(&mut rr));
        }
        // Impulse at global sample 0, then silence.
        l[0] = 1.0;
        r.process_pair(5, &mut l, Some(&mut rr));
        assert!((l[0] - 1.0).abs() < 1e-6, "dry path is bit-transparent");
        assert!(
            rr.iter().all(|&x| x == 0.0),
            "R stays silent (impulse in L)"
        );

        let delay_samples = 250.0 * FS as f32 / 1000.0; // 12000
        let expect = delay_samples as usize;
        let blocks = (expect + 4 * BS) / BS + 1;
        let mut out = Vec::with_capacity(blocks * BS);
        out.extend_from_slice(&l);
        for _ in 1..blocks {
            l.iter_mut().for_each(|x| *x = 0.0);
            rr.iter_mut().for_each(|x| *x = 0.0);
            r.process_pair(5, &mut l, Some(&mut rr));
            out.extend_from_slice(&l);
            assert!(rr.iter().all(|&x| x == 0.0));
        }
        // No echo energy at half the distance...
        assert!(out[expect / 2].abs() < 1e-6);
        // ...echo at the expected position (±2 samples) near unity.
        let lo = expect.saturating_sub(2);
        let hi = (expect + 2).min(out.len() - 1);
        let (peak_pos, peak) = out[lo..=hi]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, &v)| (lo + i, v))
            .expect("window is non-empty");
        assert!(
            (peak_pos as i32 - expect as i32).abs() <= 2,
            "echo at {peak_pos}, expected {expect}"
        );
        assert!(
            (0.98..=1.02).contains(&peak),
            "echo amplitude {peak} ≈ unity (L_GAIN 0 dB)"
        );
    }

    /// The single-channel degrade path runs stereo effects with L = R
    /// in and out; the L row keeps the effect's L output.
    #[test]
    fn single_channel_degrade_runs_stereo_effects() {
        let mut r = rack();
        r.set_slot(0, 0, 8).unwrap();
        r.set_param(0, 0, 3, 0.0).unwrap(); // L_GAIN 0 dB
        r.set_param(0, 0, 4, 125.0).unwrap(); // L_TIME 125 ms → 6000 samples
        r.set_enabled(0, 0, true).unwrap();

        let mut l = vec![0.0f32; BS];
        for _ in 0..64 {
            r.process_pair(0, &mut l, None);
        }
        l[0] = 1.0;
        r.process_pair(0, &mut l, None);
        let expect = 125.0 * FS as f32 / 1000.0;
        let expect = expect as usize;
        let blocks = (expect + 4 * BS) / BS + 1;
        let mut out = Vec::with_capacity(blocks * BS);
        out.extend_from_slice(&l);
        for _ in 1..blocks {
            l.iter_mut().for_each(|x| *x = 0.0);
            r.process_pair(0, &mut l, None);
            out.extend_from_slice(&l);
        }
        let (peak_pos, peak) = out[expect - 2..=expect + 2]
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .map(|(i, &v)| (expect - 2 + i, v))
            .expect("window is non-empty");
        assert!(
            (peak_pos as i32 - expect as i32).abs() <= 2,
            "degraded echo at {peak_pos}, expected {expect}"
        );
        assert!((0.98..=1.02).contains(&peak), "echo amplitude {peak}");
    }

    #[test]
    fn catalog_json_parses_with_expected_shape() {
        let j = catalog_json();
        let v: serde_json::Value = serde_json::from_str(&j).expect("valid JSON");
        assert_eq!(v["slots"], 11);
        assert_eq!(v["slotParams"], 48);
        let effects = v["effects"].as_array().unwrap();
        assert_eq!(effects.len(), 83);
        // First row spot-checked against gxfx_catalog.h.
        let wah = &effects[0];
        assert_eq!(wah["id"], 0);
        assert_eq!(wah["key"], "wah");
        assert_eq!(wah["label"], "Wah");
        assert_eq!(wah["category"], "wah");
        assert_eq!(wah["stereo"], false);
        assert_eq!(wah["memory"], false);
        assert_eq!(wah["available"], true);
        let wah_params = wah["params"].as_array().unwrap();
        assert_eq!(wah_params.len(), 2);
        assert_eq!(wah_params[0]["port"], 2);
        assert_eq!(wah_params[0]["symbol"], "VOLUME");
        assert_eq!(wah_params[0]["name"], "Volume");
        assert_eq!(wah_params[0]["default"], 0.0);
        assert_eq!(wah_params[0]["min"], -20.0);
        assert_eq!(wah_params[0]["max"], 4.0);
        assert_eq!(wah_params[0]["step"], 0.1);
        assert_eq!(wah_params[0]["integer"], false);
        // Integer flag survives the wire (delay "mode").
        let delay = &effects[8];
        assert_eq!(delay["params"][0]["integer"], true);
        assert_eq!(delay["memory"], true, "delay is memory-class");
        assert_eq!(delay["available"], true);
        // The live eleven are available; everything past them is a
        // not-yet-ported catalog row and must say so.
        for (i, e) in effects.iter().enumerate() {
            assert_eq!(e["available"], i < 11, "effect {i}");
        }
        // ...and the memory-class flag is exposed for later waves too
        // (duck_delay 45, zita 57, cabinet 76, livelooper 82).
        for (i, key) in [
            (45, "duck_delay"),
            (57, "zita_rev1"),
            (76, "cabinet"),
            (82, "livelooper"),
        ] {
            assert_eq!(effects[i]["key"], key);
            assert_eq!(effects[i]["memory"], true);
            assert_eq!(effects[i]["available"], false);
        }
        // Last row: livelooper (39 params, mono, unavailable).
        let looper = effects.last().unwrap();
        assert_eq!(looper["id"], 82);
        assert_eq!(looper["key"], "livelooper");
        assert_eq!(looper["label"], "Live Looper");
        assert_eq!(looper["category"], "special");
        assert_eq!(looper["stereo"], false);
        assert_eq!(looper["available"], false);
        assert_eq!(looper["params"].as_array().unwrap().len(), 39);
        // A zero-param effect serializes an empty params array (autowah).
        assert_eq!(effects[36]["key"], "autowah");
        assert_eq!(effects[36]["params"].as_array().unwrap().len(), 0);
        // Reverb keeps its old spot-check shape (stereo, 5 params).
        let reverb = &effects[10];
        assert_eq!(reverb["key"], "reverb");
        assert_eq!(reverb["stereo"], true);
        assert_eq!(reverb["memory"], true);
        assert_eq!(reverb["params"].as_array().unwrap().len(), 5);
    }
}
