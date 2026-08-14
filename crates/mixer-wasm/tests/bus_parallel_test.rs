//! Bus signal-flow honesty tests: buses tap RAW inputs in parallel.
//!
//! Correct model: every input channel (0-127) ALWAYS feeds master directly
//! through its own strip. Each bus slot (idx 128 + bus*16 + slot) taps the
//! RAW input buffer of its assigned source channel — the input strip's
//! mute/fader/EQ/dynamics have ZERO effect on the bus path. The slot runs
//! its own complete chain, then bus sum → bus gain/mute → master.
//!
//! The previous (broken, series) model diverted assigned inputs away from
//! master: muting an input killed its bus feeds too, and assigning a bus
//! did not raise master level. These tests distinguish the two models.
//!
//! Run with: wasm-pack test --node --release

use js_sys::Float32Array;
use wasm_bindgen_test::*;

const SAMPLE_RATE: u32 = 48_000;
const BLOCK_SIZE: u32 = 128;

/// Slot channel index for bus 0, slot 0 (first bus slot).
const SLOT_0_0: u32 = 128;

fn sine(freq: f32, gain: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| gain * (2.0 * std::f32::consts::PI * freq * i as f32 / SAMPLE_RATE as f32).sin())
        .collect()
}

/// Feed a buffer into channel `ch` and process one block, returning the
/// interleaved stereo output as a Rust Vec.
fn run_block(mixer: &mut mixer_wasm::MixerWasm, ch: u32, input: &[f32]) -> Vec<f32> {
    let fa = Float32Array::new_with_length(input.len() as u32);
    fa.copy_from(input);
    mixer.set_channel_input(ch, &fa).expect("set input");
    let out = mixer.process(BLOCK_SIZE).expect("process");
    let mut buf = vec![0.0f32; out.length() as usize];
    out.copy_to(&mut buf);
    buf
}

fn max_abs(buf: &[f32]) -> f32 {
    buf.iter().fold(0.0f32, |a, &b| a.max(b.abs()))
}

/// Fresh mixer with exact level math: limiter off, EQ bypassed on the input
/// channel and (once created) on bus 0 slot 0.
fn mixer_exact() -> mixer_wasm::MixerWasm {
    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();
    mixer
}

/// Bypass the slot's EQ too (it is lazily created by set_bus_source).
fn assign_bus_0_slot_0(mixer: &mut mixer_wasm::MixerWasm) {
    mixer.set_bus_source(0, 0, 0).expect("set bus source");
    mixer.set_eq_bypass(SLOT_0_0, true).unwrap();
}

/// Test a: assigning a bus source must ADD to master (parallel), not divert.
/// Direct path ≈ L0; with the bus slot at default gain 1.0 and bus gain 1.0,
/// master should rise to roughly 2× L0 (both paths default-panned).
#[wasm_bindgen_test]
fn test_bus_is_parallel_not_series() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);
    let mut mixer = mixer_exact();

    // Direct path only.
    let out = run_block(&mut mixer, 0, &input);
    let l0 = max_abs(&out);
    assert!(l0 > 1e-3, "direct path must produce signal, got {l0}");

    // Assign bus 0 slot 0 ← input 0: master must now carry BOTH paths.
    assign_bus_0_slot_0(&mut mixer);
    let out = run_block(&mut mixer, 0, &input);
    let l1 = max_abs(&out);

    assert!(
        l1 > 1.5 * l0,
        "bus assignment must add to master (parallel), got L0={l0:.4} L1={l1:.4} (series model would stay ≈L0)"
    );
}

/// Test b: muting the INPUT channel must not mute its raw bus tap.
/// With input 0 muted, master still carries the bus path alone (≈ L0).
#[wasm_bindgen_test]
fn test_input_mute_does_not_affect_bus_tap() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);
    let mut mixer = mixer_exact();

    let out = run_block(&mut mixer, 0, &input);
    let l0 = max_abs(&out);

    assign_bus_0_slot_0(&mut mixer);
    let out = run_block(&mut mixer, 0, &input);
    let l_both = max_abs(&out);

    // Mute the input strip: only the bus (raw tap) path remains.
    mixer.set_channel_mute(0, true).unwrap();
    let out = run_block(&mut mixer, 0, &input);
    let l_muted = max_abs(&out);

    assert!(
        l_muted > 0.3 * l0,
        "bus tap must survive input mute (raw tap, not processed output): \
         L0={l0:.4} muted={l_muted:.4} — silence means series model"
    );
    assert!(
        l_muted < 0.75 * l_both,
        "input mute should remove the direct path: muted={l_muted:.4} both={l_both:.4}"
    );

    // Unmute: back to both paths.
    mixer.set_channel_mute(0, false).unwrap();
    let out = run_block(&mut mixer, 0, &input);
    let l_restored = max_abs(&out);
    assert!(
        (l_restored - l_both).abs() < 0.15 * l_both,
        "unmuting input should restore both paths: restored={l_restored:.4} both={l_both:.4}"
    );
}

/// Test c: bus mute kills ONLY the bus path; the direct input path remains.
#[wasm_bindgen_test]
fn test_bus_mute_kills_bus_path_only() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);
    let mut mixer = mixer_exact();

    let out = run_block(&mut mixer, 0, &input);
    let l0 = max_abs(&out);

    assign_bus_0_slot_0(&mut mixer);
    let out = run_block(&mut mixer, 0, &input);
    let l_both = max_abs(&out);

    // Mute the bus: master falls back to the direct path alone (≈ L0).
    mixer.set_bus_mute(0, true);
    let out = run_block(&mut mixer, 0, &input);
    let l_bus_muted = max_abs(&out);

    assert!(
        (l_bus_muted - l0).abs() < 0.2 * l0,
        "bus mute must leave direct path at ≈L0: L0={l0:.4} bus_muted={l_bus_muted:.4}"
    );

    // Unmute the bus: back to both paths.
    mixer.set_bus_mute(0, false);
    let out = run_block(&mut mixer, 0, &input);
    let l_restored = max_abs(&out);
    assert!(
        (l_restored - l_both).abs() < 0.15 * l_both,
        "bus unmute should restore both paths: restored={l_restored:.4} both={l_both:.4}"
    );
}

/// Test d: the SLOT's fader scales only the bus path, not the direct path.
/// Slot gain 0.5 → master ≈ L0 × (1 + 0.5·panfactor) ≈ 1.5 × L0.
#[wasm_bindgen_test]
fn test_slot_fader_scales_bus_path_only() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);
    let mut mixer = mixer_exact();

    let out = run_block(&mut mixer, 0, &input);
    let l0 = max_abs(&out);

    assign_bus_0_slot_0(&mut mixer);
    mixer.set_channel_gain(SLOT_0_0, 0.5).unwrap();

    let out = run_block(&mut mixer, 0, &input);
    let l = max_abs(&out);

    assert!(
        l > 1.2 * l0 && l < 1.8 * l0,
        "slot fader 0.5 should give partial bus contribution (≈1.5×L0): \
         L0={l0:.4} got={l:.4} (input ch gain stays 1.0)"
    );
}
