//! Keyed PID mapping (multi-session) tests, run under node.
//!
//! Distinct WebSRT sessions may reuse the same TS PID numbers; the keyed
//! map (`(sessionId << 16) | pid`) lets both streams coexist on separate
//! mixer channels instead of the second mapping clobbering the first.
//!
//! Run with: wasm-pack test --node --release

use js_sys::Float32Array;
use wasm_bindgen_test::*;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;

/// Feed one mono chunk through a keyed mapping (mirrors pcm_fifo_test's
/// feed_stereo helper).
fn feed_keyed(m: &mut mixer_wasm::MixerWasm, key: u32, mono: &[f32]) {
    let fa = Float32Array::new_with_length(mono.len() as u32);
    fa.copy_from(mono);
    m.feed_pcm_keyed(key, &fa).expect("feed_pcm_keyed");
}

/// Session 1 (key 0x1_0101 → ch0) and session 2 (key 0x2_0101 → ch1),
/// both mono and both carrying TS PID 0x101: each strip must carry its
/// OWN signal. Feed DC bursts of 0.04 / 0.08 — the per-channel peak
/// meters (pre-fader) must differ by exactly 20·log10(2) ≈ 6.02 dB.
#[wasm_bindgen_test]
fn keyed_pid_two_sessions_feed_own_channels() {
    let mut m = mixer_wasm::MixerWasm::new(SR, BLOCK, 256).expect("ctor");
    m.set_limiter_enabled(false);
    m.set_eq_bypass(0, true).expect("eq bypass ch0");
    m.set_eq_bypass(1, true).expect("eq bypass ch1");
    m.map_pid_keyed(0x1_0101, 0, 1).expect("map session 1");
    m.map_pid_keyed(0x2_0101, 1, 1).expect("map session 2");

    let lo = vec![0.04f32; BLOCK as usize];
    let hi = vec![0.08f32; BLOCK as usize];
    for _ in 0..4 {
        feed_keyed(&mut m, 0x1_0101, &lo);
        feed_keyed(&mut m, 0x2_0101, &hi);
        m.process_block(BLOCK).expect("process");
    }

    let p0 = m.channel_peak_db(0);
    let p1 = m.channel_peak_db(1);
    // 20·log10(0.08/0.04) = 6.0206 dB; a clobbered mapping would leave
    // one strip at −200 dB (silence).
    assert!(
        (p1 - p0 - 6.0206).abs() < 0.1,
        "meter delta {p1:.3} − {p0:.3} ≠ 6.02 dB — strips carry wrong signals"
    );
    assert!(p0 > -60.0, "session 1 strip silent: {p0:.1} dB");
    assert!(p1 > -60.0, "session 2 strip silent: {p1:.1} dB");
}

/// Unmapping one session's key: feeding it again returns Ok (counted
/// drop, never an error) and bumps the unmapped-drop counter.
#[wasm_bindgen_test]
fn keyed_pid_unmapped_feed_counts_drop() {
    let mut m = mixer_wasm::MixerWasm::new(SR, BLOCK, 256).expect("ctor");
    m.map_pid_keyed(0x1_0101, 0, 1).expect("map session 1");
    m.map_pid_keyed(0x2_0101, 1, 1).expect("map session 2");
    assert_eq!(m.unmapped_pid_count(), 0);

    m.unmap_pid_keyed(0x2_0101);
    feed_keyed(&mut m, 0x2_0101, &[0.08; BLOCK as usize]);
    assert_eq!(m.unmapped_pid_count(), 1);
    // The sibling mapping is untouched.
    assert_eq!(m.pid_channel_keyed(0x1_0101), 0);
}
