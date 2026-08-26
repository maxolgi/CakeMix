//! Elastic per-channel input FIFOs: queueing plus drift reconciliation.
use std::collections::VecDeque;

/// Per-channel input FIFO memory cap (frames @48 k ≈ 170 ms). The elastic
/// playout logic below keeps live depth far below this; the cap is only a
/// last-resort guard against unbounded growth if that logic ever fails.
/// Oldest frames are dropped on overflow.
pub(crate) const MAX_QUEUE_FRAMES: usize = 8192;

// ── Elastic playout buffer ─────────────────────────────────────────────
//
// Delivery is paced by the WebSRT worker's `performance.now()` clock while
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
//   starvation. The burst is capped near the trigger so a between-windows
//   stale statistic cannot overshoot.
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
/// A FIFO is "recently fed" if it saw input within this many windows.
/// Insertions are gated on it (never stretch a stopped/disconnected
/// source); trims are not (pre-draining a stall backlog is harmless).
pub(crate) const FEED_RECENCY_WINDOWS: u64 = 2;
/// Consecutive fed windows required before the anchor is set.
pub(crate) const ANCHOR_INIT_WINDOWS: u32 = 2;

/// Per-channel elastic input FIFO with drift bookkeeping.
pub(crate) struct ChannelFifo {
    pub(crate) q: VecDeque<f32>,
    /// Elastic corrections apply (fed via the interleaved PCM path).
    pub(crate) elastic: bool,
    /// Current drift-statistics window id (`block_counter / WINDOW`).
    pub(crate) window_id: u64,
    /// Minimum depth observed in the current window.
    pub(crate) window_min: usize,
    /// Any block starved (queue empty mid-drain) in the current window.
    pub(crate) window_starved: bool,
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
    /// Diagnostics: total trim / insert corrections applied.
    pub(crate) slips: u64,
    pub(crate) inserts: u64,
}

impl ChannelFifo {
    pub(crate) fn new() -> Self {
        Self {
            q: VecDeque::with_capacity(2048),
            elastic: false,
            window_id: 0,
            window_min: usize::MAX,
            window_starved: false,
            ewma: -1.0,
            fed_windows: 0,
            anchor: None,
            last_fed_window: 0,
            cooldown: 0,
            slips: 0,
            inserts: 0,
        }
    }

    /// Mark fed in `window` and arm elasticity (idempotent).
    pub(crate) fn note_feed(&mut self, window: u64) {
        self.elastic = true;
        self.last_fed_window = window;
    }

    /// Close the statistics window that just ended: fold `window_min`
    /// into the EWMA and initialize the anchor once the feed is stable.
    pub(crate) fn finalize_window(&mut self) {
        let m = self.window_min;
        if self.ewma < 0.0 {
            self.ewma = m as f32;
        } else {
            self.ewma += EWMA_ALPHA * (m as f32 - self.ewma);
        }
        self.window_min = usize::MAX;
        self.window_starved = false;
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
