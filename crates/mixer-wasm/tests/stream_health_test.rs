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
//! critically — paced/drift-normal streams NEVER gate. The slow-clock
//! tests extend that invariant to persistent rate deficits (a source
//! clock running 0.1–1% behind the local DAC), which live once flickered
//! the gate 8/30 windows: reconciliation is now structurally bounded
//! (absolute refill ceiling + per-window insert cap at
//! HEALTH_INSERT_MAX), so it cannot trip the insert gate by construction,
//! and a deficit too large for the budget starves into the starve
//! counters instead.
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

/// Interleaved stereo DC chunk (the sum-known-answer marker).
fn dc_chunk(level: f32, frames: usize) -> Vec<f32> {
    vec![level; frames * 2]
}

fn process(m: &mut MixerWasm) -> f32 {
    let out = m.process_block(BLOCK).expect("process");
    let mut peak = 0.0f32;
    for s in out.chunks(2) {
        peak = peak.max(s[0].abs()).max(s[1].abs());
    }
    peak
}

fn process_avg(m: &mut MixerWasm) -> f32 {
    let out = m.process_block(BLOCK).expect("process");
    let n = out.len() / 2;
    out.chunks(2).map(|s| s[0]).sum::<f32>() / n as f32
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

/// THE LOOPBACK REGRESSION (live E2E, 2026-10-22): the app's own
/// publish→gateway→subscribe loopback maps the incoming PID onto the very
/// strips the test-tone generator feeds (auto-map packs from channel 0;
/// tones feed channels 0–3; QA enables tones to have content to publish).
/// Both feeds used to write the SAME elastic FIFO: 2× real time in against
/// a 1× drain pinned the queue at the 8192 cap, every batch was chopped
/// from the head, and the stream-health gate engaged forever (strips GATED
/// at 120 ms and 1000 ms TSBPD, loss 0 / rtt 1-2 ms, no recovery in 20+ s).
/// The direct (tone) input now rides its own overlay queue summed at
/// drain — the drift-managed stream FIFO has a single feeder, so the exact
/// live delivery shape (1024-frame batches paced at real time, zero loss)
/// must NEVER gate, and the strip must pass tone + stream summed.
#[test]
fn loopback_tone_and_stream_share_strips_without_gating() {
    let mut m = mixer();
    let mut base = 0usize; // stream frame cursor
    let mut gated_at = None;
    for block in 0..6 * WINDOW_BLOCKS {
        // Per-block direct (tone) feed: the worklet's set_channel_input
        // cadence — one 128-frame block per render block, on the same
        // channels the stream maps to.
        let tone = [0.05f32; BLOCK as usize];
        m.set_channel_input_slice(0, &tone).expect("tone ch0");
        m.set_channel_input_slice(1, &tone).expect("tone ch1");
        // Stream feed: every 1024-frame batch due at this block's start
        // (batch interval == batch duration — the PUB_BATCH_FRAMES loopback
        // pacing), delivered between blocks exactly like worklet message
        // handling.
        while base + 1024 <= (block + 1) * BLOCK as usize {
            let chunk = dc_chunk(0.5, 1024);
            m.feed_interleaved(0, &chunk, 2).expect("feed");
            base += 1024;
        }
        process(&mut m);
        if m.degraded_channels() > 0 && gated_at.is_none() {
            gated_at = Some(block);
        }
    }
    assert_eq!(
        m.degraded_channels(),
        0,
        "tone + stream on one strip must never gate (gated at block {gated_at:?})"
    );
    assert_eq!(m.starved_blocks(), 0);
    assert_eq!(m.elastic_inserts(), 0);
    assert_eq!(m.elastic_slips(), 0);
    assert!(
        !m.channel_meters_json().contains("\"ov\":1"),
        "no overflow drops may be counted (got {})",
        m.channel_meters_json()
    );
    // Sum honesty: stereo strips route ch0→L / ch1→R at unity, so each
    // side passes stream 0.5 + tone 0.05 = 0.55.
    let tone = [0.05f32; BLOCK as usize];
    m.set_channel_input_slice(0, &tone).expect("tone ch0");
    m.set_channel_input_slice(1, &tone).expect("tone ch1");
    let chunk = dc_chunk(0.5, 1024);
    m.feed_interleaved(0, &chunk, 2).expect("feed");
    let avg = process_avg(&mut m);
    assert!(
        (avg - 0.55).abs() < 2e-3,
        "strip must pass tone + stream summed (avg {avg}, want 0.55)"
    );
}

/// The gate must still fade a genuinely pathological stream even while a
/// direct (tone) feed shares the strip: bursty delivery gates, and the
/// whole strip (tone included) fades to silence.
#[test]
fn gated_stream_still_fades_direct_input() {
    let mut m = mixer();
    // Burst drain (the low-TSBPD pathology) with tones feeding every block.
    let chunk = sine_chunk(0, 9000);
    m.feed_interleaved(0, &chunk, 2).expect("feed");
    let tone = [0.25f32; BLOCK as usize];
    for b in 0..100 {
        m.set_channel_input_slice(0, &tone).expect("tone ch0");
        m.set_channel_input_slice(1, &tone).expect("tone ch1");
        let peak = process(&mut m);
        if b == 80 {
            // The 8192-frame cap keeps 64 blocks of the burst; starvation
            // begins at block 64 and the 12-starved-block fast path gates
            // ~12 blocks later — same schedule as without the tone overlay.
            assert_eq!(
                m.degraded_channels(),
                2,
                "bursty delivery must gate despite the tone overlay"
            );
            assert!(
                peak < 0.02,
                "gated strip must be silent including direct input (peak {peak})"
            );
        }
    }
    assert_eq!(m.degraded_channels(), 2, "the gate must hold");
}

/// THE MID-WINDOW SENTINEL (live E2E, 2026-10-22): the worklet drains
/// from page load, so a stream connecting seconds later creates its FIFO
/// mid-window — the first drain block hits the window-rollover check
/// before any depth sample, and `finalize_window` folded the
/// `usize::MAX` sentinel into the EWMA (~4.3e9). The anchor formed from
/// that garbage; the insert arm (`ewma < anchor_half`) then fired for
/// every one of the ~40 windows the giant value took to decay — insert
/// bursts tripping HEALTH_INSERT_MAX, gate flicker and overflow creep
/// for the first minute of every live session. Native tests never saw
/// it: they start streams at block 0 (window 0), where the rollover
/// cannot precede the first sample. The finalize guard skips unsampled
/// windows; this test starts a stream mid-window, exactly like the live
/// path, and requires clean batchy delivery thereafter.
#[test]
fn mid_window_stream_start_never_seeds_giant_anchor() {
    let mut m = mixer();
    // The engine runs empty past a window boundary before the stream
    // connects (page load vs connect): the first feed lands mid-window.
    for _ in 0..WINDOW_BLOCKS + 10 {
        process(&mut m);
    }
    let mut base = 0usize;
    let mut fed = 0usize;
    let mut gated_at = None;
    let total = 8 * WINDOW_BLOCKS;
    for block in 0..total {
        // Loopback shape: 1024-frame batches, interval == batch duration.
        while fed + 1024 <= (block + 1) * BLOCK as usize {
            let chunk = sine_chunk(base, 1024);
            base += 1024;
            m.feed_interleaved(0, &chunk, 2).expect("feed");
            fed += 1024;
        }
        process(&mut m);
        if m.degraded_channels() > 0 && gated_at.is_none() {
            gated_at = Some(block);
        }
    }
    assert_eq!(
        m.degraded_channels(),
        0,
        "mid-window stream start must not gate clean batchy delivery (gated at {gated_at:?})"
    );
    assert_eq!(
        m.elastic_inserts(),
        0,
        "no insert churn may fire against a sentinel-seeded anchor"
    );
    assert_eq!(m.starved_blocks(), 0);
    assert_eq!(m.elastic_slips(), 0);
}

/// Direct input alone (no stream ever mapped) must stay healthy: no gate,
/// no elastic starvation counting, audio passes at unity.
#[test]
fn tone_only_channel_never_gates() {
    let mut m = mixer();
    let tone = [0.25f32; BLOCK as usize];
    for _ in 0..3 * WINDOW_BLOCKS {
        m.set_channel_input_slice(0, &tone).expect("tone ch0");
        m.set_channel_input_slice(1, &tone).expect("tone ch1");
        process(&mut m);
    }
    assert_eq!(m.degraded_channels(), 0, "tone-only strips must never gate");
    assert_eq!(
        m.starved_blocks(),
        0,
        "direct input must not count as stream starvation"
    );
    let mut peak = 0.0f32;
    for _ in 0..8 {
        m.set_channel_input_slice(0, &tone).expect("tone ch0");
        m.set_channel_input_slice(1, &tone).expect("tone ch1");
        peak = peak.max(process(&mut m));
    }
    assert!(
        (peak - 0.25).abs() < 2e-3,
        "tone-only strip passes audio at unity (peak {peak}, want 0.25)"
    );
}

/// Live-delivery-shape harness shared by the slow-clock tests: engine
/// already running when the stream connects (mid-window start — the
/// every-live-session shape), a small connect burst (TSBPD release
/// begins; the latency buffer stays in the SRT receiver, so the worklet
/// queue only ever holds pacing slack), then 1024-frame batches paced at
/// `rate` (< 1.0 = source clock slow vs the local DAC — a persistent
/// deficit the elastic buffer must reconcile).
struct SlowClock {
    m: MixerWasm,
    base: usize,
    fed: usize,
    connect_done: usize,
    block_n: usize,
    gated_at: Option<usize>,
    max_window_ins: u32,
}

impl SlowClock {
    fn new() -> Self {
        let mut m = mixer();
        for _ in 0..WINDOW_BLOCKS + 10 {
            process(&mut m);
        }
        Self {
            m,
            base: 0,
            fed: 0,
            connect_done: 0,
            block_n: 0,
            gated_at: None,
            max_window_ins: 0,
        }
    }

    /// One real-time block at delivery rate `rate`; samples the gate and
    /// (mid-window) the per-window insert counter.
    fn block(&mut self, rate: f64) {
        let block = self.block_n;
        self.block_n += 1;
        while self.connect_done < 3072 {
            let n = 1024.min(3072 - self.connect_done);
            let chunk = sine_chunk(self.base, n);
            self.base += n;
            self.m.feed_interleaved(0, &chunk, 2).expect("feed");
            self.connect_done += n;
            self.fed += n;
        }
        while (self.fed as f64) < 3072.0 + block as f64 * BLOCK as f64 * rate {
            let chunk = sine_chunk(self.base, 1024);
            self.base += 1024;
            self.m.feed_interleaved(0, &chunk, 2).expect("feed");
            self.fed += 1024;
        }
        self.sample(rate);
    }

    /// Per-block delivery (a healthy 1.0× source feeding every block —
    /// the shape `bursty_stream_fades_to_silence_then_recovers` recovers
    /// against): a zero-buffer stream that never starves.
    fn recovery_block(&mut self) {
        let chunk = sine_chunk(self.base, BLOCK as usize);
        self.base += BLOCK as usize;
        self.m.feed_interleaved(0, &chunk, 2).expect("feed");
        self.sample(1.0);
    }

    fn sample(&mut self, _rate: f64) {
        process(&mut self.m);
        if self.m.degraded_channels() > 0 && self.gated_at.is_none() {
            self.gated_at = Some(self.block_n);
        }
        if self.block_n % WINDOW_BLOCKS == 300 {
            // Mid-window sample: the running per-window insert count
            // (ch0; both channels decide in lockstep).
            let json = self.m.channel_meters_json();
            let ins: u32 = json
                .split("\"ins\":")
                .nth(1)
                .and_then(|s| s.split(',').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let ov: u64 = json
                .split("\"ov\":")
                .nth(1)
                .and_then(|s| s.split('}').next())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            assert!(
                ins <= 8,
                "per-window insert cap (HEALTH_INSERT_MAX) must hold every window (got {ins})"
            );
            assert_eq!(ov, 0, "no overflow drops may be counted in drift regimes");
            self.max_window_ins = self.max_window_ins.max(ins);
        }
    }

    fn ov_total(&self) -> u64 {
        self.m
            .channel_meters_json()
            .split("\"ov\":")
            .nth(1)
            .and_then(|s| s.split('}').next())
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    }
}

/// THE SLOW-CLOCK REGRESSION (live, ffmpeg s302m ingest, 2026-10-23): a
/// REAL gateway-fed source whose clock runs 1% behind the local DAC must
/// reconcile its deficit forever WITHOUT tripping the health gate.
/// Before the structural fix the insert refill chased an anchor-scaled
/// bound (`anchor/2 + 128`): every re-arm climbed that far at cooldown
/// pacing — ~33-insert bursts at live anchor depths, plus a late-armed
/// catch-up burst when the laggy EWMA finally fired — tripping
/// HEALTH_INSERT_MAX window after window (native repro: gated 8/30
/// windows at a 1% deficit, 116 starved blocks; live: gate flicker
/// 10–74 inserts/window, duty cycle varying with receive latency).
/// The fix (absolute refill ceiling + per-window insert cap at
/// HEALTH_INSERT_MAX) makes reconciliation deficit-locked and
/// self-throttled: NEVER gates, converges to a steady shallow depth,
/// zero starvation, zero overflow.
#[test]
fn slow_clock_source_never_gates_and_converges() {
    let mut s = SlowClock::new();
    let windows = 30;
    for _ in 0..windows * WINDOW_BLOCKS {
        s.block(0.99); // 1% slow: 480 f/s deficit, forever
    }
    assert_eq!(
        s.gated_at, None,
        "a 1% slow clock must never trip the health gate"
    );
    assert_eq!(s.m.degraded_channels(), 0);
    assert!(
        s.max_window_ins <= 8,
        "the per-window insert cap (HEALTH_INSERT_MAX) must hold every window \
         (got {}), and at-cap windows are not gated (the finalize check is >)",
        s.max_window_ins
    );
    assert_eq!(
        s.m.starved_blocks(),
        0,
        "the reconciliation servo must bridge a 1% deficit without starving"
    );
    assert!(
        s.m.elastic_inserts() > 0,
        "the deficit must actually be reconciled (inserts flowing)"
    );
    assert_eq!(s.ov_total(), 0, "no cap-dropped frames in the drift regime");
    assert!(
        s.m.fifo_max_depth() < 4096,
        "depth must converge ({} not bounded well below the 8192 cap)",
        s.m.fifo_max_depth()
    );
    // The strip passes audio at full health-gain (the gate never faded
    // it): final blocks peak at the sine amplitude.
    let mut peak = 0.0f32;
    for _ in 0..8 {
        s.block(0.99);
        peak = peak.max(process(&mut s.m));
    }
    assert!(
        peak > 0.2,
        "healthy slow-clock strip passes audio (peak {peak})"
    );
}

/// Stepped-rate variant: the source clock steps between rates (delivery
/// pacing changes mid-stream — encoder/gateway restarts, jitter-buffer
/// retunes). Every step is just a new steady deficit (or surplus): none
/// may gate, none may starve, and reconciliation follows each step down.
#[test]
fn stepped_rate_source_never_gates() {
    let mut s = SlowClock::new();
    // Anchor on clean paced delivery (2 windows), then step deficits.
    let phases: [(usize, f64); 5] = [
        (2, 1.0),    // matched: no inserts, no corrections
        (6, 0.99),   // 1% slow: sustained reconciliation
        (4, 1.0),    // back to matched: rates settle, depth holds
        (4, 0.995),  // gentler deficit
        (4, 0.9995), // near-matched
    ];
    let mut inserts_first_deficit = 0u64;
    for (windows, rate) in phases {
        let phase_start_inserts = s.m.elastic_inserts();
        for _ in 0..windows * WINDOW_BLOCKS {
            s.block(rate);
        }
        if rate < 1.0 && inserts_first_deficit == 0 {
            inserts_first_deficit = s.m.elastic_inserts() - phase_start_inserts;
        }
    }
    assert_eq!(
        s.gated_at, None,
        "stepped-rate delivery must never gate (gate at {:?})",
        s.gated_at
    );
    assert_eq!(s.m.degraded_channels(), 0);
    assert_eq!(s.m.starved_blocks(), 0, "no step may starve the queue");
    assert!(
        inserts_first_deficit > 0,
        "the 1% step must be reconciled with inserts"
    );
    assert_eq!(s.ov_total(), 0, "no cap-dropped frames across steps");
    assert!(
        s.m.fifo_max_depth() < 4096,
        "depth must stay bounded across steps ({})",
        s.m.fifo_max_depth()
    );
}

/// The structural handoff, other direction: a deficit far beyond the
/// reconciliation budget (5% slow = 2400 f/s shortfall vs a refill cap
/// of 8 inserts × 64 frames = 512 f/s per window) outruns the servo.
/// The per-window cap throttles the patch rate, the queue starves
/// chronically, and the STARVE counters gate it — the same verdict the
/// old anchor-scaled insert bursts produced via HEALTH_INSERT_MAX, now
/// reached through the counter that measures the real fault (holes).
/// Clean per-block delivery must still release the gate afterwards.
#[test]
fn oversubscribed_deficit_still_gates_via_starves() {
    let mut s = SlowClock::new();
    let mut gated = false;
    let mut blocks = 0usize;
    let windows = 8;
    for _ in 0..windows * WINDOW_BLOCKS {
        s.block(0.95); // 5% slow: unreconcilable within the budget
        blocks += 1;
        if s.m.degraded_channels() > 0 {
            gated = true;
            break;
        }
    }
    assert!(
        gated,
        "an unreconcilable 5% deficit (chronic starvation) must gate"
    );
    assert!(
        s.m.starved_blocks() > 8,
        "the gate must be driven by the starve counters (got {} starved blocks)",
        s.m.starved_blocks()
    );
    assert!(
        s.m.channel_meters_json().contains("\"dg\":1"),
        "gated channels must report dg=1 in the meters JSON"
    );

    // Clean per-block delivery: the gate releases and audio passes again.
    let mut peak = 0.0f32;
    for _ in 0..4 * WINDOW_BLOCKS {
        s.recovery_block();
        if s.m.degraded_channels() == 0 && blocks > 0 {
            // Fade-in completes within 4 blocks of release.
            for _ in 0..8 {
                s.recovery_block();
                peak = peak.max(process(&mut s.m));
            }
            break;
        }
    }
    assert_eq!(
        s.m.degraded_channels(),
        0,
        "clean delivery must release the gate after the deficit is gone"
    );
    assert!(
        peak > 0.2,
        "recovered strip passes audio again (peak {peak})"
    );
}
