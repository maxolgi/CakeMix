//! AudioEffect adapters that bridge oximedia-mixer DSP modules
//! into the runtime effects chain.
//!
//! These adapters wrap modules that operate on f64 (ParametricEq) or
//! per-sample APIs (dynamics) into the `AudioEffect` trait's
//! `fn process(&mut self, samples: &mut [f32])` signature.

use oximedia_mixer::effects_chain::AudioEffect;
use oximedia_mixer::eq_band::{EqFilterType, ParametricEq};

/// Adapter wrapping `ParametricEq` as an `AudioEffect` for the effects chain.
///
/// ParametricEq operates on f64 buffers; this adapter converts f32→f64,
/// processes, and converts back. Mono only (1 channel) since the mixer's
/// working buffer is mono per channel.
pub struct EqEffect {
    eq: ParametricEq,
    // Scratch buffer for f64 conversion (reused per call).
    scratch: Vec<f64>,
}

impl EqEffect {
    /// Create a new EQ effect with a single peaking band.
    #[must_use]
    pub fn peaking(sample_rate: u32, freq: f64, gain_db: f64, q: f64) -> Self {
        let mut eq = ParametricEq::new(sample_rate, 1);
        eq.add_band("Peak".into(), EqFilterType::Peaking, freq, gain_db, q);
        Self {
            eq,
            scratch: Vec::new(),
        }
    }

    /// Create a new EQ effect with a 4-band standard configuration.
    #[must_use]
    pub fn four_band(sample_rate: u32) -> Self {
        let eq = ParametricEq::four_band(sample_rate, 1);
        Self {
            eq,
            scratch: Vec::new(),
        }
    }

    /// Create from an existing ParametricEq.
    #[must_use]
    pub fn from_eq(eq: ParametricEq) -> Self {
        Self {
            eq,
            scratch: Vec::new(),
        }
    }

    /// Get a reference to the inner EQ for parameter changes.
    #[must_use]
    pub fn inner(&self) -> &ParametricEq {
        &self.eq
    }

    /// Get a mutable reference to the inner EQ for parameter changes.
    #[must_use]
    pub fn inner_mut(&mut self) -> &mut ParametricEq {
        &mut self.eq
    }
}

impl AudioEffect for EqEffect {
    fn process(&mut self, samples: &mut [f32]) {
        // Convert f32 → f64, process through EQ, convert back.
        self.scratch.clear();
        self.scratch.extend(samples.iter().map(|&s| s as f64));
        self.eq.process_buffer(&mut self.scratch, 1);
        for (out, &proc_sample) in samples.iter_mut().zip(&self.scratch) {
            *out = proc_sample as f32;
        }
    }

    fn name(&self) -> &str {
        "ParametricEQ"
    }
}
