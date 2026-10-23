//! Elastic per-channel input FIFOs: queueing plus drift reconciliation.
use std::collections::VecDeque;

/// Per-channel input FIFO memory cap (frames @48 k ≈ 170 ms). The elastic
/// playout logic below keeps live depth far below this; the cap is only a
/// last-resort guard against unbounded growth if that logic ever fails.
/// Oldest frames are dropped on overflow.
pub(crate) const MAX_QUEUE_FRAMES: usize = 8192;

// ── Elastic playout buffer ─────────────────────────────────────────────
//
// Delivery is paced by the QuicSRT worker's `performance.now()` clock while
// process() consumes at the audio hardware clock. The two always differ by
// some ppm, so a fixed-cap FIFO either accumulates drift until it wraps
// into drop-oldest glitching (net-fast delivery) or drains into periodic
// starvation (net-slow). Instead each elastic FIFO tracks its depth's
// smoothed window-minimum, anchors at the level established around stream
// start, and reconciles drift with tiny crossfaded slips:
//
// - trim: drop SLIP_FRAMES from the queue head, crossfading the splice
//   (net-fast delivery → standing latency would otherwise creep up).
//   Fires when the smoothed depth rises above `anchor + PLAYOUT_BAND`.
// - insert: repeat SLIP_FRAMES at the queue tail, crossfaded (net-slow
//   delivery → the queue would otherwise starve). Fires when the smoothed
//   depth falls below half the anchor (relative — chunked delivery
//   establishes anchors of only a few hundred frames, so an absolute
//   lower band could sit below zero and never trigger), or on live
//   starvation. The burst is capped near the trigger — clamped to the
//   absolute RECONCILE_CEIL_FRAMES — and throttled to HEALTH_INSERT_MAX
//   inserts per window, so reconciliation can never outspend the health
//   budget: a deficit too large for the budget starves into the starve
//   counters instead (see the gate notes below).
//
// All channels of a PID mapping are fed and drained in lockstep, so their
// depths, EWMA and cooldowns stay equal and every channel applies the same
// correction at the same frame boundary (inter-channel phase preserved).
// Corrections are rate-limited (one slip per COOLDOWN blocks) and bounded
// by a hysteresis band around the anchor, so a correction never fires for
// ordinary delivery jitter.
//
// The anchor is reset by `map_pid` — a remap onto the same channels is a
// new stream whose natural standing depth may differ.

/// Window length for depth statistics (blocks): 375 × 128 = 1 s @ 48 k.
pub(crate) const DRIFT_WINDOW_BLOCKS: u64 = 375;
/// EWMA smoothing factor applied to the per-window depth minimum.
const EWMA_ALPHA: f32 = 0.25;
/// Upper hysteresis band above the anchor (frames, ≈ 21 ms @ 48 k): the
/// smoothed depth must rise past `anchor + BAND` before trims arm. The
/// lower trigger is relative (`anchor / 2`) — see the notes above.
pub(crate) const PLAYOUT_BAND_FRAMES: f32 = 1024.0;
/// Correction size (frames, ≈ 1.3 ms @ 48 k) — the length of audio a slip
/// drops or repeats.
pub(crate) const SLIP_FRAMES: usize = 64;
/// Crossfade length over which a splice morphs (frames). Must be ≤ SLIP.
pub(crate) const SLIP_XFADE: usize = 32;
/// Blocks between corrections on the same FIFO (rate limit: a slip every
/// ~10.7 ms → max correction rate ≈ 6000 f/s, far above any real drift).
pub(crate) const SLIP_COOLDOWN_BLOCKS: u32 = 4;
/// Absolute ceiling (frames, ≈ 21 ms @ 48 k) on the insert refill target:
/// `min(anchor/2 + 2·SLIP, this)`. The anchor-scaled bound alone made
/// every drift re-arm climb `anchor/4 + 64` frames at cooldown pacing —
/// ~33-insert bursts at live anchor depths (5248-frame anchor), periodic,
/// deficit-independent, indistinguishable from the pathological churn
/// HEALTH_INSERT_MAX exists to catch: a 1% slow clock flickered the gate
/// 8/30 windows (native repro 2026-10-23, live ffmpeg s302m ingest). With
/// the ceiling, a refill can never climb toward a far target — the queue
/// must fall to ≤ 21 ms first — so the steady correction rate is the
/// deficit itself (one insert per 64 frames of shortfall). Together with
/// the per-window insert cap (lib.rs: `window_inserts <
/// HEALTH_INSERT_MAX`) this keeps reconciliation inside the health
/// budget by construction; a deficit too large for the budget outruns
/// the refill and starves into the starve counters instead. Small-anchor
/// streams (chunked delivery bottoms near zero → anchors of a few
/// hundred frames) keep the unclamped relative bound exactly as before.
pub(crate) const RECONCILE_CEIL_FRAMES: usize = 1024;
/// A FIFO is "recently fed" if it saw input within this many windows.
/// Insertions are gated on it (never stretch a stopped/disconnected
/// source); trims are not (pre-draining a stall backlog is harmless).
pub(crate) const FEED_RECENCY_WINDOWS: u64 = 2;
/// Consecutive fed windows required before the anchor is set.
pub(crate) const ANCHOR_INIT_WINDOWS: u32 = 2;

// ── Stream-health gate ─────────────────────────────────────────────────
//
// A stream whose delivery is pathologically gappy (TSBPD latency below
// the network's jitter tail → constant SRT drops; a source bursting far
// over real-time then stalling) cannot be patched over by the elastic
// buffer: every starvation is a hole and every insert a repeat, and at
// high rates the "repair" itself becomes full-scale granular static —
// far worse for ears and speakers than a brief dropout. The same holds
// for the net-fast mirror: a backlog flush pins the queue at the
// drop-oldest cap and the audio is chopped from the head (overflow
// drops + trim slips), with no starvation at all. Such streams are
// faded to silence instead (a console protects the bus; the meters +
// UI surface the state), and fade back in once delivery is clean again.
//
// Degraded = more than HEALTH_STARVE_MAX starved blocks, more than
// HEALTH_INSERT_MAX inserts, more than HEALTH_SLIP_MAX trims, or more
// than HEALTH_OVERFLOW_MAX_FRAMES cap-dropped in one statistics window
// (~1 s) — or the corresponding FAST thresholds hit mid-window (engages
// the fade immediately rather than at the next window boundary).
// Recovery requires HEALTH_RECOVERY_WINDOWS consecutive clean windows.
pub(crate) const HEALTH_STARVE_MAX: u32 = 8;
pub(crate) const HEALTH_INSERT_MAX: u32 = 8;
pub(crate) const HEALTH_SLIP_MAX: u32 = 12;
/// Cap-dropped frames per window that constitute pathology (0.5% of a
/// 48 k window; the drift regime drops none by construction).
pub(crate) const HEALTH_OVERFLOW_MAX_FRAMES: u64 = 256;
/// Mid-window fast-path thresholds. 12 starved blocks ≈ 32 ms of holes;
/// 12 trims ≈ 1.6% of a window chopped; 1024 cap-dropped frames ≈ 21 ms
/// skipped outright. (Inserts have no fast path — any deficit large
/// enough to burst inserts drains the queue to the starve gate first.)
pub(crate) const HEALTH_FAST_STARVE_BLOCKS: u32 = 12;
pub(crate) const HEALTH_FAST_SLIPS: u32 = 12;
pub(crate) const HEALTH_FAST_OVERFLOW_FRAMES: u64 = 1024;
pub(crate) const HEALTH_RECOVERY_WINDOWS: u32 = 2;
/// Per-sample fade rates (asymmetric): OUT fast to protect the output
/// (≈5.3 ms full swing @ 48 k), IN slower to avoid pumping (≈10.7 ms).
pub(crate) const HEALTH_FADE_OUT_STEP: f32 = 1.0 / 256.0;
pub(crate) const HEALTH_FADE_IN_STEP: f32 = 1.0 / 512.0;

/// Per-channel elastic input FIFO with drift bookkeeping.
pub(crate) struct ChannelFifo {
    pub(crate) q: VecDeque<f32>,
    /// Per-block direct input (test tones), summed with `q` at drain.
    /// A separate overlay queue — NOT part of the elastic/health path:
    /// `set_channel_input` shares the context clock with the drain (no
    /// drift to reconcile), and letting it write `q` double-feeds the
    /// drift-managed stream FIFO when a mapped PID lands on the same
    /// strips (the publish→subscribe loopback's exact setup): 2× real
    /// time in against a 1× drain pins `q` at the cap and trips the
    /// stream-health gate permanently. Direct input never arms
    /// elasticity (no `note_feed`) and never counts toward the health
    /// statistics.
    pub(crate) direct_q: VecDeque<f32>,
    /// Elastic corrections apply (fed via the interleaved PCM path).
    pub(crate) elastic: bool,
    /// Current drift-statistics window id (`block_counter / WINDOW`).
    pub(crate) window_id: u64,
    /// Minimum depth observed in the current window.
    pub(crate) window_min: usize,
    /// Any block starved (queue empty mid-drain) in the current window.
    pub(crate) window_starved: bool,
    /// Starved blocks in the current window (stream-health statistic).
    pub(crate) window_starved_blocks: u32,
    /// Inserts applied in the current window (stream-health statistic).
    pub(crate) window_inserts: u32,
    /// Trim slips applied in the current window (stream-health statistic).
    pub(crate) window_slips: u32,
    /// Cap-dropped frames in the current window (stream-health statistic).
    pub(crate) window_overflow_frames: u64,
    /// Smoothed window-min depth; negative = not yet initialized.
    pub(crate) ewma: f32,
    /// Consecutive windows with feed activity (anchor initialization).
    pub(crate) fed_windows: u32,
    /// Established standing depth; `None` until anchored.
    pub(crate) anchor: Option<f32>,
    /// Window id of the last feed.
    pub(crate) last_fed_window: u64,
    /// Blocks remaining before the next correction may fire.
    pub(crate) cooldown: u32,
    /// Stream-health gate active (fade this channel to silence).
    pub(crate) degraded: bool,
    /// Consecutive clean windows while degraded (recovery count).
    pub(crate) clean_windows: u32,
    /// Live fade gain toward 0 (degraded) or 1 (healthy), per sample.
    pub(crate) health_gain: f32,
    /// Diagnostics: total trim / insert corrections applied.
    pub(crate) slips: u64,
    pub(crate) inserts: u64,
    /// Diagnostics: frames dropped by the MAX_QUEUE_FRAMES overflow guard.
    pub(crate) overflow_drops: u64,
}

impl ChannelFifo {
    pub(crate) fn new() -> Self {
        Self {
            q: VecDeque::with_capacity(2048),
            direct_q: VecDeque::with_capacity(256),
            elastic: false,
            window_id: 0,
            window_min: usize::MAX,
            window_starved: false,
            window_starved_blocks: 0,
            window_inserts: 0,
            window_slips: 0,
            window_overflow_frames: 0,
            ewma: -1.0,
            fed_windows: 0,
            anchor: None,
            last_fed_window: 0,
            cooldown: 0,
            degraded: false,
            clean_windows: 0,
            health_gain: 1.0,
            slips: 0,
            inserts: 0,
            overflow_drops: 0,
        }
    }

    /// Mark fed in `window` and arm elasticity (idempotent).
    pub(crate) fn note_feed(&mut self, window: u64) {
        self.elastic = true;
        self.last_fed_window = window;
    }

    /// Close the statistics window that just ended: fold `window_min`
    /// into the EWMA, initialize the anchor once the feed is stable, and
    /// apply the stream-health decision (see the gate notes above).
    pub(crate) fn finalize_window(&mut self) {
        let m = self.window_min;
        // A window that never sampled a depth (the FIFO was created
        // mid-window by a stream connecting while the engine was already
        // running — every live session) leaves `window_min` at the
        // usize::MAX sentinel: folding it seeded the EWMA with ~4.3e9,
        // the anchor formed from that garbage, and the insert arm
        // (`ewma < anchor_half`) then fired for the ~40 windows it took
        // the giant value to decay — insert bursts, gate flicker and
        // overflow creep for the first minute of every live stream.
        // Skip the fold; the first sampled window seeds the EWMA.
        if m != usize::MAX {
            if self.ewma < 0.0 {
                self.ewma = m as f32;
            } else {
                self.ewma += EWMA_ALPHA * (m as f32 - self.ewma);
            }
        }
        if self.window_starved_blocks > HEALTH_STARVE_MAX
            || self.window_inserts > HEALTH_INSERT_MAX
            || self.window_slips > HEALTH_SLIP_MAX
            || self.window_overflow_frames > HEALTH_OVERFLOW_MAX_FRAMES
        {
            self.degraded = true;
            self.clean_windows = 0;
        } else if self.degraded {
            self.clean_windows += 1;
            if self.clean_windows >= HEALTH_RECOVERY_WINDOWS {
                self.degraded = false;
                // Recovery = delivery judged clean again: re-base the
                // anchor on the current smoothed depth. The anchor is a
                // snapshot of the stream's early shape (often its subscribe
                // transient); without this re-base the insert arm
                // (`ewma < anchor_half`) re-fires every window against the
                // stale snapshot, stretching +64 frames at cooldown rate,
                // tripping HEALTH_INSERT_MAX, and re-gating a healthy
                // stream. True rate-deficit streams never gate (a 200 ppm
                // deficit is ~0.2 inserts/window), so their drift
                // reconciliation is untouched; genuinely pathological
                // streams re-gate on their own counters.
                self.anchor = Some(self.ewma);
            }
        }
        self.window_min = usize::MAX;
        self.window_starved = false;
        self.window_starved_blocks = 0;
        self.window_inserts = 0;
        self.window_slips = 0;
        self.window_overflow_frames = 0;
    }

    /// Reset the stream-health gate (a remap is a new stream).
    pub(crate) fn reset_health(&mut self) {
        self.degraded = false;
        self.clean_windows = 0;
        self.health_gain = 1.0;
        self.window_starved_blocks = 0;
        self.window_inserts = 0;
        self.window_slips = 0;
        self.window_overflow_frames = 0;
    }
}

/// Drop `SLIP_FRAMES` from the queue head, crossfading the boundary
/// between the dropped tail and the kept head over `SLIP_XFADE` frames.
/// No-op when the queue is too short to splice safely.
pub(crate) fn slip_trim(q: &mut VecDeque<f32>) {
    let d = SLIP_FRAMES;
    let f = SLIP_XFADE;
    if q.len() <= d + f {
        return;
    }
    for i in 0..f {
        let kept = q[d + i];
        let dropped = q[d - f + i];
        let w = (i + 1) as f32 / (f + 1) as f32;
        q[d + i] = dropped * (1.0 - w) + kept * w;
    }
    q.drain(0..d);
}

/// Append `SLIP_FRAMES` at the queue tail by repeating the tail with a
/// crossfaded splice (time-stretch: net depth grows by SLIP_FRAMES).
/// No-op when the queue is too short to splice safely.
pub(crate) fn slip_insert(q: &mut VecDeque<f32>) {
    let d = SLIP_FRAMES;
    let f = SLIP_XFADE;
    let m = q.len();
    if m < d + f {
        return;
    }
    // Snapshot the pre-splice tail: the morph overwrites samples the
    // appended segment must read in their original form.
    let mut tail = [0.0f32; SLIP_FRAMES + SLIP_XFADE];
    for (i, t) in tail.iter_mut().enumerate() {
        *t = q[m - d - f + i];
    }
    // Morph the last f samples from the original timeline into the
    // delayed-by-d timeline (position p plays original[p - d]).
    for i in 0..f {
        let orig = tail[d + i];
        let delayed = tail[i];
        let w = (i + 1) as f32 / (f + 1) as f32;
        let idx = m - f + i;
        q[idx] = orig * (1.0 - w) + delayed * w;
    }
    // The delayed timeline now continues with original[m - d .. m].
    for j in 0..d {
        q.push_back(tail[f + j]);
    }
}
