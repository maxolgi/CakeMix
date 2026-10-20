//! gxfx FX-chain integration into the MixerWasm console (native, no JS).
//!
//! Covers the `fx_*` API validation surface (owner/slot/effect ranges,
//! cancel-on-set during a fade), scene capture/recall of chain state
//! (instant and timed fade — FX snaps once, never interpolates), the
//! `fxChains` block of the console-params JSON, and an end-to-end
//! process_block proof of the insert point (staging → FX →
//! meter/gain ordering, via a delayed impulse at the master output).
//!
//! Run: cargo test -p mixer-wasm --test gxfx_console_test

use mixer_wasm::gxfx::{CHAIN_PAIRS, CHAIN_SLOTS};
use mixer_wasm::MixerWasm;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;
const BS: usize = BLOCK as usize;
/// One processing block in ms (128 frames @ 48 kHz).
const BLOCK_MS: f64 = BLOCK as f64 * 1000.0 / SR as f64;

fn mixer() -> MixerWasm {
    MixerWasm::new(SR, BLOCK, 256).expect("ctor")
}

// ── API validation ─────────────────────────────────────────────────────

/// The fx_* setters validate their addressing exactly like the rest of
/// the console: pair owner must be an even input strip, slots 0..11,
/// effect ids -1..=catalog max; the rack's unavailable-effect error
/// surfaces; rejected calls leave the chain untouched.
#[test]
fn fx_api_validates_channel_slot_and_effect() {
    let mut m = mixer();
    // The chain lives on the rack regardless, but the SNAPSHOT (and
    // everything else this test asserts through) only covers pairs
    // whose owner strip exists — create strip 0.
    m.set_channel_gain(0, 1.0).unwrap();
    // Owner: even input strip below 128.
    assert!(m.fx_set_slot(1, 0, 0).is_err(), "odd strip rejected");
    assert!(m.fx_set_slot(129, 0, 0).is_err(), "bus slot rejected");
    assert!(m.fx_set_slot(128, 0, 0).is_err(), "strip 128 rejected");
    assert!(
        m.fx_set_slot(130, 0, 0).is_err(),
        "even but >= 128 rejected"
    );
    // Slot range 0..CHAIN_SLOTS.
    assert!(
        m.fx_set_slot(0, CHAIN_SLOTS as u32, 0).is_err(),
        "slot 11 rejected"
    );
    assert!(m.fx_set_slot(0, 99, 0).is_err(), "slot 99 rejected");
    // Effect id range -1..=10.
    assert!(m.fx_set_slot(0, 0, 11).is_err(), "fx id 11 rejected");
    assert!(m.fx_set_slot(0, 0, -2).is_err(), "fx id -2 rejected");
    assert!(
        m.fx_set_slot(0, 0, 100_000).is_err(),
        "huge fx id rejected before the i16 cast"
    );
    // Distortion (id 2) landed with its TU: it sets like any effect
    // (then cleared, so the untouched-chain sweep below stays uniform).
    m.fx_set_slot(0, 0, 2).expect("distortion sets");
    m.fx_set_slot(0, 0, -1).expect("...and clears");

    // A valid set works; param/enabled/move share the validation shape.
    m.fx_set_slot(0, 4, 8).expect("valid set"); // delay (7 live params)
    assert!(m.fx_set_enabled(0, CHAIN_SLOTS as u32, true).is_err());
    assert!(m.fx_set_enabled(1, 0, true).is_err());
    assert!(
        m.fx_set_param(0, 4, 7, 1.0).is_err(),
        "param ordinal 7 of delay rejected"
    );
    assert!(
        m.fx_set_param(0, 5, 0, 1.0).is_err(),
        "empty slot has no params"
    );
    assert!(m.fx_move_slot(0, 4, CHAIN_SLOTS as u32).is_err());
    assert!(m.fx_move_slot(131, 0, 1).is_err());
    m.fx_move_slot(0, 4, 2).expect("valid move");

    // Every rejected call left the chain untouched: the delay moved to
    // slot 2, everything else is empty.
    let snap = m.console_snapshot();
    assert_eq!(snap.fx_chains[0][2].fx, 8, "delay moved to slot 2");
    assert_eq!(snap.fx_chains[0][4].fx, -1, "slot 4 emptied by the move");
    for p in 0..CHAIN_PAIRS {
        for s in 0..CHAIN_SLOTS {
            if (p, s) == (0, 2) {
                continue;
            }
            assert_eq!(snap.fx_chains[p][s].fx, -1, "pair {p} slot {s} untouched");
        }
    }

    // The catalog JSON parses (wasm surface delegates to the rack's
    // serializer, pinned in gxfx's own unit tests).
    let v: serde_json::Value = serde_json::from_str(&m.fx_catalog_json()).expect("catalog json");
    assert_eq!(v["slots"], CHAIN_SLOTS as i64);
    assert_eq!(v["effects"].as_array().unwrap().len(), 11);
}

/// fx_* writes are scene-affecting setters: they cancel an in-progress
/// cross-fade exactly like a fader move (and the FX state they wrote
/// survives — the fade never resumes).
#[test]
fn fx_set_cancels_in_progress_fade() {
    let mut m = mixer();
    m.set_limiter_enabled(false);
    // Target scene: gain 0.25 with a delay enabled on pair 0.
    m.set_channel_gain(0, 0.25).unwrap();
    m.fx_set_slot(0, 0, 8).unwrap();
    m.fx_set_enabled(0, 0, true).unwrap();
    let id = m.save_scene();
    // From-state: gain 1.0, chain mutated (slot 1 set, slot 0 emptied).
    m.set_channel_gain(0, 1.0).unwrap();
    m.fx_set_slot(0, 0, -1).unwrap();
    m.fx_set_slot(0, 1, 0).unwrap();

    m.recall_scene_fade(id, 500.0).expect("fade recall");

    // FX snapped to the target at fade START (before any block runs).
    let snap = m.console_snapshot();
    assert_eq!(snap.fx_chains[0][0].fx, 8);
    assert!(snap.fx_chains[0][0].enabled);
    assert_eq!(snap.fx_chains[0][1].fx, -1, "target chain applied at start");
    // The fade is live: params JSON shows the fade TARGET gain.
    let v: serde_json::Value = serde_json::from_str(&m.console_params_json_inner()).unwrap();
    assert_eq!(v["strips"][0]["gain"].as_f64().unwrap() as f32, 0.25);

    // A user fx write cancels the fade: the JSON falls back to the
    // CURRENT state (gain 1.0, not the target 0.25).
    m.fx_set_param(0, 0, 4, 333.0).unwrap();
    let v: serde_json::Value = serde_json::from_str(&m.console_params_json_inner()).unwrap();
    assert_eq!(
        v["strips"][0]["gain"].as_f64().unwrap() as f32,
        1.0,
        "fx write must cancel the fade"
    );
    // The user's write survived (no further fade application).
    assert_eq!(m.console_snapshot().fx_chains[0][0].params[4], 333.0);
}

// ── Scenes ─────────────────────────────────────────────────────────────

/// Scene save → mutate → recall restores each pair's chain exactly
/// (ids, enabled flags, param rows); a pair whose owner strip the
/// scene never saw is left alone (the strips-never-destroyed /
/// post-save-strips-are-stream-state rule the per-strip state follows).
#[test]
fn scene_round_trips_fx_chains_instant() {
    let mut m = mixer();
    m.set_limiter_enabled(false);
    // Pair 0 (strips 0/1): owner exists via strip 0; pair 1 owner via
    // strip 2. Pair 10 (strips 20/21) never exists below.
    m.set_channel_gain(0, 0.5).unwrap();
    m.set_channel_gain(2, 0.5).unwrap();

    m.fx_set_slot(0, 3, 8).unwrap(); // delay, enabled, custom time
    m.fx_set_param(0, 3, 4, 333.0).unwrap();
    m.fx_set_enabled(0, 3, true).unwrap();
    m.fx_set_slot(0, 5, 0).unwrap(); // wah, DISABLED, custom param
    m.fx_set_param(0, 5, 1, 0.75).unwrap();
    m.fx_set_slot(20, 0, 10).unwrap(); // reverb on an unseen pair (inert)

    let snap = m.console_snapshot();
    let id = m.save_scene();
    assert_eq!(m.console_snapshot(), snap, "save must not disturb state");
    // The capture read what the setters wrote.
    assert_eq!(snap.fx_chains[0][3].fx, 8);
    assert!(snap.fx_chains[0][3].enabled);
    assert_eq!(snap.fx_chains[0][3].params[4], 333.0);
    assert_eq!(snap.fx_chains[0][5].fx, 0);
    assert!(!snap.fx_chains[0][5].enabled);
    assert_eq!(snap.fx_chains[0][5].params[1], 0.75);
    assert_eq!(snap.fx_chains[1][0].fx, -1, "pair 1 chain is empty");
    assert_eq!(
        snap.fx_chains[10][0].fx, -1,
        "non-existent-owner pair is not captured"
    );

    // Mutate every chain the console can see.
    m.fx_set_slot(0, 3, -1).unwrap();
    m.fx_set_enabled(0, 5, true).unwrap();
    m.fx_set_slot(0, 7, 9).unwrap();
    m.fx_set_param(0, 7, 1, 0.123).unwrap();
    m.fx_set_enabled(0, 7, true).unwrap();
    m.fx_set_slot(20, 1, 8).unwrap(); // unseen pair mutated too
    assert_ne!(m.console_snapshot(), snap, "mutation must change state");

    m.recall_scene(id).expect("recall");
    assert_eq!(
        m.console_snapshot(),
        snap,
        "recall must restore every chain exactly (ids, enabled, params)"
    );
    // Idempotent (the identical-id path keeps the live DSP).
    m.recall_scene(id).expect("re-recall");
    assert_eq!(m.console_snapshot(), snap);

    // The unseen pair-10 chain was LEFT ALONE through both recalls —
    // creating its owner strip now surfaces the mutated chain, not
    // anything the scene held.
    m.set_channel_gain(20, 1.0).unwrap();
    let s10 = &m.console_snapshot().fx_chains[10];
    assert_eq!(s10[0].fx, 10, "pre-save unseen slot survives recall");
    assert_eq!(s10[1].fx, 8, "post-save unseen mutation survives too");
}

/// Timed-fade recall snaps the FX chains once at fade start (they
/// never interpolate) and the fade's per-block driver never touches
/// them again; the interpolated parameters still land on the target.
#[test]
fn scene_fade_snaps_fx_once_at_start() {
    let mut m = mixer();
    m.set_limiter_enabled(false);
    m.set_channel_gain(0, 1.0).unwrap(); // target gain 1.0
    m.fx_set_slot(0, 2, 8).unwrap();
    m.fx_set_param(0, 2, 4, 250.0).unwrap();
    m.fx_set_enabled(0, 2, true).unwrap();
    let id = m.save_scene();
    // From-state: gain 0.125, chain emptied.
    m.fx_set_slot(0, 2, -1).unwrap();
    m.set_channel_gain(0, 0.125).unwrap();

    m.recall_scene_fade(id, 4.0 * BLOCK_MS)
        .expect("fade recall");
    // FX snapped at fade START — before a single block has run.
    let snap = m.console_snapshot();
    let s = &snap.fx_chains[0][2];
    assert_eq!(s.fx, 8);
    assert!(s.enabled);
    assert_eq!(s.params[4], 250.0);

    // Run the fade out: FX stays snapped (never re-touched), the
    // interpolated gain lands on the target verbatim.
    for _ in 0..8 {
        m.process_block(BLOCK).expect("process");
    }
    let snap = m.console_snapshot();
    assert_eq!(snap.fx_chains[0][2].fx, 8);
    assert_eq!(snap.fx_chains[0][2].params[4], 250.0);
    assert!(snap.fx_chains[0][2].enabled);
    assert_eq!(snap.strips[0].gain, 1.0, "fade end state is the target");
}

// ── Console-params JSON ────────────────────────────────────────────────

/// The console JSON carries the FX chains in the shape the scene
/// stores (per pair, per slot: id + enabled + params), with only the
/// LIVE param ordinals on the wire; pairs without any effect are
/// absent (absent pair = empty chain).
#[test]
fn params_json_carries_fx_chains() {
    let mut m = mixer();
    m.set_channel_gain(0, 0.5).unwrap(); // pair 0 owner exists
    m.fx_set_slot(0, 3, 8).unwrap();
    m.fx_set_param(0, 3, 4, 333.0).unwrap();
    m.fx_set_enabled(0, 3, true).unwrap();
    m.fx_set_slot(0, 5, 0).unwrap(); // set but DISABLED still syncs

    let v: serde_json::Value = serde_json::from_str(&m.console_params_json_inner()).expect("json");
    let chains = v["fxChains"].as_array().expect("fxChains array");
    assert_eq!(
        chains.len(),
        1,
        "only pairs with a non-empty chain serialize"
    );
    let c0 = &chains[0];
    assert_eq!(c0["pair"].as_i64().unwrap(), 0);
    let slots = c0["slots"].as_array().expect("slots");
    assert_eq!(slots.len(), CHAIN_SLOTS);
    // Enabled delay with 7 live params (no padded row on the wire).
    assert_eq!(slots[3]["fx"].as_i64().unwrap(), 8);
    assert!(slots[3]["enabled"].as_bool().unwrap());
    let params = slots[3]["params"].as_array().expect("params");
    assert_eq!(params.len(), 7);
    assert_eq!(params[4].as_f64().unwrap() as f32, 333.0);
    // Set-but-disabled wah still round-trips (id, flag, 2 live params).
    assert_eq!(slots[5]["fx"].as_i64().unwrap(), 0);
    assert!(!slots[5]["enabled"].as_bool().unwrap());
    assert_eq!(slots[5]["params"].as_array().unwrap().len(), 2);
    // Empty slot: -1 with an empty param array.
    assert_eq!(slots[0]["fx"].as_i64().unwrap(), -1);
    assert!(slots[0]["params"].as_array().unwrap().is_empty());
}

// ── End-to-end insert point (process_block level) ──────────────────────

/// The insert-point proof, end to end through the real console: an
/// impulse into strip 0 with the pair's delay enabled comes back as
/// an echo at +250 ms ON THE MASTER OUTPUT — reachable only through
/// the engine call lists, i.e. the FX pass feeds the engine input
/// (post-staging, pre-gain: the echo is scaled by the strip's pan
/// law like any engine input) — while the DRY sample passes the whole
/// console bit-transparently (EQ bypassed, unity gains, limiter off).
/// The strip METER seeing the echo during an input-silent block pins
/// metering AFTER the FX pass, and strip 1's meter staying at the
/// floor pins that the pair pass runs rows independently.
#[test]
fn fx_delay_echo_reaches_master_end_to_end() {
    let mut m = mixer();
    m.set_limiter_enabled(false); // level-exact assertions
    m.set_channel_gain(0, 1.0).unwrap(); // creates strip 0 (pair 0 owner)
    m.set_channel_gain(1, 1.0).unwrap(); // odd half of pair 0 → full stereo
    m.set_eq_bypass(0, true).unwrap(); // staging must not filter the impulse
    m.set_eq_bypass(1, true).unwrap();

    // Delay on pair 0, slot 0: 250 ms → 12000 samples @ 48 kHz, unity.
    m.fx_set_slot(0, 0, 8).unwrap();
    m.fx_set_param(0, 0, 3, 0.0).unwrap(); // L_GAIN 0 dB
    m.fx_set_param(0, 0, 4, 250.0).unwrap(); // L_TIME 250 ms
    m.fx_set_enabled(0, 0, true).unwrap();

    // Silent lead-in (creates the FIFOs, settles the delay's level
    // one-pole and time glide — same warmup as the rack unit test).
    // Feeds are PACED 1:1 with processing (a block fed per block
    // consumed) so the elastic FIFOs never starve: starving them trips
    // the stream-health gate, whose fade-out would scale the impulse.
    let silence = vec![0.0f32; BS];
    for _ in 0..64 {
        m.feed_interleaved(0, &silence, 1).unwrap();
        m.feed_interleaved(1, &silence, 1).unwrap();
        m.process_block(BLOCK).unwrap();
    }
    assert!(
        m.channel_peak_db(0) < -199.0,
        "lead-in must meter digital silence"
    );

    // Impulse: 1.0 at the first sample of the next block, L only;
    // silence continues on both channels afterwards.
    let mut impulse = vec![0.0f32; BS];
    impulse[0] = 1.0;

    let echo_at = (250.0 * SR as f32 / 1000.0) as usize; // 12000 samples
    let echo_block = echo_at / BS; // block index after the impulse block
    let blocks = echo_block + 4;
    let mut left: Vec<f32> = Vec::with_capacity(blocks * BS);
    let mut right: Vec<f32> = Vec::with_capacity(blocks * BS);
    for b in 0..blocks {
        let feed_l: &[f32] = if b == 0 { &impulse } else { &silence };
        m.feed_interleaved(0, feed_l, 1).unwrap();
        m.feed_interleaved(1, &silence, 1).unwrap();
        let out = m.process_block(BLOCK).expect("process");
        for i in 0..BS {
            left.push(out[i * 2]);
            right.push(out[i * 2 + 1]);
        }
        if b == echo_block {
            // Metering runs AFTER the FX pass: this block's INPUT was
            // silent (starved FIFO) yet the meter saw the echo tail.
            let peak_db = m.channel_peak_db(0);
            assert!(
                peak_db.abs() < 1.0,
                "echo block must meter ~0 dB post-FX, got {peak_db}"
            );
        }
    }

    // DRY: bit-transparent through the whole console — staging copies
    // the drained row, the delay's settled dry path is unity, and the
    // engine path contributes exactly the Linear center-pan gain 0.5
    // (on BOTH master channels: mono strip, center pan).
    assert!(
        (left[0] - 0.5).abs() < 1e-6,
        "dry impulse must reach master at pan gain, got {}",
        left[0]
    );
    // Everything before the echo (other than the dry sample) is EXACT
    // silence — no staging/FX/engine residue anywhere.
    for (i, &s) in left.iter().enumerate() {
        if i != 0 && i < echo_at - 4 {
            assert!(s == 0.0, "unexpected pre-echo energy at {i}: {s}");
        }
    }
    // Center-panned mono strip: master R mirrors master L exactly
    // (Linear law, both sides 0.5 — pinned by known_answer).
    assert_eq!(
        right, left,
        "center-panned mono strip must feed L and R identically"
    );
    // The pair's R ROW stayed silent (the delay doesn't cross-feed):
    // strip 1's meter observes the post-FX staged signal, so any L→R
    // leak in the pair pass would show here.
    assert!(
        m.channel_peak_db(1) < -199.0,
        "pair R row must stay silent post-FX"
    );

    // ECHO: at +12000 samples (±2), through the engine gain path —
    // unity delay output × 0.5 pan law. This is only possible if the
    // FX pass feeds the ENGINE input rows (post-staging, pre-gain).
    let lo = echo_at - 2;
    let hi = (echo_at + 2).min(left.len() - 1);
    let (pos, peak) = left[lo..=hi]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
        .map(|(i, &v)| (lo + i, v))
        .expect("window is non-empty");
    assert!(
        (pos as i32 - echo_at as i32).abs() <= 2,
        "echo at {pos}, expected {echo_at}"
    );
    assert!(
        (0.49..=0.51).contains(&peak),
        "echo amplitude {peak} ≈ 0.5 (unity delay × Linear center pan)"
    );
}
