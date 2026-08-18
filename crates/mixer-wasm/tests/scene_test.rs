//! Scene capture/recall tests (native, no JS runtime).
//!
//! Scenes are binding-owned console snapshots (`ConsoleScene` in lib.rs —
//! see the design note there): save captures every strip's gain/pan/
//! mute/solo/input gain/phase/pan law/name/main assign, 6-band EQ,
//! dynamics (comp/gate/expander enables + params), each bus's 16 slot
//! assignments + gain/mute/feeds_main, and master gain. Recall reapplies
//! everything through the SAME setter paths the JS surface uses.
//!
//! These tests verify via `console_snapshot()` — the identical read
//! primitive `save_scene` itself uses — plus a behavioral solo-gating
//! check through the real audio path (`feed_interleaved` +
//! `process_block`, no js_sys needed).
//!
//! Run: cargo test -p mixer-wasm --test scene_test

use mixer_wasm::MixerWasm;
use oximedia_mixer::channel::PanLaw;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;

fn mixer() -> MixerWasm {
    MixerWasm::new(SR, BLOCK, 256).expect("ctor")
}

fn sine(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SR as f32).sin())
        .collect()
}

/// Feed `input` to channel `ch`, process one block, return the master
/// output (interleaved stereo).
fn run_block(m: &mut MixerWasm, ch: u32, input: &[f32]) -> Vec<f32> {
    m.feed_interleaved(ch, input, 1).expect("feed");
    m.process_block(BLOCK).expect("process").to_vec()
}

fn max_abs(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |a, &b| a.max(b.abs()))
}

/// Build the deliberately weird console state across strips 0/1/2,
/// bus 3, and master. All values stay inside the setters' clamp ranges
/// so the round-trip is bit-exact.
fn set_weird_state(m: &mut MixerWasm) {
    // Strip 0: comp on, phase inverted, -4.5 dB pan law, shelf + peak EQ.
    m.set_channel_gain(0, 0.83).unwrap();
    m.set_channel_pan(0, -0.37).unwrap();
    m.set_channel_input_gain(0, -4.5).unwrap();
    m.set_channel_phase(0, true).unwrap();
    m.set_channel_pan_law(0, 2).unwrap(); // Minus4Dot5dB
    m.set_channel_name(0, "zero".into()).unwrap();
    m.set_eq_band_gain(0, 1, 3.25).unwrap();
    m.set_eq_band_freq(0, 1, 95.5).unwrap();
    m.set_eq_band_q(0, 1, 0.8).unwrap();
    m.set_eq_band_gain(0, 3, -2.75).unwrap();
    m.set_eq_band_freq(0, 3, 1777.7).unwrap();
    m.set_eq_band_q(0, 3, 2.9).unwrap();
    m.enable_compressor(0).unwrap();
    m.set_comp_param(0, 0, -23.5).unwrap(); // threshold_db
    m.set_comp_param(0, 1, 5.5).unwrap(); // ratio
    m.set_comp_param(0, 2, 1.25).unwrap(); // attack_ms
    m.set_comp_param(0, 3, 234.0).unwrap(); // release_ms
    m.set_comp_param(0, 4, 1.5).unwrap(); // makeup_gain_db
    m.set_comp_param(0, 5, 7.25).unwrap(); // knee_db

    // Strip 1: muted, off main, EQ bypassed, gate on, -6 dB pan law.
    m.set_channel_gain(1, 0.41).unwrap();
    m.set_channel_pan(1, 0.62).unwrap();
    m.set_channel_mute(1, true).unwrap();
    m.set_channel_main_assign(1, false);
    m.set_channel_input_gain(1, 2.25).unwrap();
    m.set_channel_phase(1, false).unwrap();
    m.set_channel_pan_law(1, 3).unwrap(); // Minus6dB
    m.set_channel_name(1, "one".into()).unwrap();
    m.set_eq_bypass(1, true).unwrap();
    m.set_eq_band_gain(1, 2, 4.0).unwrap();
    m.set_eq_band_freq(1, 2, 610.0).unwrap();
    m.set_eq_band_q(1, 2, 1.4).unwrap();
    m.enable_gate(1).unwrap();
    m.set_gate_param(1, 0, -42.5).unwrap(); // threshold_db
    m.set_gate_param(1, 1, 8.5).unwrap(); // hysteresis_db
    m.set_gate_param(1, 2, 0.25).unwrap(); // attack_ms
    m.set_gate_param(1, 3, 77.0).unwrap(); // release_ms
    m.set_gate_param(1, 4, 12.5).unwrap(); // hold_ms

    // Strip 2: SOLOED, expander on, -3 dB pan law, high-shelf EQ.
    m.set_channel_gain(2, 1.27).unwrap();
    m.set_channel_pan(2, -0.88).unwrap();
    m.set_channel_solo(2, true).unwrap();
    m.set_channel_input_gain(2, -11.75).unwrap();
    m.set_channel_phase(2, true).unwrap();
    m.set_channel_pan_law(2, 1).unwrap(); // Minus3dB
    m.set_channel_name(2, "two".into()).unwrap();
    m.set_eq_band_gain(2, 5, 1.5).unwrap();
    m.set_eq_band_freq(2, 5, 12_500.0).unwrap();
    m.set_eq_band_q(2, 5, 4.2).unwrap();
    m.enable_expander(2).unwrap();
    m.set_expander_param(2, 0, -35.5).unwrap(); // threshold_db
    m.set_expander_param(2, 1, 3.5).unwrap(); // ratio
    m.set_expander_param(2, 2, 2.5).unwrap(); // attack_ms
    m.set_expander_param(2, 3, 120.0).unwrap(); // release_ms

    // Bus 3: slots 0 and 5 tap channels 2 and 0; muted, off master.
    m.set_bus_source(3, 0, 2).unwrap();
    m.set_bus_source(3, 5, 0).unwrap();
    m.set_bus_gain(3, 0.61);
    m.set_bus_mute(3, true);
    m.set_bus_feeds_main(3, false);

    m.set_master_gain(0.77);
}

/// Overwrite EVERY parameter touched by `set_weird_state` with different
/// values (dynamics migrate between strips, bus routing moves, solo
/// flips). Only touches strips that already exist — strip creation is
/// stream state, not console state (see the post-save-strip test below).
fn set_mutated_state(m: &mut MixerWasm) {
    m.set_channel_gain(0, 0.12).unwrap();
    m.set_channel_pan(0, 0.9).unwrap();
    m.set_channel_input_gain(0, 6.0).unwrap();
    m.set_channel_phase(0, false).unwrap();
    m.set_channel_pan_law(0, 0).unwrap(); // Linear
    m.set_channel_name(0, "zero-mut".into()).unwrap();
    m.set_eq_bypass(0, true).unwrap();
    m.set_eq_band_gain(0, 1, -1.5).unwrap();
    m.set_eq_band_freq(0, 1, 310.0).unwrap();
    m.set_eq_band_q(0, 1, 3.3).unwrap();
    m.set_eq_band_gain(0, 3, 2.5).unwrap();
    m.set_eq_band_freq(0, 3, 900.0).unwrap();
    m.set_eq_band_q(0, 3, 0.55).unwrap();
    m.disable_compressor(0);
    // NOTE: the mutated-state gate goes on strip 2 (muted), not strip 0 —
    // strip 0 carries the audible/silent behavioral checks below, and the
    // engine's documented zero-poisoning bug (dynamics_honesty.rs: a sine
    // starting at sin(0)=0 pins the gate envelope to -inf, never opening)
    // would make it dead regardless of recall.
    m.set_channel_solo(0, true).unwrap(); // solo moved 2 → 0

    m.set_channel_gain(1, 1.9).unwrap();
    m.set_channel_pan(1, -0.5).unwrap();
    m.set_channel_mute(1, false).unwrap();
    m.set_channel_main_assign(1, true);
    m.set_channel_input_gain(1, -2.0).unwrap();
    m.set_channel_phase(1, true).unwrap();
    m.set_channel_pan_law(1, 1).unwrap();
    m.set_channel_name(1, "one-mut".into()).unwrap();
    m.set_eq_bypass(1, false).unwrap();
    m.set_eq_band_gain(1, 2, -3.0).unwrap();
    m.set_eq_band_freq(1, 2, 800.0).unwrap();
    m.set_eq_band_q(1, 2, 5.5).unwrap();
    m.disable_gate(1);

    m.set_channel_gain(2, 0.5).unwrap();
    m.set_channel_pan(2, 0.25).unwrap();
    m.set_channel_mute(2, true).unwrap();
    m.set_channel_solo(2, false).unwrap();
    m.set_channel_input_gain(2, 3.0).unwrap();
    m.set_channel_phase(2, false).unwrap();
    m.set_channel_pan_law(2, 3).unwrap();
    m.set_channel_name(2, "two-mut".into()).unwrap();
    m.set_eq_bypass(2, true).unwrap();
    m.set_eq_band_gain(2, 5, -4.0).unwrap();
    m.set_eq_band_freq(2, 5, 15_000.0).unwrap();
    m.set_eq_band_q(2, 5, 1.1).unwrap();
    m.disable_expander(2);
    m.enable_compressor(2).unwrap(); // comp migrated 0 → 2
    m.set_comp_param(2, 0, -6.0).unwrap();
    m.enable_gate(2).unwrap(); // gate migrated 1 → 2
    m.set_gate_param(2, 0, -20.0).unwrap();

    // Bus 3 routing moves to channel 1 on slot 0, slot 5 cleared.
    m.set_bus_source(3, 0, 1).unwrap();
    m.clear_bus_source(3, 5);
    m.set_bus_gain(3, 1.3);
    m.set_bus_mute(3, false);
    m.set_bus_feeds_main(3, true);

    m.set_master_gain(1.42);
}

/// Round-trip: weird state → save → mutate everything → recall → every
/// captured parameter reads back equal. Also pins that saving itself
/// doesn't disturb state, and that recall restores the DERIVED solo
/// gating through the real audio path.
#[test]
fn scene_round_trip_restores_every_parameter() {
    let mut m = mixer();
    // Level-exact behavioral checks below: the master limiter's 4×
    // oversampler rings down for a block or two after any audible block,
    // which would smear the exact-silence assertion (same reason every
    // level-exact test in this crate disables it).
    m.set_limiter_enabled(false);
    set_weird_state(&mut m);

    // Saving must not disturb audio/control state.
    let snap_saved = m.console_snapshot();
    let id = m.save_scene();
    assert_eq!(
        m.console_snapshot(),
        snap_saved,
        "save_scene must not disturb state"
    );
    assert_eq!(id, 1, "scene ids start at 1");
    assert_eq!(m.scene_count(), 1);

    // Spot-check the capture actually read what the setters wrote
    // (engine-owned gain stage + binding-owned staging params).
    let s0 = &snap_saved.strips[0];
    assert_eq!(s0.gain, 0.83);
    assert_eq!(s0.pan, -0.37);
    assert_eq!(s0.input_gain_db, -4.5);
    assert!(s0.phase_inverted);
    assert_eq!(s0.pan_law, PanLaw::Minus4Dot5dB);
    assert_eq!(s0.name, "zero");
    assert!(!s0.eq_bypass);
    assert_eq!(s0.eq_bands[1].gain_db, 3.25);
    assert_eq!(s0.eq_bands[1].freq_hz, 95.5);
    assert_eq!(s0.eq_bands[1].q, 0.8);
    assert_eq!(s0.eq_bands[3].gain_db, -2.75);
    let c = s0.comp.as_ref().expect("strip 0 comp captured");
    assert_eq!(c.threshold_db, -23.5);
    assert_eq!(c.ratio, 5.5);
    assert_eq!(c.attack_ms, 1.25);
    assert_eq!(c.release_ms, 234.0);
    assert_eq!(c.makeup_gain_db, 1.5);
    assert_eq!(c.knee_db, 7.25);

    let s1 = &snap_saved.strips[1];
    assert!(s1.mute);
    assert!(!s1.main_assign);
    assert!(s1.eq_bypass);
    assert_eq!(s1.pan_law, PanLaw::Minus6dB);
    let g = s1.gate.as_ref().expect("strip 1 gate captured");
    assert_eq!(g.hysteresis_db, 8.5);
    assert_eq!(g.hold_ms, 12.5);

    let s2 = &snap_saved.strips[2];
    assert!(s2.solo);
    assert!(!s2.mute);
    let e = s2.expander.as_ref().expect("strip 2 expander captured");
    assert_eq!(e.threshold_db, -35.5);
    assert_eq!(e.ratio, 3.5);

    let b3 = &snap_saved.buses[3];
    assert_eq!(b3.sources[0], Some(2));
    assert_eq!(b3.sources[5], Some(0));
    assert_eq!(b3.sources[1], None);
    assert_eq!(b3.gain, 0.61);
    assert!(b3.muted);
    assert!(!b3.feeds_main);
    assert_eq!(snap_saved.master_gain, 0.77);

    // Mutate everything to different values, run audio through the
    // mutated state, then recall.
    set_mutated_state(&mut m);
    assert_ne!(
        m.console_snapshot(),
        snap_saved,
        "mutation must change state"
    );

    // Behavioral: mutated state has strip 0 soloed → ch 0 is audible.
    let tone = sine(440.0, 0.5, BLOCK as usize);
    let out = run_block(&mut m, 0, &tone);
    assert!(
        max_abs(&out) > 0.01,
        "soloed strip 0 must be audible pre-recall, got {:e}",
        max_abs(&out)
    );

    m.recall_scene(id).expect("recall");

    // EVERY captured parameter reads back equal (whole-struct equality
    // over strips, buses and master).
    let snap_after = m.console_snapshot();
    assert_eq!(
        snap_after, snap_saved,
        "recall must restore every parameter"
    );

    // Behavioral: recalled state has strip 2 soloed → ch 0 is now
    // solo-gated out of every mix path (main + buses): exact silence.
    let out = run_block(&mut m, 0, &tone);
    assert!(
        max_abs(&out) < 1e-9,
        "solo-gated strip 0 must be silent after recall, got {:e}",
        max_abs(&out)
    );

    // Recall is repeatable (idempotent on an already-recalled console).
    m.recall_scene(id).expect("re-recall");
    assert_eq!(m.console_snapshot(), snap_saved);
}

/// Independence: scenes A and B stored from different states recall
/// correctly in A → B → A order with no aliasing or mutation of the
/// stored snapshots.
#[test]
fn scene_recall_is_independent_per_scene() {
    let mut m = mixer();

    // State A: strip 0 soloed + quiet, bus 0 tapping ch 0, master low.
    // All three strips exist up front so both scenes cover them (strips
    // created after a save are deliberately left alone by recall —
    // pinned separately below).
    m.set_channel_gain(0, 0.11).unwrap();
    m.set_channel_gain(2, 1.0).unwrap();
    m.set_channel_solo(0, true).unwrap();
    m.set_channel_name(0, "A0".into()).unwrap();
    m.set_channel_mute(1, true).unwrap();
    m.set_bus_source(0, 0, 0).unwrap();
    m.set_bus_gain(0, 0.2);
    m.set_bus_mute(0, true);
    m.set_bus_feeds_main(0, false);
    m.set_master_gain(0.33);
    let snap_a = m.console_snapshot();
    let id_a = m.save_scene();

    // State B: different everything (comp moves to strip 2, routing
    // moves to slot 1, solo/mute flipped, master high).
    m.set_channel_gain(0, 0.94).unwrap();
    m.set_channel_solo(0, false).unwrap();
    m.set_channel_name(0, "B0".into()).unwrap();
    m.set_channel_mute(1, false).unwrap();
    m.set_channel_gain(2, 0.58).unwrap();
    m.enable_compressor(2).unwrap();
    m.set_comp_param(2, 0, -31.0).unwrap();
    m.set_bus_source(0, 0, 1).unwrap(); // reassign the SAME slot (no new strip)
    m.set_bus_gain(0, 1.1);
    m.set_bus_mute(0, false);
    m.set_bus_feeds_main(0, true);
    m.set_master_gain(1.21);
    let snap_b = m.console_snapshot();
    let id_b = m.save_scene();

    assert_eq!(m.scene_count(), 2);
    assert_ne!(id_a, id_b);

    // A → B → A: the right values every time. If recall mutated a
    // stored scene (aliasing), the second A would not match.
    m.recall_scene(id_a).expect("recall A");
    assert_eq!(
        m.console_snapshot(),
        snap_a,
        "recall A must restore state A"
    );
    m.recall_scene(id_b).expect("recall B");
    assert_eq!(
        m.console_snapshot(),
        snap_b,
        "recall B must restore state B"
    );
    m.recall_scene(id_a).expect("recall A again");
    assert_eq!(
        m.console_snapshot(),
        snap_a,
        "recall A twice must still match"
    );
}

/// delete_scene removes the snapshot; recalling a deleted (or never
/// issued) id errors, while surviving scenes stay recallable.
#[test]
fn delete_scene_removes_and_dead_ids_error() {
    let mut m = mixer();
    m.set_channel_gain(0, 0.7).unwrap();
    let id1 = m.save_scene();
    m.set_channel_gain(0, 1.1).unwrap();
    let id2 = m.save_scene();
    assert_eq!(m.scene_count(), 2);

    m.delete_scene(id1);
    assert_eq!(m.scene_count(), 1);
    m.delete_scene(id1); // deleting again is a no-op
    assert_eq!(m.scene_count(), 1);

    assert!(m.recall_scene(id1).is_err(), "deleted id must error");
    assert!(m.recall_scene(9999).is_err(), "unknown id must error");
    assert!(m.recall_scene(0).is_err(), "id 0 is never issued");
    m.recall_scene(id2).expect("surviving scene still recalls");
    assert_eq!(m.console_snapshot().strips[0].gain, 1.1);
}

/// Strips created AFTER a save are stream state, not console state:
/// recall leaves their parameters alone (it must not apply defaults to
/// strips the scene never saw) — but a live solo on such a strip IS
/// cleared, since solo leaks into every other strip's derived mute.
#[test]
fn recall_leaves_post_save_strips_alone_except_foreign_solos() {
    let mut m = mixer();
    m.set_channel_gain(0, 0.5).unwrap();
    let id = m.save_scene();

    // Create strip 9 after the save, with a solo + its own params.
    m.set_channel_gain(9, 1.9).unwrap();
    m.set_channel_name(9, "late".into()).unwrap();
    m.set_channel_solo(9, true).unwrap();

    m.recall_scene(id).expect("recall");
    let s9 = &m.console_snapshot().strips[9];
    assert!(s9.exists, "post-save strip must not be destroyed");
    assert_eq!(s9.gain, 1.9, "post-save strip params must be left alone");
    assert_eq!(s9.name, "late");
    assert!(
        !s9.solo,
        "post-save solo must be cleared (it would leak into every strip's derived mute)"
    );
}

// ── Timed cross-fade recall (recall_scene_fade) ──────────────────────────
//
// Fade timing model: block k after recall advances the fade clock by
// one block duration and THEN applies t = clamp(k·bd / fade_ms, 0, 1)
// (zero-order hold at each block's end position), so a fade of N block
// durations spans exactly N blocks. Per-block gain output below feeds
// DC through an EQ-bypassed strip: amplitude is then exactly
// input × fader × 0.5 (Linear pan law at center), no other stage moves.

/// Duration of one processing block in ms (128 frames @ 48 kHz).
const BLOCK_MS: f64 = BLOCK as f64 * 1000.0 / SR as f64;
/// DC test tone amplitude.
const DC: f32 = 0.5;
/// Center-pan Linear-law per-side gain (known_answer pins 0.5).
const PAN_GAIN: f32 = 0.5;

fn db_to_lin(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

/// Timed recall ramps a strip fader in the dB domain: from-gain 1.0 →
/// to-gain 0.25 (0 dB → −12.04 dB) over exactly 4 blocks; later blocks
/// settle at the target. Honesty rule: the ramp is asserted on the
/// AUDIBLE master output, not on internal state.
#[test]
fn fade_recall_ramps_gain() {
    let mut m = mixer();
    m.set_limiter_enabled(false); // level-exact assertions
    m.set_eq_bypass(0, true).unwrap(); // DC must not hit the HPF
    m.set_channel_gain(0, 1.0).unwrap();

    // Target scene: gain 0.25. Save it, then return to 1.0 (the
    // from-state at fade time).
    m.set_channel_gain(0, 0.25).unwrap();
    let id = m.save_scene();
    m.set_channel_gain(0, 1.0).unwrap();

    let fade_blocks = 4.0f64;
    m.recall_scene_fade(id, fade_blocks * BLOCK_MS)
        .expect("fade recall");

    let from_db = 0.0f32;
    let to_db = 20.0 * 0.25f32.log10(); // −12.04 dB
    let dc = vec![DC; BLOCK as usize];
    for k in 1..=8i32 {
        let out = run_block(&mut m, 0, &dc);
        assert!(out.iter().all(|s| s.is_finite()), "no NaN/Inf at block {k}");
        let peak = max_abs(&out);
        let t = (k as f64 / fade_blocks).clamp(0.0, 1.0) as f32;

        // The applied position is the block's END (ZOH): sharp check
        // against the dB-lerp at t.
        let zoh_db = from_db + t * (to_db - from_db);
        let expected = db_to_lin(zoh_db) * DC * PAN_GAIN;
        assert!(
            (peak - expected).abs() < 1e-4,
            "block {k}: amplitude {peak:.6} vs dB-ramp {expected:.6} (t={t})"
        );

        // Spec tolerance vs the block-MIDPOINT ramp value: ZOH at the
        // block end sits at most half a fade step (|to_db|/8 ≈ 1.51 dB)
        // from the midpoint.
        let mid_t = ((k as f64 - 0.5) / fade_blocks).clamp(0.0, 1.0) as f32;
        let mid_db = from_db + mid_t * (to_db - from_db);
        let measured_db = 20.0 * (peak / (DC * PAN_GAIN)).log10();
        let half_step_db = (to_db - from_db).abs() / fade_blocks as f32 / 2.0;
        assert!(
            (measured_db - mid_db).abs() <= half_step_db + 0.05,
            "block {k}: {measured_db:.3} dB vs midpoint {mid_db:.3} dB"
        );

        if k as f64 >= fade_blocks {
            // Fade done (block 4 applies t = 1 exactly): target settles
            // bit-exactly and stays.
            let target = 0.25 * DC * PAN_GAIN;
            assert!(
                (peak - target).abs() < 1e-6,
                "block {k}: must settle exactly at target ({peak:.6} vs {target:.6})"
            );
        }
    }
    assert_eq!(
        m.console_snapshot().strips[0].gain,
        0.25,
        "fade end state must be the target verbatim"
    );
}

/// fade_ms == 0 delegates to instant recall: state and audible output
/// are identical to calling recall_scene on a twin console.
#[test]
fn fade_recall_zero_ms_instant() {
    let mut a = mixer();
    let mut b = mixer();
    for m in [&mut a, &mut b] {
        m.set_limiter_enabled(false);
        m.set_eq_bypass(0, true).unwrap();
        m.set_channel_gain(0, 0.8).unwrap();
        m.set_channel_pan(0, 0.25).unwrap();
        m.set_eq_band_gain(0, 2, 3.0).unwrap();
        m.set_bus_source(0, 0, 0).unwrap();
        m.set_master_gain(0.6);
    }
    let id_a = a.save_scene();
    let id_b = b.save_scene();
    for m in [&mut a, &mut b] {
        m.set_channel_gain(0, 0.2).unwrap();
        m.set_channel_pan(0, -0.5).unwrap();
        m.set_eq_band_gain(0, 2, -2.0).unwrap();
        m.set_bus_source(0, 0, 1).unwrap();
        m.set_master_gain(1.3);
    }

    a.recall_scene(id_a).expect("instant recall");
    b.recall_scene_fade(id_b, 0.0).expect("zero-ms fade recall");

    assert_eq!(
        b.console_snapshot(),
        a.console_snapshot(),
        "fade(0) must produce instant-recall state"
    );

    // Identical inputs → identical audible output, block for block (a
    // lingering fade in b would diverge immediately).
    let dc = vec![DC; BLOCK as usize];
    let mut last_peak = 0.0f32;
    for k in 0..4 {
        let oa = run_block(&mut a, 0, &dc);
        let ob = run_block(&mut b, 0, &dc);
        assert_eq!(oa, ob, "outputs diverged at block {k}");
        last_peak = max_abs(&oa);
    }
    assert!(last_peak > 0.01, "honesty: output must be audible");
    assert_eq!(a.console_snapshot().strips[0].gain, 0.8);
}

/// Cancel-on-set: a user setter call mid-fade drops the fade — later
/// blocks hold the user's value, nothing NaNs, and the fade never
/// resumes.
#[test]
fn fade_cancelled_by_user_set() {
    let mut m = mixer();
    m.set_limiter_enabled(false);
    m.set_eq_bypass(0, true).unwrap();
    m.set_channel_gain(0, 1.0).unwrap();
    m.set_channel_gain(0, 0.25).unwrap();
    let id = m.save_scene();
    m.set_channel_gain(0, 1.0).unwrap();
    m.recall_scene_fade(id, 4.0 * BLOCK_MS).expect("fade");

    let dc = vec![DC; BLOCK as usize];

    // Block 1 is mid-ramp (t = 0.25 → −3.01 dB): proves the fade really
    // drove the block (and survived its own setter pass).
    let out1 = run_block(&mut m, 0, &dc);
    let ramp1 = db_to_lin(0.25 * 20.0 * 0.25f32.log10()) * DC * PAN_GAIN;
    assert!(
        (max_abs(&out1) - ramp1).abs() < 1e-4,
        "block 1 must be mid-fade ({:.6} vs {:.6})",
        max_abs(&out1),
        ramp1
    );

    // User takes over mid-fade.
    m.set_channel_gain(0, 0.9).unwrap();

    let user = 0.9 * DC * PAN_GAIN;
    for k in 0..4 {
        let out = run_block(&mut m, 0, &dc);
        assert!(
            out.iter().all(|s| s.is_finite()),
            "no NaN/Inf after cancel at block {k}"
        );
        assert!(
            (max_abs(&out) - user).abs() < 1e-6,
            "block {k} after cancel: user gain must hold ({:.6} vs {user:.6})",
            max_abs(&out)
        );
    }
    assert_eq!(m.console_snapshot().strips[0].gain, 0.9);
}

/// Validation: unknown (or never-issued) scene ids error, as do
/// negative and NaN fade times; zero is valid (instant delegation).
#[test]
fn fade_unknown_scene_errors() {
    let mut m = mixer();
    m.set_channel_gain(0, 0.5).unwrap();
    assert!(
        m.recall_scene_fade(9999, 100.0).is_err(),
        "unknown id must error"
    );
    assert!(
        m.recall_scene_fade(0, 100.0).is_err(),
        "id 0 is never issued"
    );
    let id = m.save_scene();
    assert!(
        m.recall_scene_fade(id, -1.0).is_err(),
        "negative fade_ms must error"
    );
    assert!(
        m.recall_scene_fade(id, f64::NAN).is_err(),
        "NaN fade_ms must error"
    );
    // Errors left the console untouched (no fade started).
    assert_eq!(m.console_snapshot().strips[0].gain, 0.5);
    m.recall_scene_fade(id, 0.0).expect("zero-ms is valid");
    assert_eq!(m.scene_count(), 1);
}
