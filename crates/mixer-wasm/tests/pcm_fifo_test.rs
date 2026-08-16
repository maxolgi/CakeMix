//! Chunked PCM delivery honesty test (feed_pcm FIFO semantics).
//!
//! WebSRT delivers PCM in ~23 ms interleaved chunks while the worklet calls
//! process(128) every 2.67 ms. The channel input must behave as a FIFO:
//! every input frame is played exactly once, in order; starvation yields
//! silence, not replayed audio.
//!
//! Regression: the input slot used to be REPLACED on every feed, so only
//! the first 128 frames of each chunk were heard — replayed ~9x per chunk
//! (comb + imaging on the viewer spectrum), 87% of the audio dropped.
//!
//! Run with: wasm-pack test --node --release

use js_sys::Float32Array;
use wasm_bindgen_test::*;

const BLOCK: u32 = 128;

fn feed_stereo(mixer: &mut mixer_wasm::MixerWasm, pid: u16, inter: &[f32]) {
    let fa = Float32Array::new_with_length(inter.len() as u32);
    fa.copy_from(inter);
    mixer.feed_pcm(pid, &fa).expect("feed_pcm");
}

fn process(mixer: &mut mixer_wasm::MixerWasm) -> Vec<f32> {
    let out = mixer.process(BLOCK).expect("process");
    let mut buf = vec![0.0f32; out.length() as usize];
    out.copy_to(&mut buf);
    buf
}

/// Feed 4 blocks worth of a strictly increasing marker ramp as ONE chunk
/// (how WebSRT delivers), then process 4 blocks. Every block must advance:
/// block b's first L sample tracks marker frame b*128 (measuring the
/// channel's static gain from block 0 to stay pan-law agnostic).
#[wasm_bindgen_test]
fn pcm_chunk_played_exactly_once_in_order() {
    let mut m = mixer_wasm::MixerWasm::new(48_000, BLOCK, 256).expect("ctor");
    m.set_limiter_enabled(false);
    m.set_eq_bypass(0, true).expect("eq bypass ch0");
    m.set_eq_bypass(1, true).expect("eq bypass ch1");
    m.map_pid(100, 0, 2).expect("map");
    let mut inter = Vec::new();
    for f in 0..(4 * BLOCK) {
        inter.push(0.1 + 0.001 * f as f32); // L marker
        inter.push(0.1 + 0.001 * f as f32); // R marker
    }
    feed_stereo(&mut m, 100, &inter);

    let b0 = process(&mut m);
    let g = b0[0] / (0.1 + 0.0); // static gain incl. pan law (≈1.0, measured)
    assert!(g > 0.5 && g < 2.0, "implausible static gain {g}");
    let expect_first = |b: usize| g * (0.1 + 0.001 * (b as u32 * BLOCK) as f32);

    for b in 1..4usize {
        let out = process(&mut m);
        let want = expect_first(b);
        assert!(
            (out[0] - want).abs() < 0.02 * (1.0 + want.abs()),
            "block {b} first L {:.4} != marker-derived {:.4} — chunk replay/skip",
            out[0], want
        );
    }
}

/// After the queued frames are consumed, process() must output silence
/// (clean dropout), not a held/replayed buffer.
#[wasm_bindgen_test]
fn pcm_starvation_outputs_silence() {
    let mut m = mixer_wasm::MixerWasm::new(48_000, BLOCK, 256).expect("ctor");
    m.set_limiter_enabled(false);
    m.set_eq_bypass(0, true).expect("eq bypass ch0");
    m.set_eq_bypass(1, true).expect("eq bypass ch1");
    m.map_pid(100, 0, 2).expect("map");
    let inter: Vec<f32> = (0..BLOCK).flat_map(|f| [0.5, 0.5]).collect();
    feed_stereo(&mut m, 100, &inter);
    let _ = process(&mut m); // consumes the block

    let starved = process(&mut m);
    let peak = starved.iter().fold(0.0f32, |a, &s| a.max(s.abs()));
    assert!(peak < 1e-4, "starved block must be silent, peak {peak}");
}

/// Late chunks must not clobber queued audio: two chunks fed back-to-back
/// play as one continuous stream (all 4 blocks in order).
#[wasm_bindgen_test]
fn pcm_chunks_queue_without_loss() {
    let mut m = mixer_wasm::MixerWasm::new(48_000, BLOCK, 256).expect("ctor");
    m.set_limiter_enabled(false);
    m.set_eq_bypass(0, true).expect("eq bypass ch0");
    m.set_eq_bypass(1, true).expect("eq bypass ch1");
    m.map_pid(100, 0, 2).expect("map");
    let chunk = |base: f32| -> Vec<f32> {
        (0..BLOCK).flat_map(|_f| [base, 0.0]).collect()
    };
    feed_stereo(&mut m, 100, &chunk(0.10));
    feed_stereo(&mut m, 100, &chunk(0.20)); // distinct base per chunk

    let b0 = process(&mut m);
    let g = b0[0] / 0.10;
    let b1 = process(&mut m);
    // Second block starts the second chunk's ramp: first sample ≈ g * 0.20.
    assert!(
        (b1[0] - g * 0.20).abs() < 0.02,
        "second chunk lost/replayed: b1[0] {:.4}, want {:.4}",
        b1[0], g * 0.20
    );
}
