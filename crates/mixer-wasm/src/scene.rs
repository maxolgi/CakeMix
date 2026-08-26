//! Console scenes: snapshot/restore types + fade interpolation helpers.
use wasm_bindgen::prelude::*;

use oximedia_mixer::channel::{db_to_linear, PanLaw};

// ── Scenes ─────────────────────────────────────────────────────────────
//
// DESIGN: scenes live HERE in the binding, not in the engine's
// mix_scene/scene_recall modules — those are unwired data containers
// that know nothing about the binding-owned console (staged EQ/dynamics
// params, bus slot assignments, routing toggles, master gain). A scene
// must snapshot and restore state the binding itself owns or stages, so
// capture/recall walk the binding's own fields and reapply through the
// binding's own setters. Strip existence, input FIFOs and PID mappings
// are LIVE stream state, not console state — scenes never capture or
// touch them.

/// One EQ band's scene state: the {gain, freq, q} surface the wire API
/// exposes (`set_eq_band_*`). The binding's chains are always the fixed
/// 6-band Fairlight layout, so scenes store exactly 6 bands per strip.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct EqBandScene {
    pub gain_db: f32,
    pub freq_hz: f32,
    pub q: f32,
}

/// Compressor params as the scene captures them (None = disabled).
/// Mirrors the `set_comp_param` surface (params 0-5).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct CompScene {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_gain_db: f32,
    pub knee_db: f32,
}

/// Gate params as the scene captures them (None = disabled).
/// Mirrors the `set_gate_param` surface (params 0-4).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct GateScene {
    pub threshold_db: f32,
    pub hysteresis_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub hold_ms: f32,
}

/// Expander params as the scene captures them (None = disabled).
/// Mirrors the `set_expander_param` surface (params 0-3).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ExpanderScene {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

/// One strip's scene state — everything the per-strip setters touch,
/// for input strips (0-127) and bus slots (128-255) alike. `mute` and
/// `solo` are USER intent; the derived engine mute (user mute OR
/// solo-gate) is recomputed on recall by the mute/solo setters.
#[derive(Clone, Debug, PartialEq)]
pub struct StripScene {
    /// Whether the strip existed at save time. Strips are never
    /// destroyed, so a saved strip always still exists at recall;
    /// strips created AFTER the save are skipped (a scene has no
    /// opinion about strips it never saw).
    pub exists: bool,
    pub name: String,
    pub gain: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    pub input_gain_db: f32,
    pub phase_inverted: bool,
    pub pan_law: PanLaw,
    /// Input strips only (meaningless for bus slots).
    pub main_assign: bool,
    pub eq_bypass: bool,
    pub eq_bands: [EqBandScene; 6],
    pub comp: Option<CompScene>,
    pub gate: Option<GateScene>,
    pub expander: Option<ExpanderScene>,
}

impl Default for StripScene {
    fn default() -> Self {
        Self {
            exists: false,
            name: String::new(),
            gain: 1.0,
            pan: 0.0,
            mute: false,
            solo: false,
            input_gain_db: 0.0,
            phase_inverted: false,
            pan_law: PanLaw::Linear,
            main_assign: true,
            eq_bypass: false,
            eq_bands: std::array::from_fn(|_| EqBandScene::default()),
            comp: None,
            gate: None,
            expander: None,
        }
    }
}

/// One bus's scene state: all 16 slot source assignments plus the bus
/// tail controls.
#[derive(Clone, Debug, PartialEq)]
pub struct BusScene {
    pub sources: [Option<u32>; 16],
    pub gain: f32,
    pub muted: bool,
    pub feeds_main: bool,
}

impl Default for BusScene {
    fn default() -> Self {
        Self {
            sources: [None; 16],
            gain: 1.0,
            muted: false,
            feeds_main: true,
        }
    }
}

/// A full console snapshot: all 256 strips, all 8 buses, master gain.
/// Plain Rust (not wasm-exported) — the JS surface is
/// save_scene/recall_scene; the struct is public so native tests can
/// verify scene round-trips field by field.
#[derive(Clone, Debug, PartialEq)]
pub struct ConsoleScene {
    pub strips: [StripScene; 256],
    pub buses: [BusScene; 8],
    pub master_gain: f32,
}

impl Default for ConsoleScene {
    fn default() -> Self {
        Self {
            strips: std::array::from_fn(|_| StripScene::default()),
            buses: std::array::from_fn(|_| BusScene::default()),
            master_gain: 1.0,
        }
    }
}

/// An in-progress timed scene cross-fade (see `recall_scene_fade`).
/// Allocated once at recall start on the control plane; `process_block`
/// advances and applies it at block granularity. Moving the state out
/// of `self` and back per block (`Option::take`) is plain memcpy work —
/// no heap operations — which is how the per-block interpolation keeps
/// the RT path allocation-free.
pub(crate) struct FadeState {
    pub(crate) from: ConsoleScene,
    pub(crate) to: ConsoleScene,
    /// Elapsed fade time (ms). Block k after recall applies
    /// t = clamp(pos / dur, 0, 1) AFTER advancing — zero-order hold at
    /// each block's end position, so a fade of N × block duration spans
    /// exactly N blocks.
    pub(crate) pos_ms: f64,
    pub(crate) dur_ms: f64,
}

// ── Scene interpolation helpers ────────────────────────────────────────
//
// Domains for `apply_scene_interp`: endpoints short-circuit so an exact
// apply (t = 1.0) writes the target's stored values verbatim (instant
// recall stays bit-exact through the same code path).

/// Linear gain → dB with the binding's meter floor (−200 dB, matching
/// the channel/bus meter inserts; 0 linear is digital silence).
pub(crate) fn lin_to_db_floor(x: f32) -> f32 {
    if x > 1e-10 {
        20.0 * x.log10()
    } else {
        -200.0
    }
}

/// dB-domain interpolation of a LINEAR gain (strip fader, bus gain,
/// master gain): equal-dB steps sound equal-loud, so fades ramp
/// perceptually linearly. 0 linear maps to the −200 dB floor.
pub(crate) fn lerp_gain_db(a: f32, b: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let da = lin_to_db_floor(a);
    let db = lin_to_db_floor(b);
    db_to_linear(da + t * (db - da))
}

/// Plain linear interpolation (pan, dB-denominated params, ratios,
/// times — params already stored in their perceptual domain).
pub(crate) fn lerp_lin(a: f32, b: f32, t: f32) -> f32 {
    if t <= 0.0 {
        a
    } else if t >= 1.0 {
        b
    } else {
        a + t * (b - a)
    }
}

/// Log2-domain interpolation (EQ frequency, Q) — geometric mean at the
/// midpoint, matching how those controls are swept by hand.
pub(crate) fn lerp_log2(a: f32, b: f32, t: f32) -> f32 {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    // Scenes shouldn't hold ≤0 values; the clamp keeps log2 finite.
    let la = a.max(1e-10).log2();
    let lb = b.max(1e-10).log2();
    (la + t * (lb - la)).exp2()
}

/// Booleans (mute/solo/phase/bypass/enables/feeds_main/main_assign)
/// snap to the target at t >= 0.5 — the engine mix_scene lerp
/// convention (ChannelSceneState::lerp).
pub(crate) fn lerp_bool(a: bool, b: bool, t: f32) -> bool {
    if t >= 0.5 {
        b
    } else {
        a
    }
}

/// Inverse of the pan-law wire mapping in `set_channel_pan_law`
/// (scene recall converts a stored `PanLaw` back to the wire u32).
pub(crate) fn pan_law_to_wire(law: PanLaw) -> u32 {
    match law {
        PanLaw::Linear => 0,
        PanLaw::Minus3dB => 1,
        PanLaw::Minus4Dot5dB => 2,
        PanLaw::Minus6dB => 3,
    }
}

/// Scene-recall error. `JsValue::from_str` is a wasm-only intrinsic
/// (it panics "function not implemented on non-wasm32 targets" on
/// native builds), and native tests DO exercise the Err path — so on
/// native targets return a bare NULL (callers only observe Ok/Err).
pub(crate) fn scene_err(msg: String) -> JsValue {
    #[cfg(target_arch = "wasm32")]
    {
        JsValue::from_str(&msg)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = msg;
        JsValue::NULL
    }
}
