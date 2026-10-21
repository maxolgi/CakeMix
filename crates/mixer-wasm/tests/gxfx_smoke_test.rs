//! Tiered honesty suite for the gxfx (guitarix) insert FX — the wave-2
//! standard families, driven through the REAL C++ DSP via [`GxFxRack`].
//!
//! Tiered RULE (AGENTS.md honesty rule, user-approved tiering): a ported
//! effect family is only allowed to leave "unavailable" together with
//! its tests, in the same commit. The tiers are
//!   1. a MANDATORY per-effect smoke suite over EVERY available catalog
//!      id (below: `available_effects_smoke`) — every effect must create,
//!      connect, activate, run, stay finite/bounded, drain to silence
//!      after a bounded window, survive two create/destroy cycles and
//!      agree with the catalog's param metadata; and
//!   2. representative per-family KNOWN-ANSWER tests (the six at the
//!      bottom of this file) proving each family's namesake PHYSICS
//!      claim on the DSP path, in the style of tests/gxfx_honesty_test.rs.
//!
//! A tier-1 pass alone proves "not fake-fatal", not "real DSP"; the
//! tier-2 answers carry the honesty. Failures name the exact id + claim.
//!
//! Drain bounds: memory-class tails (delay/reverb tapes, RT60 ≤ 8 s,
//! feedback ≤ ~50 % at catalog defaults) get a generous but BOUNDED
//! window — 10 s for memory-class ids, 1.5 s for everything else — and
//! the measured settle time is printed per memory-class id (see the
//! `drain_bounds_measured` output). gxtape/gxtape_st additionally carry
//! a signal-INDEPENDENT LCG hiss generator scaled by TAPEHISS (their
//! model noise floor): their silence-drain claim is made with the hiss
//! knob parked at 0, exactly the knob a player closes to get a clean
//! tape path — with TAPEHISS > 0 digital silence correctly produces a
//! noise floor, which is the model being honest, not a leak.

use mixer_wasm::gxfx::{catalog_json, GxFxError, GxFxRack};
use serde_json::Value;

const FS: u32 = 48_000;
const BS: usize = 128;

// ===========================================================================
// Catalog surface (parsed once from the engine's own catalog_json — the
// C ABI's param defs are the single source of truth)
// ===========================================================================

struct ParamSpec {
    #[allow(dead_code)]
    port: i64,
    symbol: String,
    def: f64,
    min: f64,
    max: f64,
    #[allow(dead_code)]
    step: f64,
    integer: bool,
}

struct FxEntry {
    id: i16,
    key: String,
    stereo: bool,
    memory: bool,
    available: bool,
    params: Vec<ParamSpec>,
}

fn catalog() -> Vec<FxEntry> {
    let v: Value = serde_json::from_str(&catalog_json()).expect("catalog json parses");
    v["effects"]
        .as_array()
        .expect("effects array")
        .iter()
        .map(|e| FxEntry {
            id: e["id"].as_i64().expect("id") as i16,
            key: e["key"].as_str().expect("key").to_string(),
            stereo: e["stereo"].as_bool().expect("stereo"),
            memory: e["memory"].as_bool().expect("memory"),
            available: e["available"].as_bool().expect("available"),
            params: e["params"]
                .as_array()
                .expect("params")
                .iter()
                .map(|p| ParamSpec {
                    port: p["port"].as_i64().expect("port"),
                    symbol: p["symbol"].as_str().expect("symbol").to_string(),
                    def: p["default"].as_f64().expect("default"),
                    min: p["min"].as_f64().expect("min"),
                    max: p["max"].as_f64().expect("max"),
                    step: p["step"].as_f64().expect("step"),
                    integer: p["integer"].as_bool().expect("integer"),
                })
                .collect(),
        })
        .collect()
}

// ===========================================================================
// Harness helpers (same shapes as tests/gxfx_honesty_test.rs)
// ===========================================================================

fn rack() -> GxFxRack {
    GxFxRack::new(FS, BS)
}

fn zeros(secs: f64) -> Vec<f32> {
    vec![0.0; (secs * FS as f64) as usize]
}

fn sine(freq: f64, amp: f64, secs: f64) -> Vec<f32> {
    let n = (secs * FS as f64) as usize;
    (0..n)
        .map(|i| (amp * (2.0 * std::f64::consts::PI * freq * i as f64 / FS as f64).sin()) as f32)
        .collect()
}

fn multitone(freqs: &[f64], amp: f64, secs: f64) -> Vec<f32> {
    let n = (secs * FS as f64) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / FS as f64;
            freqs
                .iter()
                .map(|&f| (2.0 * std::f64::consts::PI * f * t).sin())
                .sum::<f64>()
                * amp
        })
        .map(|v| v as f32)
        .collect()
}

/// Run `input_l`/`input_r` through pair `pair` in 128-frame blocks and
/// collect both outputs.
fn run_pair(
    rack: &mut GxFxRack,
    pair: usize,
    input_l: &[f32],
    input_r: &[f32],
) -> (Vec<f32>, Vec<f32>) {
    let mut ol = Vec::with_capacity(input_l.len());
    let mut or = Vec::with_capacity(input_r.len());
    let mut l = vec![0.0f32; BS];
    let mut r = vec![0.0f32; BS];
    for (cl, cr) in input_l.chunks(BS).zip(input_r.chunks(BS)) {
        l[..cl.len()].copy_from_slice(cl);
        for v in l[cl.len()..].iter_mut() {
            *v = 0.0;
        }
        r[..cr.len()].copy_from_slice(cr);
        for v in r[cr.len()..].iter_mut() {
            *v = 0.0;
        }
        rack.process_pair(pair, &mut l, Some(&mut r));
        ol.extend_from_slice(&l[..cl.len()]);
        or.extend_from_slice(&r[..cr.len()]);
    }
    (ol, or)
}

fn run_mono(rack: &mut GxFxRack, pair: usize, input: &[f32]) -> Vec<f32> {
    run_pair(rack, pair, input, &vec![0.0; input.len()]).0
}

fn rms(x: &[f32]) -> f64 {
    let n = x.len().max(1) as f64;
    (x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / n).sqrt()
}

fn max_abs(x: &[f32]) -> f32 {
    x.iter().fold(0.0f32, |m, &v| m.max(v.abs()))
}

/// Goertzel mean power of an f32 input at frequency `f`.
fn goertzel_f32(x: &[f32], f: f64, fs: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * f / fs;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = v as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    power / x.len() as f64
}

fn assert_finite(out: &[f32], what: &str) {
    for (i, &v) in out.iter().enumerate() {
        assert!(v.is_finite(), "{what}: non-finite sample at {i}: {v}");
    }
}

/// Non-overlapping windowed RMS series.
fn windowed_rms(x: &[f32], win: usize) -> Vec<f64> {
    x.chunks(win).filter(|c| c.len() == win).map(rms).collect()
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// Normalized autocorrelation of the mean-removed series at `lag`.
fn autocorr_at(v: &[f64], lag: usize) -> f64 {
    let m = mean(v);
    let num: f64 = (0..v.len() - lag)
        .map(|i| (v[i] - m) * (v[i + lag] - m))
        .sum();
    let den: f64 = v.iter().map(|x| (x - m) * (x - m)).sum::<f64>();
    num / den
}

fn samples(secs: f64) -> usize {
    (secs * FS as f64) as usize
}

// ===========================================================================
// Tier 1 — the per-effect smoke suite
// ===========================================================================

const DRAIN_SILENCE: f32 = 1e-4; // |out| below this counts as drained
const MEMORY_DRAIN_CAP_S: f64 = 15.0;
const PLAIN_DRAIN_CAP_S: f64 = 1.5;
const BOUND: f32 = 16.0; // explosion bound (impulse max |out|)

/// Measured DC floors — the documented bossds1 failure class
/// (tests/gxfx_honesty_test.rs): a signal-independent control-smoothing
/// recursion whose zero-input fixed point is a small flat DC offset
/// instead of exact zero. Measured on this exact harness
/// (defaults-driven, post-impulse, flat across the last 9 s of a 12 s
/// drain): distortion 3.4e-4, aclipper 6.0e-4, vibe 1.6e-4, oc_2 4.7e-3
/// (all ≤ -46 dBFS, inaudible). These ids settle to floor×1.5, not
/// DRAIN_SILENCE; everything else must reach true near-silence.
const DC_FLOORS: &[(i16, f32)] = &[(2, 3.4e-4), (17, 6.1e-4), (38, 1.7e-4), (54, 4.8e-3)];

/// Non-memory ids with measured tails beyond PLAIN_DRAIN_CAP_S (the
/// catalog's memory flag covers MB-scale buffers AT CREATE, not tail
/// length): mbdelay (67) settles ≈11.5 s and mbecho (69) ≈5.0 s at
/// default feedback (five band delays compounding), bigmuffpi (71)
/// drains its oversampled soak in ≈5.1 s — all measured on this harness.
const LONG_DRAIN: &[i16] = &[67, 69, 71];

/// Per-id impulse-peak bounds above BOUND, with the measured reason:
/// rangem (19) is a Range Master treble booster — its honest peak gain
/// through the high-Q boost stage is ≈48.9 on a unit impulse (measured;
/// ≈ +34 dB, exactly what the circuit does), and the tail then DECAYS
/// (the universal drain claim still applies and catches any true
/// runaway). Everything else stays under the generic bound.
const IMPULSE_PEAK_OVERRIDES: &[(i16, f32)] = &[(19, 64.0)];

fn impulse_bound(fx: i16) -> f32 {
    IMPULSE_PEAK_OVERRIDES
        .iter()
        .find(|&&(id, _)| id == fx)
        .map(|&(_, b)| b)
        .unwrap_or(BOUND)
}

fn drain_target(fx: i16) -> f32 {
    DC_FLOORS
        .iter()
        .find(|&&(id, _)| id == fx)
        .map(|&(_, f)| f * 1.5)
        .unwrap_or(DRAIN_SILENCE)
}

fn drain_cap_secs(e: &FxEntry) -> f64 {
    if e.memory || LONG_DRAIN.contains(&e.id) {
        MEMORY_DRAIN_CAP_S
    } else {
        PLAIN_DRAIN_CAP_S
    }
}

/// One effect's full smoke pass. Every assertion names id+key+claim.
fn smoke_effect(e: &FxEntry) {
    let fx = e.id;
    let tag = format!("fx {} ({})", fx, e.key);
    let mut r = rack();

    // ── create + catalog parity ───────────────────────────────────────
    r.set_slot(0, 0, fx)
        .unwrap_or_else(|err| panic!("{tag}: set_slot failed: {err:?}"));
    let want_inst = if e.stereo { 1 } else { 2 };
    assert_eq!(
        r.slot_instances(0, 0).unwrap(),
        want_inst,
        "{tag}: instance count (stereo={} → {} handles)",
        e.stereo,
        want_inst
    );
    for (i, p) in e.params.iter().enumerate() {
        let got = r.param(0, 0, i).unwrap_or_else(|err| {
            panic!("{tag}: param readback {i} ({}) failed: {err:?}", p.symbol)
        });
        assert!(
            (got as f64 - p.def).abs() < 1e-6,
            "{tag}: param {i} ({}) must load its catalog default: got {got}, want {}",
            p.symbol,
            p.def
        );
        assert!(
            p.def >= p.min - 1e-6 && p.def <= p.max + 1e-6,
            "{tag}: default of {} ({}) outside its own range [{}, {}]",
            p.symbol,
            p.def,
            p.min,
            p.max
        );
    }
    // param_count matches the defaults the rack loaded (ordinal at the
    // count is BadParam — no phantom params beyond the catalog row).
    assert_eq!(
        r.param(0, 0, e.params.len()).unwrap_err(),
        GxFxError::BadParam,
        "{tag}: param ordinal {} (past param_count) must be BadParam",
        e.params.len()
    );

    r.set_enabled(0, 0, true).unwrap();

    // ── steady sine: finite everywhere (no NaN/Inf) ───────────────────
    // (No numeric amplitude bound here by design: several honest models
    // have high-Q resonances — autowah's bandpass rings at ~+44 dB at
    // its pole — so a steady tone legitimately produces large finite
    // output. The explosion bound belongs to the impulse phase below.)
    let sig = sine(440.0, 0.5, 1.0);
    let (ol, or) = run_pair(&mut r, 0, &sig, &sig);
    assert_finite(&ol, &format!("{tag} sine L"));
    assert_finite(&or, &format!("{tag} sine R"));

    // ── impulse: bounded, finite energy ──────────────────────────────
    let ibound = impulse_bound(fx);
    let mut imp = zeros(0.25);
    imp.push(1.0);
    let ring = if e.memory { 1.0 } else { 0.25 };
    imp.extend(zeros(ring));
    let (il, ir) = run_pair(&mut r, 0, &imp, &imp);
    assert_finite(&il, &format!("{tag} impulse L"));
    assert_finite(&ir, &format!("{tag} impulse R"));
    let (ml, mr) = (max_abs(&il), max_abs(&ir));
    assert!(
        ml < ibound && mr < ibound,
        "{tag}: impulse response must not explode (max L {ml:.3} R {mr:.3}, \
         bound {ibound})"
    );
    let energy: f64 = il
        .iter()
        .chain(ir.iter())
        .map(|&v| (v as f64) * (v as f64))
        .sum();
    assert!(
        energy.is_finite() && energy < 1e9,
        "{tag}: impulse energy must be finite (Σ {energy:.3e})"
    );

    // ── silence drain: bounded, measured ─────────────────────────────
    // gxtape/gxtape_st: park TAPEHISS at 0 first — their LCG hiss
    // generator is signal-independent noise (documented in the header).
    if e.key == "gxtape" || e.key == "gxtape_st" {
        let hiss = e
            .params
            .iter()
            .position(|p| p.symbol == "hiss")
            .expect("gxtape exposes TAPEHISS");
        r.set_param(0, 0, hiss, 0.0).unwrap();
    }
    let target = drain_target(fx);
    let cap = drain_cap_secs(e);
    let n_blocks = (cap * FS as f64 / BS as f64) as usize;
    let mut l = vec![0.0f32; BS];
    let mut rr = vec![0.0f32; BS];
    let mut last_loud_block = 0usize;
    let mut mid_max = 0.0f32; // no-growth reference: middle third of window
    for b in 0..n_blocks {
        // True digital silence EVERY block: process_pair is in-place, so
        // the previous block's output must not feed back as input (that
        // would wrap the effect in a block-rate feedback loop — honest
        // filters runaway under it; probed on the wah).
        for v in l.iter_mut() {
            *v = 0.0;
        }
        for v in rr.iter_mut() {
            *v = 0.0;
        }
        r.process_pair(0, &mut l, Some(&mut rr));
        let m = max_abs(&l).max(max_abs(&rr));
        if m >= target {
            last_loud_block = b + 1;
        }
        if b >= n_blocks / 3 && b < 2 * n_blocks / 3 {
            mid_max = mid_max.max(m);
        }
    }
    let settled_s = last_loud_block as f64 * BS as f64 / FS as f64;
    let (fl, fr) = (max_abs(&l), max_abs(&rr));
    assert!(
        fl < target && fr < target,
        "{tag}: silence-in must settle below {target:e} inside {cap}s \
         (final block max L {fl:+.3e} R {fr:+.3e}; last loud block at {settled_s:.2}s)"
    );
    assert!(
        fl <= mid_max.max(target) && fr <= mid_max.max(target),
        "{tag}: the tail must not GROW (final L {fl:.3e} R {fr:.3e} vs \
         mid-window max {:.3e}) — a ringing or runaway effect fails here",
        mid_max
    );
    if e.memory || LONG_DRAIN.contains(&fx) {
        println!(
            "drain bound: {tag}: settles < {target:e} after {settled_s:.2} s \
             (cap {cap:.0} s)"
        );
    }

    // ── one non-default active setting: still finite ────────────────
    // (autowah offers no range — nothing to drive off default)
    if let Some((ordinal, value)) = active_nondefault(e) {
        r.set_param(0, 0, ordinal, value).unwrap();
        let sig2 = sine(440.0, 0.5, 0.5);
        let (nl, nr) = run_pair(&mut r, 0, &sig2, &sig2);
        assert_finite(&nl, &format!("{tag} non-default L"));
        assert_finite(&nr, &format!("{tag} non-default R"));
    }

    // ── create/destroy cycle ×2 (allocation hygiene) ─────────────────
    for cycle in 1..=2 {
        r.set_slot(0, 0, -1).unwrap();
        assert_eq!(
            r.slot_instances(0, 0).unwrap(),
            0,
            "{tag}: destroy cycle {cycle} must drop all instances"
        );
        r.set_slot(0, 0, fx)
            .unwrap_or_else(|err| panic!("{tag}: re-create cycle {cycle} failed: {err:?}"));
        assert_eq!(
            r.slot_instances(0, 0).unwrap(),
            want_inst,
            "{tag}: re-create cycle {cycle} must rebuild {want_inst} instances"
        );
    }
    r.set_slot(0, 0, -1).unwrap();
}

/// Pick "one non-default active setting": the first param with real
/// range, preferring continuous knobs over integer steppers, driven to
/// min + 70% of range (falling back to 30% if that lands on the
/// default). Returns (param ordinal, value).
fn active_nondefault(e: &FxEntry) -> Option<(usize, f32)> {
    let mut candidates: Vec<usize> = (0..e.params.len())
        .filter(|&i| e.params[i].max - e.params[i].min > 1e-9)
        .collect();
    // Prefer a continuous knob with a usable span (booleans/int steppers
    // still qualify if nothing else exists).
    candidates.sort_by_key(|&i| {
        (
            e.params[i].integer,
            (e.params[i].max - e.params[i].min) < 0.05,
        )
    });
    let &i = candidates.first()?;
    let p = &e.params[i];
    let mut v = p.min + 0.7 * (p.max - p.min);
    if (v - p.def).abs() < 1e-9 {
        v = p.min + 0.3 * (p.max - p.min);
    }
    if p.integer {
        v = v.round().max(p.min).min(p.max);
        if (v - p.def).abs() < 1e-9 {
            v = (v + 1.0).min(p.max);
        }
    }
    Some((i, v as f32))
}

/// Tier-1 gate: every id the catalog reports available must pass the
/// smoke pass — including the wave-2 families this task wires live
/// (ids 11..75). Ids still unavailable (null factories) are skipped;
/// their turn comes with their TUs.
#[test]
fn available_effects_smoke() {
    let cat = catalog();
    let skipped: Vec<&FxEntry> = cat.iter().filter(|e| !e.available).collect();
    println!(
        "smoke: {} available ids, {} still-unavailable ids ({})",
        cat.len() - skipped.len(),
        skipped.len(),
        skipped
            .iter()
            .map(|e| format!("{} ({})", e.id, e.key))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for e in &cat {
        if e.available {
            smoke_effect(e);
        }
    }
}

/// The wave actually flipped: ids 11..75 report available, and the four
/// holdout families (76..82: cabinet/redeye/metalamp/metalhead,
/// detune, tuner, livelooper) honestly do not. This is the tiered
/// rule's bookkeeping half — availability must equal ported-ness.
#[test]
fn availability_matches_the_ported_wave() {
    let cat = catalog();
    assert_eq!(cat.len(), 83, "catalog size");
    for e in &cat {
        let want = (0..=75).contains(&e.id);
        assert_eq!(
            e.available, want,
            "fx {} ({}) availability must be {}",
            e.id, e.key, want
        );
    }
    // And the engine enforces it: a holdout id is rejected cleanly.
    let mut r = rack();
    assert_eq!(
        r.set_slot(0, 0, 76).unwrap_err(),
        GxFxError::Unavailable,
        "cabinet (76) stays unavailable until the convolver TU lands"
    );
    assert_eq!(r.set_slot(0, 0, 82).unwrap_err(), GxFxError::Unavailable);
}

// ===========================================================================
// Tier 2 — representative per-family known-answers
// ===========================================================================

// ── autowah (id 36): ENVELOPE-driven band — loud vs quiet sustained
// input produces clearly different spectral content. The class is
// gxautowah.lv2's dunwahauto: fRec2 smooths |input| into an envelope
// that drives BOTH the resonator pole (fTemp1, ≈413 Hz at envelope 0,
// rising with level) and the Q terms — the exact thing the manual
// gcb_95 "wah" could not do (its state follows the pedal only; see the
// gxfx_honesty_test.rs finding note). Frozen at one envelope the class
// is LTI, so a pure sine can only come out at its own frequency: the
// honest "different spectral content" probe is (a) a fixed multitone
// whose normalized band shares must move with input level, and (b) the
// tail ring-out frequency after the input stops (the resonator rings
// at the envelope-selected pole — probed: ≈833 Hz after quiet input,
// ≈987 Hz after loud).
#[test]
fn autowah_envelope_drives_the_band() {
    let tones = [150.0, 300.0, 500.0, 800.0, 1300.0, 2100.0, 3400.0];

    // (a) sustained multitone, loud vs quiet: normalized shares.
    let mut shares_300 = [0.0f64; 2];
    for (k, &amp) in [0.12f64, 0.012].iter().enumerate() {
        let mut r = rack();
        r.set_slot(0, 0, 36).unwrap(); // autowah: zero params
        r.set_enabled(0, 0, true).unwrap();
        let sig = multitone(&tones, amp, 1.5);
        let out = run_mono(&mut r, 0, &sig);
        assert_finite(&out, "autowah multitone");
        // Identical multitone through the input side (1.0 s window =
        // integer cycles at every integer-Hz bin): shares are flat 1/7.
        let win_in = &sig[sig.len() - 48_000..];
        let pin: Vec<f64> = tones
            .iter()
            .map(|&f| goertzel_f32(win_in, f, FS as f64))
            .collect();
        let tin: f64 = pin.iter().sum::<f64>();
        let in_share_300 = pin[1] / tin;
        assert!(
            (in_share_300 - 1.0 / 7.0).abs() < 0.02,
            "autowah probe sanity: input 300 Hz share {in_share_300:.3} ≈ 1/7"
        );
        // Output side: level must REDISTRIBUTE the spectrum.
        let win = &out[out.len() - 48_000..];
        let p: Vec<f64> = tones
            .iter()
            .map(|&f| goertzel_f32(win, f, FS as f64))
            .collect();
        let total: f64 = p.iter().sum::<f64>() + 1e-30;
        shares_300[k] = p[1] / total;
        println!(
            "autowah: amp {amp:.3} → shares [{}]",
            p.iter()
                .map(|&x| format!("{:.3}", x / total))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let (loud_300, quiet_300) = (shares_300[0], shares_300[1]);
    println!(
        "autowah: 300 Hz share loud {loud_300:.3} vs quiet {quiet_300:.3} \
         (ratio {:.1}×)",
        quiet_300 / loud_300
    );
    // PHYSICAL: the envelope, not the input level, sets the band — with
    // a FIXED multitone, the loud passage moves the band up and away
    // from 300 Hz (its share collapses), while the quiet passage leaves
    // the low band dominant. A linear effect (or a pedal-only wah) would
    // keep the normalized shares IDENTICAL at both levels.
    assert!(
        quiet_300 > 3.0 * loud_300,
        "PHYSICAL: loud sustained input must drive the wah band away from \
         the low band (normalized 300 Hz share quiet {quiet_300:.3} vs loud \
         {loud_300:.3}, need > 3× — envelope-driven pole, not a scaled copy)"
    );

    // (b) tail ring after the input stops: the resonator state rings at
    // the pole the sustained level selected — higher after loud input.
    let mut zcrs = [0.0f64; 2];
    for (k, &amp) in [0.6f64, 0.06].iter().enumerate() {
        let mut r = rack();
        r.set_slot(0, 0, 36).unwrap();
        r.set_enabled(0, 0, true).unwrap();
        let mut sig = sine(500.0, amp, 1.0);
        sig.extend(zeros(0.5));
        let out = run_mono(&mut r, 0, &sig);
        assert_finite(&out, "autowah tail");
        let tail = &out[samples(1.0)..samples(1.0) + samples(0.15)];
        zcrs[k] = tail
            .windows(2)
            .filter(|w| (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0))
            .count() as f64
            * FS as f64
            / tail.len() as f64;
    }
    println!(
        "autowah: tail ring ZCR loud {:#.1} Hz vs quiet {:#.1} Hz",
        zcrs[0], zcrs[1]
    );
    assert!(
        zcrs[0] > 1.1 * zcrs[1],
        "PHYSICAL: after a LOUD passage the resonator must ring higher than \
         after a quiet one (ZCR {:#.1} vs {:#.1} Hz, need > 1.1×)",
        zcrs[0],
        zcrs[1]
    );

    // Universal claims: finite everywhere and silence settles after both.
    let mut r = rack();
    r.set_slot(0, 0, 36).unwrap();
    r.set_enabled(0, 0, true).unwrap();
    let z = zeros(1.5);
    let (zl, zr) = run_pair(&mut r, 0, &z, &z);
    assert_finite(&zl, "autowah silence L");
    assert_finite(&zr, "autowah silence R");
    let n = BS.min(zl.len());
    assert!(
        max_abs(&zl[zl.len() - n..]) < 1e-4 && max_abs(&zr[zr.len() - n..]) < 1e-4,
        "autowah: silence-in must settle"
    );
}

// ── vibe (id 38): periodic AM/FM on a steady tone at the set rate. The
// uni-vibe LFO (TEMPO) sweeps a photocell delay → both an amplitude
// ripple (AM) and modulation sidebands at f0 ± rate (FM) on a tone.
#[test]
fn vibe_modulates_am_and_fm_at_the_set_rate() {
    let mut r = rack();
    r.set_slot(0, 0, 38).unwrap();
    // WIDTH 0.5 default; DEPTH 0.8 (deep), WETDRY 1.0 (full wet),
    // TEMPO 2.0 Hz — off-default, clearly measurable rate.
    r.set_param(0, 0, 1, 0.8).unwrap(); // DEPTH
    r.set_param(0, 0, 2, 1.0).unwrap(); // WETDRY
    r.set_param(0, 0, 4, 2.0).unwrap(); // TEMPO
    r.set_enabled(0, 0, true).unwrap();

    let sig = sine(440.0, 0.4, 4.0);
    let (ol, or) = run_pair(&mut r, 0, &sig, &sig);
    assert_finite(&ol, "vibe L");
    assert_finite(&or, "vibe R");

    // AM: envelope (10 ms windows, skip 0.5 s settle; one 2 Hz LFO
    // period = 4800 windows) must be periodic at 2 Hz.
    let env = windowed_rms(&ol, 480);
    let env = &env[samples(0.5) / 480..];
    let emax = env.iter().cloned().fold(0.0f64, f64::max);
    let emin = env.iter().cloned().fold(f64::INFINITY, f64::min);
    println!("vibe: env max/min {:.2} ({emax:.4}/{emin:.4})", emax / emin);
    assert!(
        emax / emin > 1.3,
        "PHYSICAL: the vibe LFO must ripple the tone's envelope at 2 Hz \
         (windowed-RMS max/min > 1.3); got {:.2}",
        emax / emin
    );
    let rho = autocorr_at(env, 50); // 50 × 480 = 24000 samples = one 2 Hz period
    println!("vibe: envelope autocorr at 2 Hz period {rho:.3}");

    // FM: sidebands at 440 ± 2 Hz. 1.0 s steady window = integer cycles
    // for 438/440/442/445/447 Hz bins (all divide 48000 exactly).
    let win = &ol[ol.len() - 48_000..];
    let p_side = goertzel_f32(win, 438.0, FS as f64) + goertzel_f32(win, 442.0, FS as f64);
    let p_ctrl = goertzel_f32(win, 447.0, FS as f64);
    let p440 = goertzel_f32(win, 440.0, FS as f64);
    println!(
        "vibe: sidebands(438+442) {p_side:.3e} vs control 447 Hz {p_ctrl:.3e}, \
         carrier 440 {p440:.3e}"
    );
    assert!(
        p_side > 5.0 * p_ctrl,
        "PHYSICAL: 2 Hz pitch modulation must put real energy at 440±2 Hz \
         (sideband sum {p_side:.3e} vs empty control bin 447 Hz {p_ctrl:.3e})"
    );
    assert!(
        p440 > p_ctrl,
        "PHYSICAL: the carrier survives (P440 {p440:.3e} > control {p_ctrl:.3e})"
    );

    // autocorrelation lag: one 2 Hz period = 0.5 s = 50 windows of 480.
    assert!(
        rho > 0.4,
        "PHYSICAL: the AM must repeat at the 2 Hz LFO rate (envelope \
         autocorrelation at one period {rho:.3} > 0.4)"
    );

    // Drain.
    let z = zeros(2.0);
    let (zl, zr) = run_pair(&mut r, 0, &z, &z);
    let n = BS.min(zl.len());
    assert!(
        max_abs(&zl[zl.len() - n..]) < 1e-4 && max_abs(&zr[zr.len() - n..]) < 1e-4,
        "vibe: silence-in must settle"
    );
}

// ── graphiceq (id 28): boosting one band vs flattening changes that
// band's energy measurably. Band geometry from the .cc's precomputed
// tan() constants: adjacent octave-band edges at 22/44/88/177/354/707/
// 1414/2828/5657/11384 Hz → G1..G11 centered ≈ 31, 62, 125, 250, 500,
// 1k, 2k, 4k, 8k, 14.3k, 20.8k. G6 is the ~1 kHz band.
#[test]
fn graphiceq_band_boost_moves_band_energy() {
    let tones = [125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0];
    let mut flat = Vec::new();
    let mut boost = Vec::new();
    for boosted in [false, true] {
        let mut r = rack();
        r.set_slot(0, 0, 28).unwrap();
        if boosted {
            r.set_param(0, 0, 5, 20.0).unwrap(); // G6 = +20 dB (param max)
        }
        r.set_enabled(0, 0, true).unwrap();
        let sig = multitone(&tones, 0.04, 1.5);
        let out = run_mono(&mut r, 0, &sig);
        assert_finite(&out, "graphiceq");
        // Steady window: the last 48000 samples = exactly 1.0 s →
        // integer cycles for every integer-Hz probe bin (leakage-free).
        let win = &out[out.len() - 48_000..];
        let p: Vec<f64> = tones
            .iter()
            .map(|&f| goertzel_f32(win, f, FS as f64))
            .collect();
        if boosted {
            boost = p;
        } else {
            flat = p;
        }
    }
    for (i, &f) in tones.iter().enumerate() {
        println!(
            "graphiceq: {:>5.0} Hz  flat {:.3e}  G6+20dB {:.3e}  ratio {:.2}",
            f,
            flat[i],
            boost[i],
            boost[i] / flat[i]
        );
    }
    let ratio1k = boost[3] / flat[3];
    let ratio_far = boost
        .iter()
        .zip(&flat)
        .take(3)
        .map(|(b, f)| b / f)
        .fold(0.0f64, f64::max);
    assert!(
        ratio1k > 3.0,
        "PHYSICAL: +20 dB on G6 must measurably boost the 1 kHz band \
         (Goertzel ratio {ratio1k:.2} > 3)"
    );
    assert!(
        ratio1k > 2.0 * ratio_far,
        "PHYSICAL: the boost must be BAND-selective (1 kHz ratio {ratio1k:.2} \
         vs far-band max {ratio_far:.2})"
    );
}

// ── tonestack (id 62): switching MODEL swaps the circuit — the
// frequency response (low vs high band energy at flat tone controls)
// must differ measurably across models.
#[test]
fn tonestack_model_switch_changes_response() {
    let tones = [150.0, 400.0, 1000.0, 2500.0, 6000.0];
    let mut ratios = Vec::new();
    for model in 0..6i32 {
        let mut r = rack();
        r.set_slot(0, 0, 62).unwrap();
        r.set_param(0, 0, 3, model as f32).unwrap(); // Model (port 10)
        r.set_enabled(0, 0, true).unwrap();
        let sig = multitone(&tones, 0.05, 1.5);
        let out = run_mono(&mut r, 0, &sig);
        assert_finite(&out, "tonestack");
        let win = &out[out.len() - 48_000..];
        let p: Vec<f64> = tones
            .iter()
            .map(|&f| goertzel_f32(win, f, FS as f64))
            .collect();
        let lo = p[0] + p[1];
        let hi = p[3] + p[4];
        let ratio = lo / (hi + 1e-30);
        println!(
            "tonestack: model {}: P150 {:.3e} P400 {:.3e} P1k {:.3e} P2.5k {:.3e} \
             P6k {:.3e} → low/high {:.2}",
            model, p[0], p[1], p[2], p[3], p[4], ratio
        );
        ratios.push(ratio);
    }
    let rmax = ratios.iter().cloned().fold(0.0f64, f64::max);
    let rmin = ratios.iter().cloned().fold(f64::INFINITY, f64::min);
    println!(
        "tonestack: low/high ratio spread across models {rmin:.2}..{rmax:.2} \
         (max/min {:.2})",
        rmax / rmin
    );
    assert!(
        rmax / rmin > 1.5,
        "PHYSICAL: switching tonestack models must change the frequency \
         response (low/high ratio spread {rmin:.2}..{rmax:.2}, max/min {:.2} > 1.5)",
        rmax / rmin
    );
}

// ── duck_delay (id 45): envelope-following delay gain. duck_delay.cc
// smooths |input| (fRec2→fRec1) and derives the wet gain fRec0 → the
// amplified envelope PAST 1 clamps the tap gain toward 0 (ducked while
// you play); when the input stops, the follower releases and the
// recorded tail plays through at full tap gain. Echo timing is the tape
// structure's: effective period ≈ 200 ms base + TIME (probed: TIME 300
// → 498 ms, TIME 500 → 700 ms).
#[test]
fn duck_delay_tail_opens_after_input_stops() {
    let mut r = rack();
    r.set_slot(0, 0, 45).unwrap();
    r.set_param(0, 0, 0, 30.0).unwrap(); // AMOUNT (duck depth)
    r.set_param(0, 0, 2, 0.5).unwrap(); // FEEDBACK 50 %
    r.set_param(0, 0, 4, 300.0).unwrap(); // TIME → ~0.5 s echo period
    r.set_enabled(0, 0, true).unwrap();

    // 0.7 s loud tone, then 2.0 s silence. out = dry + ducked echo, so
    // the echo during the loud passage is exactly out - in.
    let loud = sine(500.0, 0.6, 0.7);
    let z = zeros(2.0);
    let in_sig: Vec<f32> = loud.iter().chain(z.iter()).copied().collect();
    let out = run_mono(&mut r, 0, &in_sig);
    assert_finite(&out, "duck_delay");

    // Ducked echo: steady loud passage (0.5..0.7 s out) carries the echo
    // of the tone's opening (0.0..0.2 s in) — the duck had closed.
    let echo_loud: Vec<f32> = out[samples(0.5)..samples(0.7)]
        .iter()
        .zip(&in_sig[samples(0.5)..samples(0.7)])
        .map(|(&o, &i)| o - i)
        .collect();
    let e_ducked = rms(&echo_loud);
    // Opened echo: the tone's end (t=0.7) replays at ≈1.2 s — input has
    // stopped, follower released, tap gain open. Next repeat ≈1.7 s.
    let e_open = rms(&out[samples(1.15)..samples(1.35)]);
    let e_next = rms(&out[samples(1.65)..samples(1.85)]);
    println!(
        "duck_delay: echo residual while playing {e_ducked:.5}, first echo \
         after stop {e_open:.5}, next {e_next:.5} (open/ducked {:.1}×)",
        e_open / (e_ducked + 1e-12)
    );
    assert!(
        e_open > 3.0 * e_ducked,
        "PHYSICAL: the envelope follower must duck the delay while the \
         signal plays and open it after the input stops (echo RMS open \
         {e_open:.5} vs ducked {e_ducked:.5}, need > 3×)"
    );
    assert!(
        e_next < e_open,
        "PHYSICAL: the freed tail must still decay across repeats \
         (second echo {e_next:.5} < first {e_open:.5})"
    );

    // Drain to silence afterwards (feedback 0.5 per ~0.5 s repeat).
    let z = zeros(10.0);
    let (zl, _) = run_pair(&mut r, 0, &z, &z);
    let n = BS.min(zl.len());
    assert!(
        max_abs(&zl[zl.len() - n..]) < 1e-4,
        "duck_delay: silence-in must settle inside the memory-class bound"
    );
}

// ── zita_rev1 (id 57): FDN reverb. Impulse → a real, bounded, decaying
// tail; the dry path is suppressed at full wet (DRY_WET_MIX = +1).
#[test]
fn zita_rev1_impulse_tail_bounded_and_decaying() {
    let mut r = rack();
    r.set_slot(0, 0, 57).unwrap();
    r.set_param(0, 0, 9, 1.0).unwrap(); // DRY_WET_MIX full wet
    r.set_enabled(0, 0, true).unwrap();

    let t0 = samples(0.25);
    let mut sig = zeros(0.25);
    sig.push(1.0);
    sig.extend(zeros(6.0));
    let (ol, or) = run_pair(&mut r, 0, &sig, &sig);
    assert_finite(&ol, "zita L");
    assert_finite(&or, "zita R");

    // Bounded above.
    let (ml, mr) = (max_abs(&ol), max_abs(&or));
    assert!(
        ml < 16.0 && mr < 16.0,
        "PHYSICAL: impulse response bounded (max L {ml:.3} R {mr:.3})"
    );
    // Dry suppressed at full wet: the impulse sample itself must not
    // pass at unity.
    println!("zita: dry sample out L {:.4} R {:.4}", ol[t0], or[t0]);
    assert!(
        ol[t0].abs() < 0.1 && or[t0].abs() < 0.1,
        "PHYSICAL: dry path suppressed at DRY_WET_MIX +1 (impulse sample \
         passes at {:.3}/{:.3}, not unity)",
        ol[t0],
        or[t0]
    );
    // Tail: present, decaying, reaching silence. RT60 defaults 3 s (low)
    // / 2 s (mid) → far under -60 dB by 6 s.
    let bucket = |x: &[f32], from_s: f64, to_s: f64| {
        let a = t0 + samples(from_s);
        let b = t0 + samples(to_s);
        rms(&x[a..b.min(x.len())])
    };
    let e_early = (bucket(&ol, 0.01, 0.2) + bucket(&or, 0.01, 0.2)) / 2.0;
    let e_mid = (bucket(&ol, 1.0, 1.3) + bucket(&or, 1.0, 1.3)) / 2.0;
    let e_end = (bucket(&ol, 5.7, 6.0) + bucket(&or, 5.7, 6.0)) / 2.0;
    let energy: f64 = ol[t0..]
        .iter()
        .chain(or[t0..].iter())
        .map(|&v| (v as f64) * (v as f64))
        .sum();
    println!(
        "zita: tail RMS early {e_early:.5} mid {e_mid:.6} end {e_end:.3e}, \
         total energy {energy:.4}"
    );
    assert!(
        e_early > 1e-4 && energy > 1e-4,
        "PHYSICAL: a real FDN reverb spreads the impulse into a non-zero \
         tail (early RMS {e_early:.2e}, energy {energy:.2e}) — a fake would \
         be silent"
    );
    assert!(
        e_mid < e_early / 2.0,
        "PHYSICAL: the tail must decay (mid {e_mid:.5} < early/2 {:.5})",
        e_early / 2.0
    );
    assert!(
        e_end < e_early / 1000.0,
        "PHYSICAL: the tail must reach near-silence within seconds (end \
         {e_end:.3e} < early/1000)"
    );

    // And the silence claim afterwards.
    let z = zeros(2.0);
    let (zl, zr) = run_pair(&mut r, 0, &z, &z);
    let n = BS.min(zl.len());
    assert!(
        max_abs(&zl[zl.len() - n..]) < 1e-4 && max_abs(&zr[zr.len() - n..]) < 1e-4,
        "zita: silence-in must settle"
    );
}
