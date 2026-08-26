//! Console-state JSON serialization for the scene-recall UI loop.
use crate::effects::{CompressorEffect, ExpanderEffect, GateEffect};
use crate::scene::{BusScene, CompScene, ExpanderScene, GateScene};

// ── Console-state JSON (scene-recall UI loop) ─────────────────────────
//
// Recall changes parameters the SolidJS stores mirror; unless the UI is
// told, controls show stale values and the next interaction writes them
// back over the recalled scene. After a recall the worklet pulls this
// serialization (get-params → console-params) and the stores apply it.
// Field names mirror `ChannelState`/`BusState` in
// frontend/src/stores/mixer.ts EXACTLY — keep the two in lockstep.

/// JSON string escape for user-typed names (quote, backslash, control
/// characters). The other manual-JSON builders here only emit numbers
/// and booleans; names are the one place user text reaches the wire.
pub(crate) fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Compressor params an `enable_compressor` would create — the knob
/// values the UI reports for a strip whose comp is disabled (the stores
/// keep knob state while a module is off, so a later enable matches).
pub(crate) fn comp_enable_defaults(sample_rate: u32) -> CompScene {
    let fx = CompressorEffect::broadcast(sample_rate);
    let c = fx.config();
    CompScene {
        threshold_db: c.threshold_db,
        ratio: c.ratio,
        attack_ms: c.attack_ms,
        release_ms: c.release_ms,
        makeup_gain_db: c.makeup_gain_db,
        knee_db: c.knee_db,
    }
}

/// Gate params an `enable_gate` would create (see `comp_enable_defaults`).
pub(crate) fn gate_enable_defaults(sample_rate: u32) -> GateScene {
    let fx = GateEffect::denoise(sample_rate);
    let g = fx.config();
    GateScene {
        threshold_db: g.threshold_db,
        hysteresis_db: g.hysteresis_db,
        attack_ms: g.attack_ms,
        release_ms: g.release_ms,
        hold_ms: g.hold_ms,
    }
}

/// Expander params an `enable_expander` would create (see
/// `comp_enable_defaults`).
pub(crate) fn expander_enable_defaults(sample_rate: u32) -> ExpanderScene {
    let fx = ExpanderEffect::gentle(sample_rate);
    let e = fx.config();
    ExpanderScene {
        threshold_db: e.threshold_db,
        ratio: e.ratio,
        attack_ms: e.attack_ms,
        release_ms: e.release_ms,
    }
}

/// One bus's JSON object: tail controls + all 16 slot source
/// assignments (`null` = unassigned). All 8 buses always serialize —
/// the UI shows all 8.
pub(crate) fn bus_params_json(b: &BusScene) -> String {
    let mut j = format!(
        "{{\"gain\":{},\"muted\":{},\"feedsMain\":{},\"sources\":[",
        b.gain, b.muted, b.feeds_main
    );
    for (slot, src) in b.sources.iter().enumerate() {
        if slot > 0 {
            j.push(',');
        }
        match src {
            Some(ch) => j.push_str(&ch.to_string()),
            None => j.push_str("null"),
        }
    }
    j.push_str("]}");
    j
}
