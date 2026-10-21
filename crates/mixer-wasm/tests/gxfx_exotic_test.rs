//! Tier-2 representative known-answers for the FINAL gxfx wave — the
//! exotic catalog tail (ids 76..82), driven through the REAL C++ DSP
//! via [`GxFxRack`], per AGENTS.md's honesty rule (every DSP path must
//! pass an end-to-end known-answer test before its factory slot is
//! trusted). The tier-1 per-effect smoke suite for all 83 ids lives in
//! tests/gxfx_smoke_test.rs; this file carries the wave's four
//! PHYSICS-claim representatives:
//!
//!   * cabinet (76) — the convolver family: a full-wet impulse produces
//!     a real decaying cab-IR tail (bounded above and below, dry
//!     suppressed), and model 18 "Off" is a bounded dry passthrough;
//!   * detune (80) — the smbPitchShift phase vocoder: +7 semitones on a
//!     440 Hz sine measured via zero-crossing ratio ≈ 2^(7/12) =
//!     1.4983 at WET 100 / DRY 0 (porting probe measured 1.4994);
//!   * tuner (81) — the NSDF pitch tracker: the FREQ control OUTPUT
//!     port (out-row 0) settles to the input sine's frequency within
//!     ±0.5 Hz after warmup and reads 0 on silence (probe measured
//!     439.999), with the audio path bit-identical (analysis only);
//!   * livelooper (82) — the 4-tape live looper: record a distinctive
//!     1 kHz sine for a bounded time, stop rec, engage rplay — the
//!     played-back audio matches the recorded material (RMS +
//!     frequency), then silence after stop.
//!
//! Vendor quirks tolerated here (documented in gxfx_dsp_looper.cpp's
//! header): rectime0..3 are uninitialized in the ctor and absent from
//! clear_state, so a fresh heap-reused instance may briefly show a
//! stale in-range bar meter — the looper claims below assert AUDIO,
//! never meters (meter-only, self-corrects after one block). The
//! rplay read head's 0 → -1 wrap reads one float before the tape (a
//! vendor indexing quirk, one sample per loop wrap, inaudible; the
//! finiteness claim still covers it).
//!
//! Every test also checks the universal claims: finite output (no
//! NaN/Inf anywhere) and silence-in → silence-out after settle.

use mixer_wasm::gxfx::GxFxRack;

const FS: u32 = 48_000;
const BS: usize = 128;

// Effect ids under test (gxfx_catalog.h row order).
const CABINET: i16 = 76;
const DETUNE: i16 = 80;
const TUNER: i16 = 81;
const LIVELOOPER: i16 = 82;

// ===========================================================================
// Harness helpers (same shapes as tests/gxfx_smoke_test.rs)
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

fn assert_finite(out: &[f32], what: &str) {
    for (i, &v) in out.iter().enumerate() {
        assert!(v.is_finite(), "{what}: non-finite sample at {i}: {v}");
    }
}

fn samples(secs: f64) -> usize {
    (secs * FS as f64) as usize
}

/// Both-direction zero-crossing frequency estimate (Hz) — the detune
/// pitch-shift probe. A Hann-windowed OLA vocoder output of a pure sine
/// is a pure sine at the shifted frequency, so crossings/2 per second
/// is the tone's frequency regardless of phase.
fn zcr_hz(x: &[f32]) -> f64 {
    let n = x
        .windows(2)
        .filter(|w| (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0))
        .count();
    n as f64 * FS as f64 / x.len() as f64 / 2.0
}

// ===========================================================================
// cabinet (id 76) — the convolver family representative
// ===========================================================================

// ── The cab IR is a STATIC table (cab_data.cc, 68..1000 taps @48k,
// resampled to engine rate, ≤ ~21 ms) convolved full-wet — the effect
// has no dry/wet knob, the convolver IS the effect. PHYSICAL claims on
// an impulse: (a) bounded above, (b) the dry impulse sample itself is
// suppressed (an IR is not a delta — its first taps are ≈0, the energy
// arrives spread over the IR), (c) a real DECAYING tail exists between
// bounded-above (energy present) and bounded-below (gone by the IR's
// end), (d) model 18 "Off" disables the convolver entirely → the
// impulse passes as a bounded, clean passthrough.
#[test]
fn cabinet_impulse_tail_decays_and_model_off_passthrough() {
    // (a-d) model 0 (default cab): impulse → IR tail.
    let mut r = rack();
    r.set_slot(0, 0, CABINET).unwrap();
    r.set_enabled(0, 0, true).unwrap();

    let t0 = samples(0.25);
    let mut sig = zeros(0.25);
    sig.push(1.0);
    sig.extend(zeros(0.06)); // IR ≤ ~21 ms; margin
    let (ol, or) = run_pair(&mut r, 0, &sig, &sig);
    assert_finite(&ol, "cabinet L");
    assert_finite(&or, "cabinet R");

    // (a) bounded above.
    let (ml, mr) = (max_abs(&ol), max_abs(&or));
    assert!(
        ml < 16.0 && mr < 16.0,
        "PHYSICAL: cabinet impulse response bounded (max L {ml:.3} R {mr:.3})"
    );
    // (b) dry suppressed: the impulse sample must not pass at unity.
    println!(
        "cabinet: dry sample out L {:+.4} R {:+.4} (model 0)",
        ol[t0], or[t0]
    );
    assert!(
        ol[t0].abs() < 0.1 && or[t0].abs() < 0.1,
        "PHYSICAL: dry path suppressed — the cab IR spreads the impulse \
         (sample passes at {:.3}/{:.3}, not unity)",
        ol[t0],
        or[t0]
    );
    // (c) tail RMS buckets: present early, decaying, gone by the IR's
    // end (cab IRs are ≤ 1000 taps @48k ≈ 20.8 ms; measured: exactly
    // zero past 22 ms — partitioned overlap-save adds no latency).
    let bucket = |x: &[f32], from_ms: f64, to_ms: f64| {
        let a = t0 + samples(from_ms / 1000.0);
        let b = t0 + samples(to_ms / 1000.0);
        rms(&x[a..b.min(x.len())])
    };
    let e_early = (bucket(&ol, 0.2, 4.0) + bucket(&or, 0.2, 4.0)) / 2.0;
    let e_mid = (bucket(&ol, 6.0, 10.0) + bucket(&or, 6.0, 10.0)) / 2.0;
    let e_end = (bucket(&ol, 24.0, 28.0) + bucket(&or, 24.0, 28.0)) / 2.0;
    let energy: f64 = ol[t0..]
        .iter()
        .chain(or[t0..].iter())
        .map(|&v| (v as f64) * (v as f64))
        .sum();
    println!(
        "cabinet: tail RMS early {e_early:.5} mid {e_mid:.6} end {e_end:.3e}, \
         total energy {energy:.4}"
    );
    assert!(
        e_early > 1e-4 && energy > 1e-4,
        "PHYSICAL: a real cab IR spreads the impulse into a non-zero tail \
         (early RMS {e_early:.2e}, energy {energy:.2e}) — a fake would be \
         silent"
    );
    assert!(
        e_mid < e_early / 2.0,
        "PHYSICAL: the IR tail must decay (mid {e_mid:.5} < early/2 {:.5})",
        e_early / 2.0
    );
    assert!(
        e_end < e_early / 1000.0,
        "PHYSICAL: the IR must end inside its bounded length (end {e_end:.3e} \
         < early/1000)"
    );

    // Silence claim afterwards.
    let z = zeros(1.5);
    let (zl, zr) = run_pair(&mut r, 0, &z, &z);
    let n = BS.min(zl.len());
    assert!(
        max_abs(&zl[zl.len() - n..]) < 1e-4 && max_abs(&zr[zr.len() - n..]) < 1e-4,
        "cabinet: silence-in must settle"
    );

    // (d) model 18 "Off" (c_model ordinal 3): the convolver is disabled
    // — the impulse must pass as a bounded, essentially clean
    // passthrough with the tail GONE.
    let mut r = rack();
    r.set_slot(0, 0, CABINET).unwrap();
    r.set_param(0, 0, 3, 18.0).unwrap(); // c_model = 18 "Off"
    r.set_enabled(0, 0, true).unwrap();
    let (ol, or) = run_pair(&mut r, 0, &sig, &sig);
    assert_finite(&ol, "cabinet Off L");
    assert_finite(&or, "cabinet Off R");
    println!(
        "cabinet: Off passthrough sample L {:+.4} R {:+.4}, post-impulse \
         max {:.3e}",
        ol[t0],
        or[t0],
        max_abs(&ol[t0 + 1..]).max(max_abs(&or[t0 + 1..]))
    );
    assert!(
        (0.9..=1.1).contains(&ol[t0]) && (0.9..=1.1).contains(&or[t0]),
        "PHYSICAL: model Off passes the impulse at ≈unity (L {:+.3} R {:+.3})",
        ol[t0],
        or[t0]
    );
    assert!(
        max_abs(&ol[t0 + 1..]) < 1e-4 && max_abs(&or[t0 + 1..]) < 1e-4,
        "PHYSICAL: model Off carries no IR tail (post-impulse max L {:.3e} \
         R {:.3e})",
        max_abs(&ol[t0 + 1..]),
        max_abs(&or[t0 + 1..])
    );
}

// ===========================================================================
// detune (id 80) — the phase-vocoder representative
// ===========================================================================

// ── smbPitchShift: +7 semitones on a 440 Hz sine must land at
// 440 × 2^(7/12) = 659.26 Hz (ratio 1.4983). Measured via both-direction
// zero crossings on a steady 1 s window at WET 100 / DRY 0 (pure wet —
// the dry copy would pin the ZCR halfway). The porting probe measured
// 1.4994; the honest vocoder tolerance here is ±2 %.
#[test]
fn detune_seven_semitones_zcr_ratio() {
    let mut r = rack();
    r.set_slot(0, 0, DETUNE).unwrap();
    r.set_param(0, 0, 0, 7.0).unwrap(); // DETUNE +7 semitones
    r.set_param(0, 0, 4, 100.0).unwrap(); // WET 100 %
    r.set_param(0, 0, 5, 0.0).unwrap(); // DRY 0 %
    r.set_enabled(0, 0, true).unwrap();

    // 0.5 s settle (plan rebuild after block 1 + OLA fill), then a 1 s
    // steady measurement window.
    let sig = sine(440.0, 0.4, 1.5);
    let out = run_mono(&mut r, 0, &sig);
    assert_finite(&out, "detune");
    let win = &out[out.len() - 48_000..];
    let f_out = zcr_hz(win);
    let f_in = zcr_hz(&sig[sig.len() - 48_000..]);
    let ratio = f_out / f_in;
    let want = 2f64.powf(7.0 / 12.0);
    println!(
        "detune: +7 st on {f_in:.1} Hz → {f_out:.1} Hz (ratio {ratio:.4}, \
         want {want:.4}, deviation {:+.2} %)",
        (ratio / want - 1.0) * 100.0
    );
    assert!(
        (f_in - 440.0).abs() < 1.0,
        "probe sanity: input ZCR must be the 440 Hz sine ({f_in:.1} Hz)"
    );
    assert!(
        (ratio - want).abs() / want < 0.02,
        "PHYSICAL: +7 semitones must shift the tone by ×{want:.4} within 2 % \
         (ZCR ratio {ratio:.4}, deviation {:+.2} %)",
        (ratio / want - 1.0) * 100.0
    );
    // The shifted tone must be real energy, not a residual: RMS within
    // an octave-ish of the input's (the vocoder's band gains are ≈1).
    let level = rms(win) / rms(&sig[sig.len() - 48_000..]);
    println!("detune: output/input RMS ratio {level:.3}");
    assert!(
        (0.25..=2.0).contains(&level),
        "PHYSICAL: the shifted tone must carry real energy (RMS ratio \
         {level:.3}, not a leaked residual)"
    );

    // Silence claim: the vocoder has no feedback path — silence-in
    // settles inside the plain window.
    let z = zeros(1.5);
    let (zl, zr) = run_pair(&mut r, 0, &z, &z);
    let n = BS.min(zl.len());
    assert!(
        max_abs(&zl[zl.len() - n..]) < 1e-4 && max_abs(&zr[zr.len() - n..]) < 1e-4,
        "detune: silence-in must settle"
    );
}

// ===========================================================================
// tuner (id 81) — the pitch-tracker representative
// ===========================================================================

// ── The tuner's FREQ control OUTPUT port (PortIndex 0, out-row 0 via
// fx_out_param): fed a 440 Hz sine it must settle to the tone's
// frequency within ±0.5 Hz (the tracker analyzes one 2048-sample NSDF
// window per ~100 ms cadence; the porting probe measured 439.999), and
// on digital silence the level gate must drive it to exactly 0. The
// audio path is BIT-IDENTICAL (the tracker only reads the input — the
// insert-FX contract for a tuner). The tone rides BOTH channels: the
// mono slot is dual-mono and both instances write the shared out row
// (last writer per block) — a silent R half would gate its tracker to
// 0 and clobber the reading.
#[test]
fn tuner_freq_out_port_tracks_and_gates() {
    let mut r = rack();
    r.set_slot(0, 0, TUNER).unwrap();
    r.set_enabled(0, 0, true).unwrap();

    // Bit-transparency: processed audio == input, sample for sample.
    let sig = sine(440.0, 0.5, 1.0);
    let out = run_mono(&mut r, 0, &sig);
    assert_finite(&out, "tuner");
    assert!(
        out.iter().zip(sig.iter()).all(|(o, &i)| *o == i),
        "PHYSICAL: the tuner must be bit-transparent (analysis only, no \
         audio modification)"
    );

    // Warmup + settle: keep the tone running on BOTH channels; the
    // FREQ port refreshes every block, one NSDF pass per ~100 ms. Poll
    // after each block and require it to ARRIVE within ±0.5 Hz of 440
    // and stay.
    let mut l = vec![0.0f32; BS];
    let mut rr = vec![0.0f32; BS];
    let mut arrived_at = None;
    let total_blocks = samples(3.0) / BS;
    for b in 0..total_blocks {
        for i in 0..BS {
            let v = sig[(b * BS + i) % sig.len()];
            l[i] = v;
            rr[i] = v;
        }
        r.process_pair(0, &mut l, Some(&mut rr));
        let f = r.out_param(0, 0, 0).unwrap();
        if (f - 440.0).abs() <= 0.5 {
            if arrived_at.is_none() {
                arrived_at = Some(b);
            }
        } else if arrived_at.is_some() && b < total_blocks - 1 {
            // After arrival the reading must HOLD (a flapping tracker
            // would drop out between analysis passes).
            panic!(
                "tuner: FREQ dropped from ≈440 to {f:.3} Hz at block {b} \
                 after arriving at {:?}",
                arrived_at
            );
        }
    }
    let f = r.out_param(0, 0, 0).unwrap();
    println!("tuner: FREQ settled to {f:.3} Hz (arrived at block {arrived_at:?})");
    assert!(
        arrived_at.is_some(),
        "PHYSICAL: FREQ must settle to the input sine's frequency within \
         ±0.5 Hz inside 3 s (last reading {f:.3} Hz)"
    );

    // Silence: the level gate (THRESHOLD −50 dB default) must drive
    // FREQ to exactly 0 — 0 is the documented "silent" sentinel. Zero
    // BOTH channels (the tuner is in-place transparent; a stale tone
    // block left in one half would keep that tracker analyzing).
    for v in l.iter_mut() {
        *v = 0.0;
    }
    for v in rr.iter_mut() {
        *v = 0.0;
    }
    for _ in 0..samples(1.5) / BS {
        r.process_pair(0, &mut l, Some(&mut rr));
    }
    let f = r.out_param(0, 0, 0).unwrap();
    println!("tuner: FREQ on silence {f:.3} Hz");
    assert!(
        f == 0.0,
        "PHYSICAL: FREQ must read exactly 0 on digital silence (got {f:.3})"
    );
    // And silence stays silent on the audio path.
    let n = BS.min(l.len());
    assert!(
        max_abs(&l[l.len() - n..]) < 1e-9,
        "tuner: silence-in must be bit-silent"
    );
}

// ===========================================================================
// livelooper (id 82) — the looper representative
// ===========================================================================

// ── Record a distinctive signal (1 kHz sine at a fixed level) onto
// tape 1 for a bounded 0.5 s, stop rec, engage rplay (reverse play):
// the played-back audio must MATCH the recorded material — frequency
// via zero crossings (a reversed steady sine keeps its frequency) and
// RMS at the tape path's fixed scaling (mix 100 × level1 50 → ×0.5) —
// then stopping rplay must leave true silence. AUDIO assertions only:
// the bar meters (rectime0..3) are the documented uninitialized vendor
// quirk (gxfx_dsp_looper.cpp header) and are deliberately not asserted.
#[test]
fn livelooper_records_and_plays_back_the_material() {
    // Param ordinals (GXFX_LIVELOOPER_PARAMS order).
    const LEVEL1: usize = 13;
    const RPLAY1: usize = 22;
    const REC1: usize = 26;

    let mut r = rack();
    r.set_slot(0, 0, LIVELOOPER).unwrap();
    r.set_enabled(0, 0, true).unwrap(); // allocates + zeroes the tapes

    // Block-level driver: the looper is a stateful transport, so the
    // phases run in sequence over shared state (not run_pair's one-shot).
    let mut l = vec![0.0f32; BS];
    let mut rr = vec![0.0f32; BS];
    let mut push = |rack: &mut GxFxRack, sig: &[f32]| -> Vec<f32> {
        let mut out = Vec::with_capacity(sig.len());
        for c in sig.chunks(BS) {
            l[..c.len()].copy_from_slice(c);
            for v in l[c.len()..].iter_mut() {
                *v = 0.0;
            }
            rr.iter_mut().for_each(|v| *v = 0.0);
            rack.process_pair(0, &mut l, Some(&mut rr));
            out.extend_from_slice(&l[..c.len()]);
        }
        out
    };

    // Warmup silence: the input gain one-pole (0.999 → unity, ~0.15 s)
    // must be settled BEFORE recording so the tape captures the tone at
    // its full fixed level.
    let warm = push(&mut r, &zeros(0.3));
    assert_finite(&warm, "looper warmup");

    // RECORD 0.5 s of 1 kHz @ 0.4 (rec1 on, dry passthrough while
    // recording — fSlow5 carries (1 - record) = 0).
    r.set_param(0, 0, REC1, 1.0).unwrap();
    let tone = sine(1000.0, 0.4, 0.5);
    let rec_out = push(&mut r, &tone);
    assert_finite(&rec_out, "looper record");
    r.set_param(0, 0, REC1, 0.0).unwrap();
    let rms_in = rms(&tone[samples(0.05)..]);
    // Dry passthrough while recording (tape play muted by (1-rec)).
    let rms_rec = rms(&rec_out[samples(0.05)..samples(0.45)]);
    println!(
        "looper: dry passthrough while recording RMS {rms_rec:.4} \
         (input {rms_in:.4})"
    );
    assert!(
        (rms_rec / rms_in - 1.0).abs() < 0.1,
        "PHYSICAL: the dry path must pass ≈unity while recording (RMS \
         {rms_rec:.4} vs input {rms_in:.4})"
    );

    // PLAY (reverse): silence in — everything the looper emits now IS
    // the tape. Defaults mix=100, level1=50 → played amplitude ≈
    // 0.01 × 100 × 0.5 = 0.5 × recorded.
    r.set_param(0, 0, RPLAY1, 1.0).unwrap();
    let play = push(&mut r, &zeros(1.2));
    assert_finite(&play, "looper playback");
    // Skip the first ~20 ms (read-head entry + wrap sample), measure a
    // steady 1 s window: two full backward passes over the 0.5 s take.
    let win = &play[samples(0.02)..samples(1.02)];
    let rms_out = rms(win);
    let f_out = zcr_hz(win);
    println!(
        "looper: playback RMS {rms_out:.4} (expected ≈{:.4} = 0.5 × \
         recorded), ZCR {f_out:.1} Hz",
        0.5 * rms_in
    );
    assert!(
        rms_out > 0.05,
        "PHYSICAL: rplay must emit the recorded material (RMS {rms_out:.4}, \
         not silence — a fake looper plays nothing)"
    );
    assert!(
        (rms_out / (0.5 * rms_in) - 1.0).abs() < 0.25,
        "PHYSICAL: playback level must sit at the tape path's fixed ×0.5 \
         scaling (RMS {rms_out:.4} vs expected ≈{:.4})",
        0.5 * rms_in
    );
    assert!(
        (f_out - 1000.0).abs() < 50.0,
        "PHYSICAL: the played-back tone must be the recorded 1 kHz (ZCR \
         {f_out:.1} Hz — a reversed steady sine keeps its frequency)"
    );

    // STOP: rplay off → tape contribution gone; silence-in is
    // silence-out immediately (no tail — the looper is a transport).
    r.set_param(0, 0, RPLAY1, 0.0).unwrap();
    let stop = push(&mut r, &zeros(0.3));
    let n = BS.min(stop.len());
    assert!(
        max_abs(&stop[stop.len() - n..]) < 1e-5,
        "PHYSICAL: stopping rplay must leave true silence (max {:.3e})",
        max_abs(&stop[stop.len() - n..])
    );

    // Band-check the fixed scaling end to end: level1 → 100 doubles the
    // played material's contribution (0.01 × mix × gain1).
    r.set_param(0, 0, RPLAY1, 1.0).unwrap();
    r.set_param(0, 0, LEVEL1, 100.0).unwrap();
    let play2 = push(&mut r, &zeros(0.6));
    let win2 = &play2[samples(0.02)..samples(0.52)];
    let rms_out2 = rms(win2);
    println!("looper: level1 100 playback RMS {rms_out2:.4} (was {rms_out:.4})");
    assert!(
        rms_out2 > 1.5 * rms_out,
        "PHYSICAL: level1 must scale the tape path (RMS {rms_out2:.4} vs \
         {rms_out:.4} at level 50)"
    );
}
