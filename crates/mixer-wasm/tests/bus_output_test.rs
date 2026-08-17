//! Main-assign / bus-feed / bus-output / compressor-GR tests for the
//! nine-engine architecture (staging → engine instances → bus tail).
//!
//! Covers the Phase B exports:
//! - `set_channel_main_assign(false)`: strip absent from MASTER but
//!   still audible via an assigned bus slot (staging is unaffected).
//! - `set_bus_feeds_main(false)`: bus absent from master but
//!   `take_bus_output` still returns the bus's own signal (drain
//!   semantics + invalid-bus handling).
//! - bus_muted: `take_bus_output` publishes silence.
//! - `channel_comp_gr_db`: > 0 under heavy compression; "gr" present in
//!   `channel_meters_json`.
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

/// Fresh mixer with exact level math: limiter off, EQ bypassed on the
/// input channel and on bus 0 slot 0 (lazily created by set_bus_source).
fn mixer_exact() -> mixer_wasm::MixerWasm {
    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();
    mixer
}

fn assign_bus_0_slot_0(mixer: &mut mixer_wasm::MixerWasm) {
    mixer.set_bus_source(0, 0, 0).expect("set bus source");
    mixer.set_eq_bypass(SLOT_0_0, true).unwrap();
}

/// Test a: main_assign(false) removes the DIRECT path from master but
/// the strip stays audible through its assigned bus slot (it is still
/// staged and still feeds the bus engine).
#[wasm_bindgen_test]
fn test_main_assign_off_still_audible_via_bus() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);
    let mut mixer = mixer_exact();
    assign_bus_0_slot_0(&mut mixer);

    // Both paths (direct + bus).
    let out = run_block(&mut mixer, 0, &input);
    let l_both = max_abs(&out);
    assert!(
        l_both > 1e-3,
        "both paths must produce signal, got {l_both}"
    );

    // Unassign from main: master must carry ONLY the bus path now.
    mixer.set_channel_main_assign(0, false);
    let out = run_block(&mut mixer, 0, &input);
    let l_bus_only = max_abs(&out);

    assert!(
        l_bus_only > 0.3 * l_both,
        "bus path must survive main-unassign: both={l_both:.4} bus_only={l_bus_only:.4}"
    );
    assert!(
        l_bus_only < 0.75 * l_both,
        "main-unassign must remove the direct path: both={l_both:.4} bus_only={l_bus_only:.4}"
    );

    // Re-assign: both paths return.
    mixer.set_channel_main_assign(0, true);
    let out = run_block(&mut mixer, 0, &input);
    let l_restored = max_abs(&out);
    assert!(
        (l_restored - l_both).abs() < 0.15 * l_both,
        "re-assign should restore both paths: restored={l_restored:.4} both={l_both:.4}"
    );

    // Out-of-range indices are ignored, not errors.
    mixer.set_channel_main_assign(300, false);
}

/// Test b: feeds_main(false) keeps the bus out of MASTER, but the bus's
/// own output is still published by take_bus_output (with the same
/// drain contract as take_channel_tap).
#[wasm_bindgen_test]
fn test_bus_feeds_main_off_take_bus_output_still_hot() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);
    let mut mixer = mixer_exact();
    assign_bus_0_slot_0(&mut mixer);

    let out = run_block(&mut mixer, 0, &input);
    let l_both = max_abs(&out);

    mixer.set_bus_feeds_main(0, false);
    let out = run_block(&mut mixer, 0, &input);
    let l_direct_only = max_abs(&out);

    assert!(
        l_direct_only < 0.75 * l_both,
        "feeds_main(false) must remove the bus from master: both={l_both:.4} got={l_direct_only:.4}"
    );
    assert!(
        l_direct_only > 0.2 * l_both,
        "direct path must remain: both={l_both:.4} got={l_direct_only:.4}"
    );

    // The bus's own output is still published (post-gain, interleaved).
    let bus = mixer.take_bus_output(0);
    assert_eq!(
        bus.length(),
        BLOCK_SIZE * 2,
        "bus output is bs×2 interleaved"
    );
    let mut bus_buf = vec![0.0f32; bus.length() as usize];
    bus.copy_to(&mut bus_buf);
    assert!(
        max_abs(&bus_buf) > 1e-3,
        "take_bus_output must carry signal with feeds_main off, got {}",
        max_abs(&bus_buf)
    );

    // Drains per take: a second take without an intervening process is
    // empty; invalid bus indices return empty.
    assert_eq!(
        mixer.take_bus_output(0).length(),
        0,
        "bus output must drain"
    );
    assert_eq!(mixer.take_bus_output(8).length(), 0, "invalid bus → empty");
    assert_eq!(
        mixer.take_bus_output(0u32.wrapping_sub(1)).length(),
        0,
        "invalid (huge) bus → empty"
    );

    // feeds_main back on: master carries both paths again.
    mixer.set_bus_feeds_main(0, true);
    let out = run_block(&mut mixer, 0, &input);
    let l_restored = max_abs(&out);
    assert!(
        (l_restored - l_both).abs() < 0.15 * l_both,
        "feeds_main(true) should restore both paths: restored={l_restored:.4} both={l_both:.4}"
    );
}

/// Test c: a muted bus publishes silence via take_bus_output (and, as
/// pinned by bus_parallel_test, the direct input path is unaffected).
#[wasm_bindgen_test]
fn test_bus_muted_publishes_silence() {
    let input = sine(440.0, 0.5, BLOCK_SIZE as usize);
    let mut mixer = mixer_exact();
    assign_bus_0_slot_0(&mut mixer);

    mixer.set_bus_mute(0, true);
    let out = run_block(&mut mixer, 0, &input);

    // Master: direct path only.
    let l_direct = max_abs(&out);
    assert!(l_direct > 1e-3, "direct path must remain, got {l_direct}");

    // Bus output: silence (post-mute → zeros), still a full block.
    let bus = mixer.take_bus_output(0);
    assert_eq!(
        bus.length(),
        BLOCK_SIZE * 2,
        "muted bus still publishes a block"
    );
    let mut bus_buf = vec![0.0f32; bus.length() as usize];
    bus.copy_to(&mut bus_buf);
    assert!(
        max_abs(&bus_buf) < 1e-7,
        "muted bus must publish silence, got {}",
        max_abs(&bus_buf)
    );
}

/// Test d: channel_comp_gr_db reports the strip's compressor gain
/// reduction (> 0 under heavy compression, 0 without a compressor), and
/// channel_meters_json carries the "gr" field.
#[wasm_bindgen_test]
fn test_channel_comp_gr_db_under_heavy_compression() {
    // DC drive (the engine compressor's envelope follower is poisoned by
    // exact-zero samples — see tests/dynamics_honesty.rs — so avoid
    // zero-crossings).
    let input = vec![0.9f32; BLOCK_SIZE as usize];

    let mut mixer = mixer_wasm::MixerWasm::new(SAMPLE_RATE, BLOCK_SIZE, 2).unwrap();
    mixer.set_limiter_enabled(false);
    mixer.set_eq_bypass(0, true).unwrap();
    mixer.enable_compressor(0).expect("enable comp");
    mixer.set_comp_param(0, 0, -40.0).unwrap(); // threshold → heavy
    mixer.set_comp_param(0, 4, 0.0).unwrap(); // makeup 0 (clean readback)

    assert_eq!(
        mixer.channel_comp_gr_db(0),
        0.0,
        "GR must read 0 before any audio"
    );
    assert_eq!(
        mixer.channel_comp_gr_db(1),
        0.0,
        "GR must read 0 without a compressor"
    );

    // Warm up: envelope follower settles (attack 5 ms, ~2.7 ms/block).
    for _ in 0..30 {
        let _ = run_block(&mut mixer, 0, &input);
    }

    let gr = mixer.channel_comp_gr_db(0);
    assert!(
        gr > 1.0,
        "heavy compression (−40 dB thr, 0.9 DC in) must show GR > 1 dB, got {gr}"
    );

    let json = mixer.channel_meters_json();
    assert!(
        json.contains("\"gr\":"),
        "channel_meters_json must carry the gr field: {json}"
    );
    assert!(
        json.contains("\"ch\":0"),
        "channel_meters_json must include channel 0: {json}"
    );
}
