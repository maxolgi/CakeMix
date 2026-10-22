//! Cabinet (id 76) meter regression — the live-E2E finding (2026-10-21,
//! tmp/qa-m3-final): with the cabinet engaged alone on a live pair, a
//! strip meter pegged full-scale and appeared frozen while the audio
//! path stayed clean.
//!
//! Root-cause characterization (native, both rates):
//! * The cabinet DSP itself is CLEAN under every stress tried — full-
//!   scale squares/DC/bipolar noise, multitone, sweeps, hot model
//!   swaps, CLevel/bass/treble at the ttl rails, all 19 models, at
//!   48 kHz (native IRs) and 44.1 kHz (every IR through the zita
//!   resampler): the partitioned convolver is linear-bounded, so
//!   finite in → finite out; post-cab peaks run HOT (measured up to
//!   +24 dB with CLevel=5 + full-scale input) but never non-finite.
//!   The "pegged+frozen" bar is the UI's dbToNorm capping every ≥0 dB
//!   value at full scale — the strip meter (post-FX, PRE-FADER) shows
//!   a genuinely hot signal the master limiter then hides.
//! * The METER path had a real non-finite hole, just one step removed:
//!   the strip meter's dB conversion floored NaN (NaN > 1e-10 is
//!   false) but let ±Inf through (Inf > 1e-10 is TRUE →
//!   20·log10(Inf) = +Inf dB), and the hand-rolled
//!   `channel_meters_json` formats non-finite f32 as a LITERAL
//!   (`"peak":inf` / `"NaN"`) — invalid JSON. In the worklet the whole
//!   meters message is built inside one try around
//!   `JSON.parse(channel_meters_json())`, so one poisoned row drops
//!   the ENTIRE message (master + every strip + bus) and every meter
//!   in the console freezes at its last value. MasterMeter had the
//!   same hole twice over: `master_lin_to_db` propagated non-finite,
//!   and Inf is a fixed point of the 0.95 peak decay (Inf·0.95 == Inf)
//!   and of the sliding RMS (Inf − x == Inf), so a single Inf block
//!   froze the master ballistics forever.
//!
//! Fixes pinned here (floor-not-propagate at the meter calculation):
//! strip meters floor non-finite peak/rms at −200 dB; the master meter
//! sanitizes non-finite samples at accumulation (clip still flags on
//! the raw value) and floors in the dB conversion.
//!
//! Run: cargo test -p mixer-wasm --test cabinet_meter_regression

use mixer_wasm::MixerWasm;
use serde_json::Value;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;
const BS: usize = BLOCK as usize;

fn mixer_at(sr: u32) -> MixerWasm {
    MixerWasm::new(sr, BLOCK, 256).expect("ctor")
}

fn mixer() -> MixerWasm {
    mixer_at(SR)
}

/// Deterministic xorshift64* broadband noise in [-1, 1).
struct Lcg(u64);
impl Lcg {
    fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        ((x.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f32 / 8_388_608.0) - 1.0
    }
}

fn multitone(amps: &[f32], freqs: &[f32], n: usize, phase: u64) -> Vec<f32> {
    let mut lc = Lcg(phase | 1);
    let ph: Vec<f32> = (0..freqs.len()).map(|_| lc.next_f32()).collect();
    (0..n)
        .map(|i| {
            let t = i as f32 / SR as f32;
            amps.iter()
                .zip(freqs.iter())
                .enumerate()
                .map(|(k, (&a, &f))| {
                    a * (2.0 * std::f32::consts::PI * f * t + ph[k] * std::f32::consts::PI).sin()
                })
                .sum()
        })
        .collect()
}

/// Assert `channel_meters_json` is parseable JSON whose every entry
/// carries a FINITE number (never null, never NaN/Inf literals) for
/// peak and rms, within sane meter bounds. Returns the parsed doc.
fn assert_meters_json_finite(m: &MixerWasm) -> Value {
    let json = m.channel_meters_json();
    let doc: Value = serde_json::from_str(&json)
        .unwrap_or_else(|e| panic!("channel_meters_json must be valid JSON, got {e}: {json}"));
    let arr = doc
        .as_array()
        .unwrap_or_else(|| panic!("meters doc must be an array: {json}"));
    for entry in arr {
        for field in ["peak", "rms", "gr"] {
            let v = entry
                .get(field)
                .unwrap_or_else(|| panic!("entry missing {field}: {entry}"));
            assert!(
                v.is_number(),
                "{field} must be a JSON number (never null): {entry}"
            );
            let f = v.as_f64().unwrap();
            assert!(
                f.is_finite() && f > -201.0 && f < 40.0,
                "{field} must be a finite meter value in dB, got {f}: {entry}"
            );
        }
    }
    doc
}

/// Run one block of program through the pair and assert the strip
/// meters + their JSON stay finite. Returns (peak0, peak1) in dB.
fn step_and_check(m: &mut MixerWasm, l: &[f32], r: &[f32]) -> (f32, f32) {
    m.feed_interleaved(0, l, 1).unwrap();
    m.feed_interleaved(1, r, 1).unwrap();
    m.process_block(BLOCK).unwrap();
    let pk0 = m.channel_peak_db(0);
    let pk1 = m.channel_peak_db(1);
    let rms0 = m.channel_rms_db(0);
    assert!(
        pk0.is_finite() && pk0 > -201.0 && pk0 < 40.0,
        "ch0 peak {pk0}"
    );
    assert!(
        pk1.is_finite() && pk1 > -201.0 && pk1 < 40.0,
        "ch1 peak {pk1}"
    );
    assert!(
        rms0.is_finite() && rms0 > -201.0 && rms0 < 40.0,
        "ch0 rms {rms0}"
    );
    assert_meters_json_finite(m);
    (pk0, pk1)
}

// ===========================================================================
// Tier 1 — cabinet on program-like input (the E2E triggering conditions)
// ===========================================================================

/// The live-E2E scenario, native: cabinet alone on a live pair, fed
/// program-like input at realistic AND punishing levels — broadband
/// noise, multitones pushed toward ±1, amplitude sweeps, DC, low-level
/// (denormal territory), full-scale bipolar noise — with the tone
/// params at the ttl rails (CLevel=5, bass/treble=±10) and the cab
/// model walked across all 19 entries (18 cabs + Off), at 48 kHz
/// (native-rate IRs) and 44.1 kHz (every IR resampled). Every block
/// must meter finite values and serialize a parseable, null-free,
/// finite meters JSON. (Post-cab peaks ARE allowed to run hot — up to
/// +24 dB was measured — only non-finite/invalid-JSON fails.)
/// A named program-material generator (see the scenarios list below).
type Program<'a> = (&'a str, Box<dyn Fn(usize) -> Vec<f32>>);

#[test]
fn cabinet_program_input_meters_stay_finite() {
    let scenarios: Vec<Program> = vec![
        (
            "noise_0.5",
            Box::new(|n| {
                let mut lc = Lcg(0xC0FFEE);
                (0..n).map(|_| 0.5 * lc.next_f32()).collect()
            }),
        ),
        (
            "noise_fs_bipolar",
            Box::new(|n| {
                let mut lc = Lcg(0xFACE);
                (0..n)
                    .map(|_| if lc.next_f32() < 0.0 { -1.0f32 } else { 1.0f32 })
                    .collect()
            }),
        ),
        (
            "multitone_hot",
            Box::new(|n| {
                multitone(
                    &[0.22; 7],
                    &[82.0, 147.0, 220.0, 443.0, 947.0, 3100.0, 7300.0],
                    n,
                    7,
                )
            }),
        ),
        ("dc_0.9", Box::new(|n| vec![0.9; n])),
        (
            "lowlevel_1e-7",
            Box::new(|n| {
                let mut lc = Lcg(0x5DCC);
                (0..n).map(|_| 1e-7 * lc.next_f32()).collect()
            }),
        ),
        (
            "amp_sweep",
            Box::new(|n| {
                (0..n)
                    .map(|i| {
                        let t = i as f64 / n as f64;
                        let f = 20.0 * (20000.0 / 20.0f64).powf(t);
                        let a = 1e-4 + 0.999 * t;
                        (a * (2.0 * std::f64::consts::PI * f * (i as f64 / SR as f64)).sin()) as f32
                    })
                    .collect()
            }),
        ),
    ];

    for (name, gen) in scenarios {
        for sr in [48_000u32, 44_100] {
            let mut m = mixer_at(sr);
            m.set_limiter_enabled(false);
            m.set_channel_gain(0, 1.0).unwrap();
            m.set_channel_gain(1, 1.0).unwrap();
            m.fx_set_slot(0, 0, 76).unwrap();
            // Tone params at the ttl rails — the worst IR bake.
            m.fx_set_param(0, 0, 0, 5.0).unwrap(); // CLevel max
            m.fx_set_param(0, 0, 1, 10.0).unwrap(); // CBass max
            m.fx_set_param(0, 0, 2, 10.0).unwrap(); // CTreble max
            m.fx_set_enabled(0, 0, true).unwrap();

            let blocks = 120;
            let sig = gen(blocks * BS);
            let mut lc = Lcg(0x7EA5 ^ sr as u64);
            for b in 0..blocks {
                // walk the cab model across all 19 entries mid-stream
                // (IR rebuild + hot-swap transients on live audio)
                if b % 8 == 0 {
                    m.fx_set_param(0, 0, 3, ((b / 8) % 19) as f32).unwrap();
                }
                let l = &sig[b * BS..(b + 1) * BS];
                let r: Vec<f32> = (0..BS).map(|_| 0.9 * lc.next_f32()).collect();
                step_and_check(&mut m, l, &r);
            }
            // quiet sanity tail at default params (convolution ring-out
            // of the previous hot block makes the first quiet block
            // meter above its steady level — only finiteness + a
            // clearly-audible-quiet bound is asserted)
            m.fx_set_param(0, 0, 0, 1.0).unwrap();
            m.fx_set_param(0, 0, 1, 0.0).unwrap();
            m.fx_set_param(0, 0, 2, 0.0).unwrap();
            let quiet = vec![0.01f32; BS];
            let (pk0, _) = step_and_check(&mut m, &quiet, &quiet);
            assert!(pk0 < 0.0, "quiet tail must meter below 0 dB, got {pk0}");
        }
        let _ = name;
    }
}

/// The honest post-cab HOTNESS the E2E actually saw (why the strip bar
/// pegs while audio stays clean): full-scale program through the cab
/// at max CLevel meters well ABOVE 0 dBFS post-FX pre-fader. This pins
/// that the peg is a real signal level (finite), not a non-finite —
/// the distinction this regression exists for.
#[test]
fn cabinet_runs_hot_but_finite() {
    let mut m = mixer();
    m.set_limiter_enabled(false);
    m.set_channel_gain(0, 1.0).unwrap();
    m.set_channel_gain(1, 1.0).unwrap();
    m.fx_set_slot(0, 0, 76).unwrap();
    m.fx_set_param(0, 0, 0, 5.0).unwrap();
    m.fx_set_enabled(0, 0, true).unwrap();

    let per = (SR / 220) as usize;
    let square: Vec<f32> = (0..200 * BS)
        .map(|i| {
            if (i / (per / 2)).is_multiple_of(2) {
                1.0f32
            } else {
                -1.0f32
            }
        })
        .collect();
    let mut max_pk = f32::NEG_INFINITY;
    for b in 0..200 {
        let l = &square[b * BS..(b + 1) * BS];
        let (pk0, _) = step_and_check(&mut m, l, l);
        if b > 20 {
            max_pk = max_pk.max(pk0);
        }
    }
    assert!(
        max_pk > 0.0,
        "full-scale square at CLevel=5 must meter above 0 dB post-cab (measured families reach +6..+24 dB), got {max_pk}"
    );
}

// ===========================================================================
// Tier 2 — the meter-path guard (non-finite must floor, never propagate)
// ===========================================================================

/// Non-finite PCM (a decoder-side anomaly is the only way one can
/// reach a staged row — the cabinet DSP provably emits finite output)
/// must floor the strip meters, not the console: with the pre-fix
/// behavior, ±Inf metered as +Inf dB and `channel_meters_json`
/// emitted the literal `inf`, making the whole document unparseable
/// and freezing EVERY meter in the UI. NaN already floored via the
/// `> 1e-10` comparison; Inf is the hole. Both must floor at −200,
/// the healthy row must keep metering, and the strip must recover on
/// clean audio. Pinned for the DRY strip (passthrough — the poison
/// reaches the row verbatim) and through the cabinet (whose FFT MACs
/// turn a lone Inf into an all-NaN block — floored too).
#[test]
fn nonfinite_pcm_floors_strip_meters_not_the_json() {
    // 440 Hz sine program at 0.3 (≈ −10.5 dB peak) — unlike DC it is
    // not shaped away by the strip's DC-blocking, so "healthy" has a
    // stable expected level.
    let prog: Vec<f32> = (0..BS)
        .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SR as f32).sin())
        .collect();
    for (label, poison) in [("inf", f32::INFINITY), ("nan", f32::NAN)] {
        for with_cabinet in [false, true] {
            let mut m = mixer();
            m.set_limiter_enabled(false);
            m.set_channel_gain(0, 1.0).unwrap();
            m.set_channel_gain(1, 1.0).unwrap();
            // EQ bypassed to isolate the METER semantics: a non-finite
            // sample latches NaN into an active EQ's biquad state and
            // kills the strip's audio permanently (separate finding,
            // filed in FINDINGS.md — this test is about the meters).
            m.set_eq_bypass(0, true).unwrap();
            m.set_eq_bypass(1, true).unwrap();
            if with_cabinet {
                m.fx_set_slot(0, 0, 76).unwrap();
                m.fx_set_enabled(0, 0, true).unwrap();
            }
            // clean lead-in on both strips (settles the FIFOs)
            for _ in 0..8 {
                step_and_check(&mut m, &prog, &prog);
            }
            // poisoned sample 0 on L only; R stays healthy
            let mut l = prog.clone();
            l[0] = poison;
            let doc = {
                m.feed_interleaved(0, &l, 1).unwrap();
                m.feed_interleaved(1, &prog, 1).unwrap();
                m.process_block(BLOCK).unwrap();
                assert_meters_json_finite(&m) // THE pin: valid, finite
            };
            let row0 = doc
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e.get("ch").and_then(|c| c.as_u64()) == Some(0))
                .expect("ch 0 row");
            let peak0 = row0.get("peak").unwrap().as_f64().unwrap();
            match (label, with_cabinet) {
                // Inf meters +Inf pre-fix (the JSON-breaking literal);
                // post-fix it floors. Through the cab the FFT MACs turn
                // the lone Inf into an all-NaN block (peak folds to 0).
                ("inf", _) => assert_eq!(
                    peak0, -200.0,
                    "[{label} cabinet={with_cabinet}] Inf-poisoned row must floor at -200 dB"
                ),
                // A lone NaN on the dry strip is IGNORED by the peak
                // fold (f32::max skips NaN) — the row's finite content
                // meters normally. Finite either way, which is the pin.
                ("nan", false) => assert!(
                    (-16.0..-5.0).contains(&peak0),
                    "[{label} cabinet={with_cabinet}] NaN-sample row must meter its finite content, got {peak0}"
                ),
                // Through the cab a NaN sample poisons the whole FFT
                // block → all-NaN row → floor.
                ("nan", true) => assert_eq!(
                    peak0, -200.0,
                    "[{label} cabinet={with_cabinet}] NaN-poisoned convolver block must floor"
                ),
                _ => unreachable!(),
            }
            // healthy R keeps metering the clean program
            let row1 = doc
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e.get("ch").and_then(|c| c.as_u64()) == Some(1))
                .expect("ch 1 row");
            let peak1 = row1.get("peak").unwrap().as_f64().unwrap();
            assert!(
                (-16.0..-5.0).contains(&peak1),
                "[{label} cabinet={with_cabinet}] healthy row must keep metering, got {peak1}"
            );
            // recovery: clean audio restores the strip meter — at once
            // through the dry strip, and within the convolver's FIR
            // memory through the cab (the default 4x12 IR is 1000 taps
            // → 8 partitions; the spectra ring fully turns over after
            // parts_+1 blocks — measured: block 9).
            let mut pk0 = -200.0;
            for _ in 0..12 {
                let (p, _) = step_and_check(&mut m, &prog, &prog);
                pk0 = p;
            }
            assert!(
                (-16.0..-5.0).contains(&pk0),
                "[{label} cabinet={with_cabinet}] strip meter must recover, got {pk0}"
            );
        }
    }
}

/// The master meter's non-finite hole: pre-fix, one Inf block set
/// peak=Inf (a fixed point of the 0.95 decay — never recovers) and
/// rms_sq=Inf (Inf − x == Inf — never drains), freezing the master
/// ballistics forever while strip meters recovered. Now the master
/// meters finite through the poison and converge back to the true
/// level of the following clean program.
#[test]
fn nonfinite_pcm_does_not_stick_master_ballistics() {
    let mut m = mixer();
    m.set_limiter_enabled(false);
    m.set_channel_gain(0, 1.0).unwrap();
    m.set_channel_gain(1, 1.0).unwrap();
    // EQ bypassed (see the strip test: an active EQ latches NaN state
    // and kills the strip — separate finding; this test pins meters).
    m.set_eq_bypass(0, true).unwrap();
    m.set_eq_bypass(1, true).unwrap();

    // Clean lead-in at a known level (0.5 → −6 dB peak). The whole
    // test stays inside one 375-block drift window so the elastic
    // FIFO's anchor machinery (built for drifting real streams) never
    // engages against this synthetic 1:1-paced feed.
    let prog: Vec<f32> = (0..BS)
        .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / SR as f32).sin())
        .collect();
    for _ in 0..16 {
        m.feed_interleaved(0, &prog, 1).unwrap();
        m.feed_interleaved(1, &prog, 1).unwrap();
        m.process_block(BLOCK).unwrap();
    }
    let pk_clean = m.master_peak_db_l();
    assert!(
        pk_clean.is_finite() && (-9.0..-3.0).contains(&pk_clean),
        "clean lead-in must meter ~-6 dB, got {pk_clean}"
    );

    // One poisoned block on L: the Inf sample must not enter the
    // ballistics (pre-fix it latched peak=Inf, a fixed point of the
    // 0.95 decay, and rms_sq=Inf, which never drains). The rest of
    // the block still meters the clean program.
    let mut l = prog.clone();
    l[0] = f32::INFINITY;
    m.feed_interleaved(0, &l, 1).unwrap();
    m.feed_interleaved(1, &prog, 1).unwrap();
    m.process_block(BLOCK).unwrap();
    let pk = m.master_peak_db_l();
    assert!(
        pk.is_finite() && pk < 0.0,
        "master peak must stay finite and sane through an Inf block, got {pk}"
    );

    // Clean program resumes: within the 100 ms RMS window the meter
    // reads exactly the clean level again — nothing stuck, at either
    // end (no Inf freeze, no floor lock).
    for _ in 0..64 {
        m.feed_interleaved(0, &prog, 1).unwrap();
        m.feed_interleaved(1, &prog, 1).unwrap();
        m.process_block(BLOCK).unwrap();
    }
    let pk = m.master_peak_db_l();
    let rms = m.master_rms_db_l();
    assert!(
        pk.is_finite() && (-9.0..-3.0).contains(&pk),
        "master peak must recover to the clean level, got {pk}"
    );
    assert!(
        rms.is_finite() && (-12.0..-3.0).contains(&rms),
        "master rms must recover, got {rms}"
    );
}
