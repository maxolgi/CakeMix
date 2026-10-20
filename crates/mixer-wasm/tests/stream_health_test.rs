//! Stream-health gate tests (native, no JS).
//!
//! The elastic playout buffer reconciles clock *drift* (hundreds of ppm),
//! but a pathologically gappy stream — TSBPD latency below the network's
//! jitter, so SRT drops constantly — cannot be patched over: every
//! starvation is a hole and every insert a repeat, and at high rates the
//! repair itself becomes full-scale granular static (the reported
//! "constant loud static when latency is too low"). The stream-health
//! gate fades such a channel to silence and holds it until delivery is
//! clean again. These tests pin: the gate engages on bursty delivery,
//! the channel goes (and stays) silent, inserts stop while gated, the
//! state surfaces in the meters JSON, overflow drops are counted, and —
//! critically — paced/drift-normal streams NEVER gate.
//!
//! Run: cargo test -p mixer-wasm --test stream_health_test -- --nocapture

use mixer_wasm::MixerWasm;

const SR: u32 = 48_000;
const BLOCK: u32 = 128;
const WINDOW_BLOCKS: usize = 375; // 1 s of blocks

fn mixer() -> MixerWasm {
    let mut m = MixerWasm::new(SR, BLOCK, 256).expect("ctor");
    m.set_limiter_enabled(false);
    for ch in 0..2 {
        m.set_eq_bypass(ch, true).expect("eq bypass");
    }
    m.map_pid(100, 0, 2).expect("map pid");
    m
}

/// Interleaved stereo sine chunk (0.5 amplitude) starting at stream
/// frame `base`.
fn sine_chunk(base: usize, frames: usize) -> Vec<f32> {
    let mut inter = Vec::with_capacity(frames * 2);
    for f in 0..frames {
        let v = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * (base + f) as f32 / SR as f32).sin();
        inter.push(v);
        inter.push(v);
    }
    inter
}

fn process(m: &mut MixerWasm) -> f32 {
    let out = m.process_block(BLOCK).expect("process");
    let mut peak = 0.0f32;
    for s in out.chunks(2) {
        peak = peak.max(s[0].abs()).max(s[1].abs());
    }
    peak
}

/// Bursty delivery: one 9000-frame burst per second (the queue cap keeps
/// 8192; the rest overflows), then ~0.85 s of starvation. This is the
/// low-TSBPD-latency pathology: full-rate average, hole-ridden reality.
#[test]
fn bursty_stream_fades_to_silence_then_recovers() {
    let mut m = mixer();
    let mut base = 0usize;

    // First burst: the fast path must engage mid-window — 12 starved
    // blocks ≈ 32 ms of holes, well before the window's 375 blocks.
    let chunk = sine_chunk(base, 9000);
    base += 9000;
    m.feed_interleaved(0, &chunk, 2).expect("feed");
    for b in 0..100 {
        process(&mut m);
        if b == 80 {
            assert_eq!(
                m.degraded_channels(),
                2,
                "fast path must gate within ~100 blocks of burst drain (got {} at block {b})",
                m.degraded_channels()
            );
        }
    }

    // Keep it degraded through a second burst window.
    let chunk = sine_chunk(base, 9000);
    base += 9000;
    m.feed_interleaved(0, &chunk, 2).expect("feed");
    for _ in 0..WINDOW_BLOCKS {
        process(&mut m);
    }
    assert_eq!(
        m.degraded_channels(),
        2,
        "bursty delivery must gate both channels"
    );
    assert!(
        m.channel_meters_json().contains("\"dg\":1"),
        "gated channels must report dg=1 in the meters JSON"
    );
    assert!(
        m.channel_meters_json().contains("\"ov\":1616"),
        "two 9000-frame bursts past the 8192 cap must count 2×808 overflow frames"
    );

    // While gated the channel is silent even though audio keeps arriving
    // paced (fade completes within 512 samples = 4 blocks).
    for _ in 0..8 {
        let chunk = sine_chunk(base, BLOCK as usize);
        base += BLOCK as usize;
        m.feed_interleaved(0, &chunk, 2).expect("feed");
        let peak = process(&mut m);
        assert!(
            peak < 0.02,
            "gated channel must be silent while fed (peak {peak})"
        );
    }

    // And no concealment churn: inserts must not fire while degraded,
    // even under continued starvation.
    let inserts_at_gate = m.elastic_inserts();
    for _ in 0..2 * WINDOW_BLOCKS {
        process(&mut m); // starve — no feed
    }
    assert_eq!(
        m.elastic_inserts(),
        inserts_at_gate,
        "inserts must stop while the health gate holds the channel"
    );
    assert_eq!(m.degraded_channels(), 2, "starvation keeps the gate on");

    // Clean paced delivery for three windows: gate releases, audio
    // fades back in (full gain within 4 blocks of recovery).
    for _ in 0..3 * WINDOW_BLOCKS + 8 {
        let chunk = sine_chunk(base, BLOCK as usize);
        base += BLOCK as usize;
        m.feed_interleaved(0, &chunk, 2).expect("feed");
        process(&mut m);
    }
    assert_eq!(
        m.degraded_channels(),
        0,
        "clean delivery must release the gate"
    );
    let mut peak = 0.0f32;
    for _ in 0..8 {
        let chunk = sine_chunk(base, BLOCK as usize);
        base += BLOCK as usize;
        m.feed_interleaved(0, &chunk, 2).expect("feed");
        peak = peak.max(process(&mut m));
    }
    assert!(
        peak > 0.2,
        "recovered channel must pass audio again (peak {peak})"
    );
    assert!(
        !m.channel_meters_json().contains("\"dg\":1"),
        "recovered channels must report dg=0"
    );
}

/// Paced 1.0x delivery with a standing buffer (the normal QuicSRT
/// delivery shape) must NEVER trip the gate.
#[test]
fn paced_stream_never_gates() {
    let mut m = mixer();
    let mut base = 0usize;
    for _ in 0..2 {
        let chunk = sine_chunk(base, 960);
        base += 960;
        m.feed_interleaved(0, &chunk, 2).expect("feed");
    }
    let mut fed = 960 * 2usize;
    let total = 3 * WINDOW_BLOCKS;
    for block in 0..total {
        while fed < block * BLOCK as usize + 512 {
            let chunk = sine_chunk(base, 960);
            base += 960;
            m.feed_interleaved(0, &chunk, 2).expect("feed");
            fed += 960;
        }
        process(&mut m);
    }
    assert_eq!(m.degraded_channels(), 0, "paced delivery must never gate");
    assert_eq!(m.starved_blocks(), 0);
}

/// Concealment churn: a net-slow stream fed in periodic small bursts
/// makes the elastic buffer insert constantly (the repeat-splice static)
/// while starvation stays under the fast-path threshold. The window
/// finalize check (HEALTH_INSERT_MAX) must still gate it.
#[test]
fn insert_churn_gates_via_window_threshold() {
    let mut m = mixer();
    let mut base = 0usize;

    // Anchor on clean paced delivery (2 windows + preload).
    for _ in 0..2 {
        let chunk = sine_chunk(base, 960);
        base += 960;
        m.feed_interleaved(0, &chunk, 2).expect("feed");
    }
    let mut fed = 960 * 2usize;
    for block in 0..2 * WINDOW_BLOCKS {
        while fed < block * BLOCK as usize + 512 {
            let chunk = sine_chunk(base, 960);
            base += 960;
            m.feed_interleaved(0, &chunk, 2).expect("feed");
            fed += 960;
        }
        process(&mut m);
    }
    assert_eq!(m.degraded_channels(), 0, "paced anchor phase must not gate");

    // Slow concealment churn: a 200 ppm deficit (the drift_test regime)
    // inserts ~0.15×/window — must NEVER gate (that is healthy drift
    // reconciliation, not pathology).
    for block in 2 * WINDOW_BLOCKS..5 * WINDOW_BLOCKS {
        while (fed as f64) < block as f64 * BLOCK as f64 * 0.9998 + 512.0 {
            let chunk = sine_chunk(base, 960);
            base += 960;
            m.feed_interleaved(0, &chunk, 2).expect("feed");
            fed += 960;
        }
        process(&mut m);
    }
    assert_eq!(
        m.degraded_channels(),
        0,
        "200 ppm drift reconciliation must not trip the gate"
    );
    assert!(
        m.elastic_inserts() > 0 || m.starved_blocks() == 0,
        "drift phase should be exercising the elastic path"
    );
}
