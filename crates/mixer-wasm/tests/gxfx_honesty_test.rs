//! Honesty suite for the gxfx (guitarix) per-strip FX chains — the
//! remaining catalog effects, driven through the REAL C++ DSP via
//! [`GxFxRack`].
//!
//! Per AGENTS.md's honesty rule, every DSP path must pass an end-to-end
//! known-answer test before being trusted. Stereo delay (rack-level) and
//! echo (console-level) are already covered; this file drives the remaining
//! nine testable effects — wah (0), overdrive (1), distortion (2),
//! compressor (3), chorus (4), flanger (5), phaser (6), tremolo (7),
//! reverb (10) — natively, with musically active (never bypass-like)
//! parameter settings, asserting the PHYSICAL claim each measurement
//! verifies.
//!
//! VERDICT: all eight are REAL DSP. One documented deviation from the
//! task brief: gcb_95 "wah" is a MANUAL pedal wah (LTI at a fixed pedal
//! position — see the finding note in its test), not an envelope-driven
//! autowah, so the level-dependent-IR claim is verified on the PEDAL
//! sweep instead.
//!
//! Every test also checks the two universal claims: finite output (no
//! NaN/Inf anywhere) and silence-in → silence-out after settle.
//!
//! DSP semantics cross-checked against the faust sources under
//! vendor/guitarix/trunk/src/LV2/faust-generated/ (param port indexes and
//! ranges from the catalog, which mirrors OctOBX's src/gxfx-params.ts).

use mixer_wasm::gxfx::GxFxRack;

const FS: u32 = 48_000;
const BS: usize = 128;

// ===========================================================================
// Harness helpers
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
/// collect both outputs (the module's own impulse test pattern).
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

/// Mono convenience: signal on L, digital silence on R.
fn run_mono(rack: &mut GxFxRack, pair: usize, input: &[f32]) -> Vec<f32> {
    run_pair(rack, pair, input, &vec![0.0; input.len()]).0
}

fn rms(x: &[f32]) -> f64 {
    let n = x.len().max(1) as f64;
    (x.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / n).sqrt()
}

/// Non-overlapping windowed RMS series.
fn windowed_rms(x: &[f32], win: usize) -> Vec<f64> {
    x.chunks(win).filter(|c| c.len() == win).map(rms).collect()
}

fn mean(v: &[f64]) -> f64 {
    v.iter().sum::<f64>() / v.len() as f64
}

/// Sign changes of (v - mean) — the classic mean-crossing count.
fn mean_crossings(v: &[f64]) -> usize {
    let m = mean(v);
    v.windows(2)
        .filter(|w| (w[0] - m) <= 0.0 && (w[1] - m) > 0.0 || (w[0] - m) >= 0.0 && (w[1] - m) < 0.0)
        .count()
}

/// Zero-crossing rate in Hz over a slice.
fn zcr_hz(x: &[f32]) -> f64 {
    let crossings = x
        .windows(2)
        .filter(|w| (w[0] <= 0.0 && w[1] > 0.0) || (w[0] >= 0.0 && w[1] < 0.0))
        .count();
    crossings as f64 * FS as f64 / x.len() as f64
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

/// Universal claim: after the effect has run its signal, feeding digital
/// silence yields digital silence (|out| < 1e-9 ≈ -180 dBFS on a 1.0-scale
/// signal, below the 24-bit LSB) on BOTH channels once the effect's memory
/// (delay lines, tails) has drained. The exact-zero threshold would be
/// dishonest for ringing IIRs whose double states decay exponentially but
/// still cast to non-zero f32 for a while.
fn drain_and_assert_silent(rack: &mut GxFxRack, pair: usize, secs: f64, what: &str) {
    let z = zeros(secs);
    let (ol, or) = run_pair(rack, pair, &z, &z);
    let n = BS.min(ol.len());
    let tail_l = &ol[ol.len() - n..];
    let tail_r = &or[or.len() - n..];
    let ml = tail_l.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    let mr = tail_r.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    assert!(
        ml < 1e-9 && mr < 1e-9,
        "{what}: silence-in must give silence-out after settle (L {ml:e}, R {mr:e} — \
         both must be < 1e-9, i.e. below -180 dBFS)"
    );
}

/// Normalized autocorrelation of the mean-removed series at `lag` entries:
/// 1.0 = the pattern repeats exactly at that lag, ~0 = unrelated. This is
/// the shape-agnostic "periodic at rate 1/(lag·window)" check (a comb
/// envelope carries its energy at LFO harmonics, not the fundamental, so a
/// single-bin detector would under-report a perfectly periodic sweep).
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
// Tremolo (id 7)
// ===========================================================================

#[test]
fn tremolo_deep_amplitude_modulation_at_lfo_rate() {
    let mut r = rack();
    r.set_slot(0, 0, 7).unwrap();
    // set_slot loaded the catalog defaults (wet 50, mode 0, depth 0.5,
    // freq 5) — override to musically active extremes.
    r.set_param(0, 0, 0, 100.0).unwrap(); // dry/wet 100 % (task spec)
    r.set_param(0, 0, 1, 1.0).unwrap(); // mode = sine
    r.set_param(0, 0, 2, 1.0).unwrap(); // depth max
    let lfo = r.param(0, 0, 3).unwrap(); // freq: keep catalog default 5 Hz
    assert!((lfo - 5.0).abs() < 1e-6, "tremolo freq default is 5 Hz");
    r.set_enabled(0, 0, true).unwrap();

    let sig = sine(500.0, 0.5, 3.0);
    let out = run_mono(&mut r, 0, &sig);
    assert_finite(&out, "tremolo");

    // Envelope of the steady tone: 50 ms windows (LFO period = 200 ms),
    // skipping 0.3 s of attack/settle.
    let env = windowed_rms(&out, samples(0.05));
    let env = &env[samples(0.3) / samples(0.05)..];
    let emax = env.iter().cloned().fold(0.0f64, f64::max);
    let emin = env.iter().cloned().fold(f64::INFINITY, f64::min);
    let ratio = emax / emin;
    println!("tremolo: env max {emax:.5} min {emin:.6} ratio {ratio:.1}");
    assert!(
        ratio > 4.0,
        "PHYSICAL: depth max + wet 100 must swing the tone's envelope deeply \
         (windowed-RMS max/min > 4); got {ratio:.2} (max {emax:.5}, min {emin:.6})"
    );

    // LFO rate: the envelope returns to its mean twice per cycle.
    let dur = env.len() as f64 * 0.05;
    let rate = mean_crossings(env) as f64 / 2.0 / dur;
    println!("tremolo: envelope rate {rate:.2} Hz (expect ~5)");
    assert!(
        (3.0..=7.0).contains(&rate),
        "PHYSICAL: sine LFO at 5 Hz must modulate the envelope at ~5 Hz \
         (mean-crossing rate {rate:.2} Hz)"
    );

    drain_and_assert_silent(&mut r, 0, 1.0, "tremolo");
}

// ===========================================================================
// Overdrive / scream (id 1)
// ===========================================================================

#[test]
fn overdrive_scream_nonlinear_and_compressing() {
    let mut r = rack();
    r.set_slot(0, 0, 1).unwrap();
    r.set_param(0, 0, 0, 1.0).unwrap(); // SCREAM max (default 0.5)
    r.set_enabled(0, 0, true).unwrap();

    // Loud drive: 0.8-amplitude 500 Hz sine (96 samples per period → the
    // 30720-sample Goertzel window is exactly 320 periods, so 500/1000/1500
    // Hz bins are leakage-free).
    let loud_in = sine(500.0, 0.8, 1.5);
    let loud_out = run_mono(&mut r, 0, &loud_in);
    assert_finite(&loud_out, "overdrive loud");
    let m = loud_out.len() - 30720;
    let win = &loud_out[m..];
    let p1 = goertzel_f32(win, 500.0, FS as f64);
    let p2 = goertzel_f32(win, 1000.0, FS as f64);
    let p3 = goertzel_f32(win, 1500.0, FS as f64);
    let peak = win.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    let harm = (p2 + p3) / p1;
    println!(
        "scream: P1 {p1:.6} P2 {p2:.6} P3 {p3:.6} harm {harm:.3} peak {peak:.3} \
         (clamp ceiling 0.4514)"
    );
    assert!(
        harm > 0.05,
        "PHYSICAL: a nonlinear drive must generate harmonic energy from a \
         pure sine ((P2+P3)/P1 > 5 %); got {harm:.3}"
    );
    assert!(
        (peak - 0.4514).abs() < 1e-3,
        "PHYSICAL: the hard clamp at +0.4514 must engage on a loud sine \
         (output peak pinned at {peak:.4})"
    );

    // Gain compression: same rack, a 40x quieter passage after settle (its
    // ~2x resonator pre-gain keeps it far below the clamp → linear region).
    let quiet_in = sine(500.0, 0.02, 1.5);
    let quiet_out = run_mono(&mut r, 0, &quiet_in);
    assert_finite(&quiet_out, "overdrive quiet");
    let rl = rms(&loud_out[loud_out.len() - samples(0.5)..]);
    let rq = rms(&quiet_out[quiet_out.len() - samples(0.5)..]);
    let ri_l = rms(&loud_in[loud_in.len() - samples(0.5)..]);
    let ri_q = rms(&quiet_in[quiet_in.len() - samples(0.5)..]);
    let in_ratio = ri_l / ri_q;
    let out_ratio = rl / rq;
    println!(
        "scream: rms loud {rl:.5} quiet {rq:.5} out ratio {out_ratio:.2} (in ratio {in_ratio:.1})"
    );
    assert!(
        out_ratio < in_ratio / 4.0,
        "PHYSICAL: the drive nonlinearity must compress (output loud/quiet \
         {out_ratio:.2} well below input {in_ratio:.1} — a linear/no-op effect \
         would pass the ratio through); the clamp at +0.45/-0.25 caps loud \
         passages"
    );

    drain_and_assert_silent(&mut r, 0, 1.0, "overdrive");
}

// ===========================================================================
// Distortion / bossds1 (id 2) — BOSS DS-1 style, oversampled via zita
// ===========================================================================

#[test]
fn distortion_bossds1_nonlinear_and_compressing() {
    let mut r = rack();
    r.set_slot(0, 0, 2).unwrap();
    // defaults: level -2 dB, tone 0.5, drive 0.5 — level to unity, full
    // drive (gain into the asymmetric diode clipper), tone at its neutral
    // mid. (In this faust model higher TONE tilts DARKER, not brighter —
    // probe-swept; 0.5 leaves the spectrum most open.)
    r.set_param(0, 0, 0, 0.0).unwrap(); // LEVEL 0 dB
    r.set_param(0, 0, 1, 0.5).unwrap(); // TONE neutral
    r.set_param(0, 0, 2, 1.0).unwrap(); // DRIVE max
    r.set_enabled(0, 0, true).unwrap();

    // Loud drive: 0.8-amplitude 500 Hz sine (96 samples per period → the
    // 30720-sample Goertzel window is exactly 320 periods, so 500/1000/
    // 1500 Hz bins are leakage-free). The DS-1 diode stage runs 2x
    // oversampled (96 kHz via zita), so aliasing cannot fake harmonics
    // into the bins either. Saturation is odd-dominant (P2 stays ~0:
    // deep in the tables both clip branches converge) — P3 is the
    // honest carrier of the claim.
    let loud_in = sine(500.0, 0.8, 1.5);
    let loud_out = run_mono(&mut r, 0, &loud_in);
    assert_finite(&loud_out, "distortion loud");
    let m = loud_out.len() - 30720;
    let win = &loud_out[m..];
    let p1 = goertzel_f32(win, 500.0, FS as f64);
    let p2 = goertzel_f32(win, 1000.0, FS as f64);
    let p3 = goertzel_f32(win, 1500.0, FS as f64);
    let peak = win.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    let harm = (p2 + p3) / p1;
    println!(
        "bossds1: P1 {p1:.6} P2 {p2:.6} P3 {p3:.6} harm {harm:.3} peak {peak:.3} \
         (clipper asymptote ~0.19)"
    );
    assert!(
        harm > 0.05,
        "PHYSICAL: the diode clipper at full drive must generate harmonic \
         energy from a pure sine ((P2+P3)/P1 > 5 %); got {harm:.3}"
    );
    assert!(
        peak < 0.25,
        "PHYSICAL: the clip tables bound the output (saturating x/(3+x) \
         curves — peak {peak:.3} must stay under the asymptote band)"
    );

    // Gain compression: same rack, a 40x quieter passage after settle —
    // below the clipper's knee, so it rides the ~2x small-signal gain
    // while the loud sine is squashed against the asymptote (probe:
    // stage gain falls 1.9 → 0.22 as input rises 0.05 → 0.8).
    let quiet_in = sine(500.0, 0.02, 1.5);
    let quiet_out = run_mono(&mut r, 0, &quiet_in);
    assert_finite(&quiet_out, "distortion quiet");
    let rl = rms(&loud_out[loud_out.len() - samples(0.5)..]);
    let rq = rms(&quiet_out[quiet_out.len() - samples(0.5)..]);
    let ri_l = rms(&loud_in[loud_in.len() - samples(0.5)..]);
    let ri_q = rms(&quiet_in[quiet_in.len() - samples(0.5)..]);
    let in_ratio = ri_l / ri_q;
    let out_ratio = rl / rq;
    println!(
        "bossds1: rms loud {rl:.5} quiet {rq:.5} out ratio {out_ratio:.2} (in ratio {in_ratio:.1})"
    );
    assert!(
        out_ratio < in_ratio / 4.0,
        "PHYSICAL: the drive nonlinearity must compress (output loud/quiet \
         {out_ratio:.2} well below input {in_ratio:.1} — a linear/no-op effect \
         would pass the ratio through)"
    );

    // FINDING (documented): unlike the ten earlier effects, bossds1 does
    // not drain to exact digital silence. Its faust code has a
    // signal-independent control-smoothing recursion
    // (fRec1 = 0.007·DRIVE + 0.993·fRec1[1] — the "control voltage"
    // charging to its operating point) whose zero-input steady state is
    // a ~3e-7 DC offset (~-130 dBFS) on BOTH channels (the never-driven
    // R output sits at exactly that floor). The signal-driven tail
    // itself does decay onto that floor (L: 2.3e-5 after 1 s → 1.5e-7
    // after 3 s, tone-stack pole τ ≈ 0.16 s). So the honest universal
    // claim for this effect is: after drain, output < 1e-6 (< -120
    // dBFS, inaudible by any margin) AND still decaying (late window
    // under early window) — no ringing, no stuck signal, just the tiny
    // model DC floor.
    let z = zeros(3.0);
    let (ol, or_) = run_pair(&mut r, 0, &z, &z);
    // Floor claim on the final block (drain helper's convention); decay
    // claim against the previous full second.
    let w = samples(1.0);
    let max_of = |x: &[f32]| x.iter().fold(0.0f32, |m, &v| m.max(v.abs()));
    let early_l = max_of(&ol[ol.len() - 2 * w..ol.len() - w]);
    let late_l = max_of(&ol[ol.len() - BS..]);
    let early_r = max_of(&or_[or_.len() - 2 * w..or_.len() - w]);
    let late_r = max_of(&or_[or_.len() - BS..]);
    println!(
        "bossds1: drain L {late_l:.3e} (early {early_l:.3e}), R {late_r:.3e} (early {early_r:.3e})"
    );
    assert!(
        late_l < 1e-6 && late_r < 1e-6,
        "PHYSICAL: silence-in must drain to the model's DC floor (< 1e-6, \
         -120 dBFS); got L {late_l:.3e} R {late_r:.3e}"
    );
    assert!(
        late_l <= early_l && late_r <= early_r,
        "PHYSICAL: the tail must DECAY onto the floor, not ring (late \
         L {late_l:.3e} vs early {early_l:.3e}, R {late_r:.3e} vs {early_r:.3e})"
    );
}
// ===========================================================================
// Compressor (id 3)
// ===========================================================================

#[test]
fn compressor_gain_reduces_loud_relative_to_quiet() {
    let mut r = rack();
    r.set_slot(0, 0, 3).unwrap();
    // defaults: ratio 2, knee 3, threshold -20, release 0.5, attack 0.002.
    r.set_param(0, 0, 0, 10.0).unwrap(); // ratio 10:1
    r.set_param(0, 0, 2, -30.0).unwrap(); // threshold -30 dB
    r.set_param(0, 0, 3, 0.25).unwrap(); // release 250 ms
    r.set_param(0, 0, 4, 0.005).unwrap(); // attack 5 ms
    r.set_enabled(0, 0, true).unwrap();

    // Quiet passage: -36 dBFS tone — below threshold, ~unity (makeup
    // neutral: the effect has no makeup control at all).
    let quiet_in = sine(500.0, 0.016, 2.0);
    let quiet_out = run_mono(&mut r, 0, &quiet_in);
    assert_finite(&quiet_out, "compressor quiet");
    // Loud passage: -6 dBFS tone — 24 dB over threshold, ratio 10.
    let loud_in = sine(500.0, 0.5, 2.0);
    let loud_out = run_mono(&mut r, 0, &loud_in);
    assert_finite(&loud_out, "compressor loud");

    let rq_in = rms(&quiet_in[quiet_in.len() - samples(0.5)..]);
    let rl_in = rms(&loud_in[loud_in.len() - samples(0.5)..]);
    let rq = rms(&quiet_out[quiet_out.len() - samples(0.5)..]);
    let rl = rms(&loud_out[loud_out.len() - samples(0.5)..]);
    let in_ratio = rl_in / rq_in;
    let out_ratio = rl / rq;
    println!(
        "compressor: quiet {rq:.5} (in {rq_in:.5}, gain {:.3}) loud {rl:.5} — out ratio \
         {out_ratio:.2} vs in ratio {in_ratio:.1}",
        rq / rq_in
    );
    assert!(
        out_ratio < in_ratio / 4.0,
        "PHYSICAL: 10:1 compression above -30 dB must reduce the loud/quiet \
         RMS ratio (out {} vs in {})",
        out_ratio,
        in_ratio
    );
    assert!(
        (0.7..1.4).contains(&(rq / rq_in)),
        "PHYSICAL: below threshold (no makeup) the compressor is ~unity; \
         quiet gain {}",
        rq / rq_in
    );

    drain_and_assert_silent(&mut r, 0, 1.5, "compressor");
}

// ===========================================================================
// Wah (id 0) — GCB-95 is a MANUAL pedal wah (LTI at fixed HOTPOTZ)
// ===========================================================================

#[test]
fn wah_pedal_sweeps_the_resonant_band() {
    // FINDING (documented): gcb_95 is the faust "Linear filter simulating
    // the GCB 95 crybaby circuit" — the filter state that sweeps the band
    // (`fRec0`) is a smoothing of HOTPOTZ ONLY; no input envelope follower
    // exists (autowah is a different, uncatalogued effect). The physical
    // wah claim is therefore verified on the PEDAL: the resonant band at
    // two pedal extremes must differ (impulse-tail dominant frequency),
    // plus the bandpass character (LF attenuated vs band-passed mid).

    let mut zcrs = [0.0f64; 2];
    for (k, &hotpotz) in [0.15f32, 0.85f32].iter().enumerate() {
        let mut r = rack();
        r.set_slot(0, 0, 0).unwrap();
        r.set_param(0, 0, 1, hotpotz).unwrap(); // HOTPOTZ pedal
        r.set_enabled(0, 0, true).unwrap();
        // 0.3 s of silence (lets the HOTPOTZ one-pole settle), impulse,
        // then 0.7 s of ring-out.
        let mut sig = zeros(0.3);
        sig.push(1.0);
        sig.extend(zeros(0.7));
        let out = run_mono(&mut r, 0, &sig);
        assert_finite(&out, "wah impulse");
        let t0 = samples(0.3);
        let tail = &out[t0 + 100..t0 + samples(0.2)];
        zcrs[k] = zcr_hz(tail);
        println!("wah: hotpotz {hotpotz} tail ZCR {:#.1} Hz", zcrs[k]);
    }
    let ratio = zcrs[0].max(zcrs[1]) / zcrs[0].min(zcrs[1]);
    println!("wah: ZCR ratio between pedal extremes {ratio:.2}");
    assert!(
        ratio > 1.4,
        "PHYSICAL: the wah pedal must sweep the resonant band — impulse-tail \
         dominant frequency at HOTPOTZ 0.15 vs 0.85 differs by >40 %; got \
         ratio {ratio:.2} ({:.1} Hz vs {:.1} Hz)",
        zcrs[0],
        zcrs[1]
    );

    // Bandpass character at a fixed mid-pedal position: very-low-frequency
    // input attenuated vs mid-frequency at matched input RMS.
    let mut r = rack();
    r.set_slot(0, 0, 0).unwrap();
    r.set_param(0, 0, 1, 0.5).unwrap(); // mid pedal
    r.set_enabled(0, 0, true).unwrap();
    let lo_in = sine(60.0, 0.2, 1.0);
    let mid_in = sine(800.0, 0.2, 1.0);
    let lo_out = run_mono(&mut r, 0, &lo_in);
    let mid_out = run_mono(&mut r, 0, &mid_in);
    assert_finite(&lo_out, "wah low");
    assert_finite(&mid_out, "wah mid");
    let g_lo = rms(&lo_out[lo_out.len() - samples(0.4)..]);
    let g_mid = rms(&mid_out[mid_out.len() - samples(0.4)..]);
    println!(
        "wah: gain @60 Hz {g_lo:.5} vs @800 Hz {g_mid:.5} (ratio {:.1})",
        g_mid / g_lo
    );
    assert!(
        g_mid / g_lo > 4.0,
        "PHYSICAL: a wah is a bandpass — 60 Hz must be strongly attenuated \
         vs the 800 Hz band at matched input RMS; ratio {}",
        g_mid / g_lo
    );

    drain_and_assert_silent(&mut r, 0, 1.0, "wah");
}

// ===========================================================================
// Chorus (id 4) — stereo
// ===========================================================================

#[test]
fn chorus_periodic_envelope_modulation() {
    let mut r = rack();
    r.set_slot(0, 0, 4).unwrap();
    // defaults: level 0.5, delay 0.02, depth 0.02, freq 3 — drive the
    // modulation depth hard for an unambiguous measurement.
    r.set_param(0, 0, 0, 1.0).unwrap(); // level (wet) max
    r.set_param(0, 0, 2, 0.5).unwrap(); // depth: delay swings ±50 %
    let freq = r.param(0, 0, 3).unwrap(); // keep default 3 Hz LFO
    assert!((freq - 3.0).abs() < 1e-6);
    r.set_enabled(0, 0, true).unwrap();

    let sig = sine(440.0, 0.4, 3.0);
    let (ol, or) = run_pair(&mut r, 0, &sig, &sig);
    assert_finite(&ol, "chorus L");
    assert_finite(&or, "chorus R");

    // Envelope: 250-sample (5.21 ms) windows, skip 0.3 s settle. 250 is
    // chosen so one 3 Hz LFO period (16000 samples) is exactly 64 windows.
    let env = windowed_rms(&ol, 250);
    let env = &env[samples(0.3) / 250..];
    let emax = env.iter().cloned().fold(0.0f64, f64::max);
    let emin = env.iter().cloned().fold(f64::INFINITY, f64::min);
    let total = rms(&ol[samples(0.3)..]);
    let in_rms = rms(&sig[samples(0.3)..]);
    println!(
        "chorus: env max/min {:.2} ({emax:.4}/{emin:.4}), long-term RMS ratio {:.3}",
        emax / emin,
        total / in_rms
    );
    assert!(
        emax / emin > 1.5,
        "PHYSICAL: the sweeping delay-line comb must amplitude-modulate a \
         steady tone (envelope max/min > 1.5); got {:.2}",
        emax / emin
    );
    assert!(
        (0.25..2.5).contains(&(total / in_rms)),
        "PHYSICAL: chorus keeps the signal in a sane band (dry at unity + wet \
         at level 1.0): long-term RMS ratio {}",
        total / in_rms
    );

    // Periodicity at the LFO rate: the envelope pattern must repeat at the
    // 3 Hz period (autocorrelation at lag 64 windows = exactly 16000
    // samples = 1/3 s; the comb's energy sits at LFO harmonics, so the
    // shape-repetition correlation is the honest detector).
    let rho = autocorr_at(env, 64);
    println!("chorus: envelope autocorrelation at one LFO period {rho:.3}");
    assert!(
        rho > 0.5,
        "PHYSICAL: the sweeping delay line must modulate the tone periodically \
         at the 3 Hz LFO rate (autocorrelation at one period > 0.5); got {rho:.3}"
    );

    drain_and_assert_silent(&mut r, 0, 3.0, "chorus");
}

// ===========================================================================
// Flanger (id 5)
// ===========================================================================

#[test]
fn flanger_deep_modulation_with_high_feedback() {
    let mut r = rack();
    r.set_slot(0, 0, 5).unwrap();
    // defaults: depth 0.5, width 5, freq 0.2, feedback -0.707, wet 100,
    // mix 0 — push feedback high positive and slow the sweep.
    r.set_param(0, 0, 2, 0.5).unwrap(); // freq 0.5 Hz
    r.set_param(0, 0, 3, 0.9).unwrap(); // feedback +0.9 (high)
    r.set_enabled(0, 0, true).unwrap();
    let wet = r.param(0, 0, 4).unwrap();
    assert!((wet - 100.0).abs() < 1e-6, "flanger wet default 100");

    let sig = sine(440.0, 0.4, 8.0);
    let out = run_mono(&mut r, 0, &sig);
    assert_finite(&out, "flanger");

    // Envelope: 384-sample (8 ms) windows, skip 0.4 s settle; 384 chosen so
    // one 0.5 Hz LFO period (96000 samples) is exactly 250 windows.
    let env = windowed_rms(&out, 384);
    let env = &env[samples(0.4) / 384..];
    let emax = env.iter().cloned().fold(0.0f64, f64::max);
    let emin = env.iter().cloned().fold(f64::INFINITY, f64::min);
    let total = rms(&out[samples(0.4)..]);
    let in_rms = rms(&sig[samples(0.4)..]);
    println!(
        "flanger: env max/min {:.2} ({emax:.4}/{emin:.4}), RMS ratio {:.3}",
        emax / emin,
        total / in_rms
    );
    assert!(
        emax / emin > 3.0,
        "PHYSICAL: a high-feedback flanger combs deeply — the 440 Hz tone's \
         envelope must swing max/min > 3 at the sweep rate; got {:.2}",
        emax / emin
    );
    assert!(
        (0.1..3.0).contains(&(total / in_rms)),
        "PHYSICAL: flanger stays in a sane loudness band; RMS ratio {}",
        total / in_rms
    );

    // Periodic at the 0.5 Hz sweep: autocorrelation at lag 250 windows =
    // exactly one 2 s LFO period (8 s of signal → ~3.8 usable periods).
    let rho = autocorr_at(env, 250);
    println!("flanger: envelope autocorrelation at one LFO period {rho:.3}");
    assert!(
        rho > 0.5,
        "PHYSICAL: the deep comb modulation must be periodic at the 0.5 Hz \
         sweep rate (autocorrelation at one period > 0.5); got {rho:.3}"
    );

    drain_and_assert_silent(&mut r, 0, 3.0, "flanger");
}

// ===========================================================================
// Phaser (id 6)
// ===========================================================================

#[test]
fn phaser_notches_sweep_at_lfo_rate() {
    let mut r = rack();
    r.set_slot(0, 0, 6).unwrap();
    // defaults: dry/wet 50, level 0 dB, speed 0.5 — full wet, 1 Hz sweep.
    r.set_param(0, 0, 0, 100.0).unwrap(); // dry/wet 100
    r.set_param(0, 0, 2, 1.0).unwrap(); // speed 1 Hz
    r.set_enabled(0, 0, true).unwrap();

    // Broadband-ish deterministic input: four tones spread over 2.5 octaves
    // so some tone always sits near a sweeping notch.
    let sig = multitone(&[300.0, 700.0, 1700.0, 3900.0], 0.1, 4.0);
    let out = run_mono(&mut r, 0, &sig);
    assert_finite(&out, "phaser");

    // Envelope: 480-sample (10 ms) windows, skip 0.3 s settle; 480 chosen
    // so one 1 Hz LFO period (48000 samples) is exactly 100 windows.
    let env = windowed_rms(&out, 480);
    let env = &env[samples(0.3) / 480..];
    let emax = env.iter().cloned().fold(0.0f64, f64::max);
    let emin = env.iter().cloned().fold(f64::INFINITY, f64::min);
    println!(
        "phaser: env max/min {:.2} ({emax:.4}/{emin:.4})",
        emax / emin
    );
    assert!(
        emax / emin > 1.25,
        "PHYSICAL: sweeping notches must dip the broadband envelope \
         (max/min > 1.25); got {:.2}",
        emax / emin
    );

    // Dips periodic at the 1 Hz sweep: autocorrelation at lag 100 windows =
    // exactly one 1 s LFO period.
    let rho = autocorr_at(env, 100);
    println!("phaser: envelope autocorrelation at one LFO period {rho:.3}");
    assert!(
        rho > 0.5,
        "PHYSICAL: notches sweep at the 1 Hz LFO — the broadband envelope's \
         dip pattern must repeat each period (autocorrelation > 0.5); got {rho:.3}"
    );

    drain_and_assert_silent(&mut r, 0, 3.0, "phaser");
}

// ===========================================================================
// Reverb (id 10) — stereo
// ===========================================================================

#[test]
fn reverb_impulse_tail_is_real_and_decays() {
    let mut r = rack();
    r.set_slot(0, 0, 10).unwrap();
    // defaults: wet 50, lfo 0.2, roomsize 0.5, damp 0.2, mode 0 — full wet,
    // a medium room that fully decays within the capture window.
    r.set_param(0, 0, 0, 100.0).unwrap(); // dry/wet 100 (task spec)
    r.set_param(0, 0, 2, 0.7).unwrap(); // roomsize
    r.set_param(0, 0, 3, 0.5).unwrap(); // damp
    r.set_enabled(0, 0, true).unwrap();

    let t0 = samples(0.25);
    let mut sig = zeros(0.25);
    sig.push(1.0);
    sig.extend(zeros(4.0));
    let (ol, or) = run_pair(&mut r, 0, &sig, &sig);
    assert_finite(&ol, "reverb L");
    assert_finite(&or, "reverb R");

    // Dry path suppressed at full wet: the impulse sample itself must NOT
    // come through at unity (dry gain = 1 - wet/100 = 0; only the tank's
    // 0.015-ish wet injection remains).
    println!("reverb: dry sample out L {:.4} R {:.4}", ol[t0], or[t0]);
    assert!(
        ol[t0].abs() < 0.1 && or[t0].abs() < 0.1,
        "PHYSICAL: dry path suppressed at wet 100 — impulse sample passes at \
         {:.3}/{:.3}, not unity",
        ol[t0],
        or[t0]
    );

    // Tail energy in 100 ms buckets (both channels must ring).
    let bucket = |x: &[f32], from_s: f64, to_s: f64| {
        let a = t0 + samples(from_s);
        let b = t0 + samples(to_s);
        rms(&x[a..b.min(x.len())])
    };
    let e_early = (bucket(&ol, 0.01, 0.2) + bucket(&or, 0.01, 0.2)) / 2.0;
    let e_mid = (bucket(&ol, 1.0, 1.3) + bucket(&or, 1.0, 1.3)) / 2.0;
    let e_late = (bucket(&ol, 2.7, 3.0) + bucket(&or, 2.7, 3.0)) / 2.0;
    let e_end = (bucket(&ol, 3.7, 4.0) + bucket(&or, 3.7, 4.0)) / 2.0;
    let energy: f64 = ol[t0..]
        .iter()
        .chain(or[t0..].iter())
        .map(|&v| (v as f64) * (v as f64))
        .sum();
    println!(
        "reverb: tail RMS early {e_early:.5} mid {e_mid:.5} late {e_late:.6} end {e_end:.3e}, \
         total energy {energy:.4}"
    );
    assert!(
        e_early > 1e-4 && energy > 1e-4,
        "PHYSICAL: a real reverb spreads the impulse into a non-zero decaying \
         tail over hundreds of ms (early RMS {e_early:.2e}, total energy {energy:.2e}) — \
         a fake would be silent"
    );
    assert!(
        e_mid < e_early / 2.0,
        "PHYSICAL: the tail must DECAY (mid {e_mid:.5} well under early \
         {e_early:.5}) — a fake would ring forever at constant level"
    );
    assert!(
        e_end < e_early / 1000.0,
        "PHYSICAL: the tail must reach near-silence within a few seconds \
         (end {e_end:.3e} vs early {e_early:.5}, < -60 dB)"
    );

    drain_and_assert_silent(&mut r, 0, 2.0, "reverb");
}
