//! Inline stereo master metering (peak/RMS/clip) for the mix bus.

/// Inline stereo master metering (peak hold + decay, sliding-window RMS,
/// sticky clip flag) replicating the oximedia `Meter` Fast ballistics the
/// binding used previously — with pre-allocated ring buffers so the
/// real-time path never allocates (`Meter::process` built four `Vec`s per
/// block).
pub(crate) struct MasterMeter {
    peak: [f32; 2],
    peak_hold_blocks: [u32; 2],
    hold_blocks: u32,
    rms_buf: Vec<f32>,
    rms_window: usize,
    rms_pos: usize,
    rms_filled: usize,
    rms_sq: [f64; 2],
    pub(crate) clipped: [bool; 2],
    processed_once: bool,
}

impl MasterMeter {
    pub(crate) fn new(sample_rate: u32, block_size: usize) -> Self {
        let hold_blocks = ((sample_rate as f32 / block_size.max(1) as f32).ceil() as u32).max(1);
        let rms_window = (sample_rate as usize / 10).max(1); // Fast = 100 ms
        Self {
            peak: [0.0; 2],
            peak_hold_blocks: [0; 2],
            hold_blocks,
            rms_buf: vec![0.0; rms_window * 2],
            rms_window,
            rms_pos: 0,
            rms_filled: 0,
            rms_sq: [0.0; 2],
            clipped: [false; 2],
            processed_once: false,
        }
    }

    /// Process one block of deinterleaved stereo (`l`/`r`, equal length).
    pub(crate) fn process(&mut self, l: &[f32], r: &[f32]) {
        let n = l.len().min(r.len());
        self.processed_once = true;
        for ch in 0..2 {
            let buf = if ch == 0 { l } else { r };
            let block_peak = buf[..n].iter().map(|s| s.abs()).fold(0.0f32, f32::max);
            if block_peak >= 1.0 {
                self.clipped[ch] = true;
            }
            if block_peak > self.peak[ch] {
                self.peak[ch] = block_peak;
                self.peak_hold_blocks[ch] = self.hold_blocks;
            } else if self.peak_hold_blocks[ch] > 0 {
                self.peak_hold_blocks[ch] -= 1;
            } else {
                self.peak[ch] *= 0.95;
            }
        }
        // Sliding-window RMS per channel over an interleaved ring.
        for i in 0..n {
            for ch in 0..2 {
                let s = if ch == 0 { l[i] } else { r[i] };
                let ring_ch = self.rms_pos + ch * self.rms_window;
                let old = self.rms_buf[ring_ch];
                self.rms_sq[ch] -= f64::from(old * old);
                self.rms_sq[ch] += f64::from(s * s);
                self.rms_buf[ring_ch] = s;
            }
            self.rms_pos += 1;
            if self.rms_pos >= self.rms_window {
                self.rms_pos = 0;
            }
        }
        self.rms_filled = (self.rms_filled + n).min(self.rms_window);
    }

    pub(crate) fn peak_db(&self, ch: usize) -> f32 {
        if !self.processed_once {
            return -f32::INFINITY;
        }
        master_lin_to_db(self.peak[ch])
    }

    pub(crate) fn rms_db(&self, ch: usize) -> f32 {
        if !self.processed_once || self.rms_filled == 0 {
            return -f32::INFINITY;
        }
        let mean = self.rms_sq[ch] / self.rms_filled as f64;
        master_lin_to_db(mean.sqrt() as f32)
    }
}

/// dBFS conversion matching the engine meter's floor (`linear_to_db`).
fn master_lin_to_db(lin: f32) -> f32 {
    if lin <= 0.0 {
        -120.0
    } else {
        20.0 * lin.log10()
    }
}
