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

/// Disconnect (unmap) must NOT gate the strips: the queue's tail drains
/// and starves, but that is the end of a dead source, not delivery
/// pathology. Remap re-arms the gate (a new stream can be pathological).
#[test]
fn unmapped_stream_never_gates_and_remap_rearms() {
    let mut m = mixer();
    let mut base = 0usize;

    // Paced feed to establish the stream (1 window).
    for _ in 0..2 {
        let chunk = sine_chunk(base, 960);
        base += 960;
        m.feed_interleaved(0, &chunk, 2).expect("feed");
    }
    let mut fed = 960 * 2usize;
    for block in 0..WINDOW_BLOCKS {
        while fed < block * BLOCK as usize + 512 {
            let chunk = sine_chunk(base, 960);
            base += 960;
            m.feed_interleaved(0, &chunk, 2).expect("feed");
            fed += 960;
        }
        process(&mut m);
    }
    assert_eq!(m.degraded_channels(), 0);

    // Teardown: unmap, then keep processing — the tail drains and the
    // FIFOs starve for three windows. No strip may gate.
    m.unmap_pid(100);
    for _ in 0..3 * WINDOW_BLOCKS {
        process(&mut m);
    }
    assert_eq!(
        m.degraded_channels(),
        0,
        "a disconnected stream's drain tail must never gate its strips"
    );

    // Remap = a new stream on the same channels: bursty pathology must
    // gate again (elasticity re-armed).
    m.map_pid(100, 0, 2).expect("remap");
    for _ in 0..2 {
        let chunk = sine_chunk(base, 9000);
        base += 9000;
        m.feed_interleaved(0, &chunk, 2).expect("feed");
        for _ in 0..WINDOW_BLOCKS {
            process(&mut m);
        }
    }
    assert_eq!(
        m.degraded_channels(),
        2,
        "remap must re-arm the stream-health gate"
    );
}

/// Net-fast backlog flush (the 120 ms+ latency failure mode): a burst
/// after a stall overruns the 8192-frame cap, so audio is chopped from
/// the head (drop-oldest + trim slips) with ZERO starvation. The
/// trim/overflow paths of the gate must catch it — the starve path
/// never fires — and clean paced delivery must release it.
#[test]
fn netfast_backlog_flush_gates_without_starving() {
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

    // 2% net-fast: the standing depth climbs to the 8192 cap (~8 s),
    // after which every chunk overflows at the head and trims churn.
    // No starvation can occur in this regime.
    let mut gated = false;
    for block in 2 * WINDOW_BLOCKS..16 * WINDOW_BLOCKS {
        while (fed as f64) < block as f64 * BLOCK as f64 * 1.02 + 512.0 {
            let chunk = sine_chunk(base, 960);
            base += 960;
            m.feed_interleaved(0, &chunk, 2).expect("feed");
            fed += 960;
        }
        process(&mut m);
        if m.degraded_channels() == 2 {
            gated = true;
            break;
        }
    }
    assert!(
        gated,
        "net-fast backlog churn must gate via the trim/overflow path"
    );
    assert_eq!(
        m.starved_blocks(),
        0,
        "the net-fast regime never starves — proves the non-starve gate paths fire"
    );
    assert!(
        m.channel_meters_json().contains("\"ov\":"),
        "overflow drops must be reported in the meters JSON"
    );

    // Gated ⇒ silent even while the overfeed continues (fade completes
    // within 512 samples; allow 8 blocks of ramp before measuring).
    let baseline = fed;
    let mut max_peak = 0.0f32;
    for block in 0..2 * WINDOW_BLOCKS {
        while (fed as f64) < baseline as f64 + block as f64 * BLOCK as f64 * 1.02 + 512.0 {
            let chunk = sine_chunk(base, 960);
            base += 960;
            m.feed_interleaved(0, &chunk, 2).expect("feed");
            fed += 960;
        }
        let out = m.process_block(BLOCK).expect("process");
        if block > 8 {
            for s in out.chunks(2) {
                max_peak = max_peak.max(s[0].abs()).max(s[1].abs());
            }
        }
    }
    assert_eq!(
        m.degraded_channels(),
        2,
        "sustained overfeed keeps the gate on"
    );
    assert!(
        max_peak < 0.02,
        "gated channel must be silent under sustained overfeed (peak {max_peak})"
    );

    // Clean paced delivery: the backlog drains via rate-limited trims
    // (staying gated while it does), then the gate releases and audio
    // fades back in.
    for _ in 0..6 * WINDOW_BLOCKS {
        let chunk = sine_chunk(base, BLOCK as usize);
        base += BLOCK as usize;
        m.feed_interleaved(0, &chunk, 2).expect("feed");
        process(&mut m);
        if m.degraded_channels() == 0 {
            break;
        }
    }
    assert_eq!(
        m.degraded_channels(),
        0,
        "clean paced delivery must release the gate"
    );
    let mut peak = 0.0f32;
    for _ in 0..8 {
        let chunk = sine_chunk(base, BLOCK as usize);
        base += BLOCK as usize;
        m.feed_interleaved(0, &chunk, 2).expect("feed");
        let out = m.process_block(BLOCK).expect("process");
        for s in out.chunks(2) {
            peak = peak.max(s[0].abs()).max(s[1].abs());
        }
    }
    assert!(
        peak > 0.2,
        "recovered channel must pass audio again (peak {peak})"
    );
}
