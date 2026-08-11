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


use oximedia_mixer::dynamics::{Compressor, CompressorConfig, Expander, ExpanderConfig, Gate, GateConfig};

/// Adapter wrapping `Compressor` as an `AudioEffect`.
pub struct CompressorEffect {
    compressor: Compressor,
    sample_rate: u32,
}

impl CompressorEffect {
    #[must_use]
    pub fn new(config: CompressorConfig, sample_rate: u32) -> Self {
        Self {
            compressor: Compressor::new(config),
            sample_rate,
        }
    }

    /// Create a compressor with standard broadcast settings.
    /// -12 dB threshold, 3:1 ratio, 5ms attack, 100ms release, +3dB makeup.
    #[must_use]
    pub fn broadcast(sample_rate: u32) -> Self {
        Self::new(
            CompressorConfig {
                threshold_db: -12.0,
                ratio: 3.0,
                attack_ms: 5.0,
                release_ms: 100.0,
                makeup_gain_db: 3.0,
                knee_db: 3.0,
            },
            sample_rate,
        )
    }

    #[must_use]
    pub fn inner_mut(&mut self) -> &mut Compressor {
        &mut self.compressor
    }
}

impl AudioEffect for CompressorEffect {
    fn process(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            *s = self.compressor.process_sample(*s, self.sample_rate);
        }
    }

    fn name(&self) -> &str {
        "Compressor"
    }
}

/// Adapter wrapping `Gate` as an `AudioEffect`.
pub struct GateEffect {
    gate: Gate,
    sample_rate: u32,
}

impl GateEffect {
    #[must_use]
    pub fn new(config: GateConfig, sample_rate: u32) -> Self {
        Self {
            gate: Gate::new(config),
            sample_rate,
        }
    }

    /// Create a gate with standard settings for cleaning up background noise.
    /// -50 dB threshold, 2ms attack, 100ms release.
    #[must_use]
    pub fn denoise(sample_rate: u32) -> Self {
        Self::new(
            GateConfig {
                threshold_db: -50.0,
                attack_ms: 2.0,
                release_ms: 100.0,
                ..Default::default()
            },
            sample_rate,
        )
    }

    #[must_use]
    pub fn inner_mut(&mut self) -> &mut Gate {
        &mut self.gate
    }
}

impl AudioEffect for GateEffect {
    fn process(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            *s = self.gate.process_sample(*s, self.sample_rate);
        }
    }

    fn name(&self) -> &str {
        "Gate"
    }
}

/// Adapter wrapping `Expander` as an `AudioEffect`.
pub struct ExpanderEffect {
    expander: Expander,
    sample_rate: u32,
}

impl ExpanderEffect {
    #[must_use]
    pub fn new(config: ExpanderConfig, sample_rate: u32) -> Self {
        Self {
            expander: Expander::new(config),
            sample_rate,
        }
    }

    #[must_use]
    pub fn inner_mut(&mut self) -> &mut Expander {
        &mut self.expander
    }
}

impl AudioEffect for ExpanderEffect {
    fn process(&mut self, samples: &mut [f32]) {
        for s in samples.iter_mut() {
            *s = self.expander.process_sample(*s, self.sample_rate);
        }
    }

    fn name(&self) -> &str {
        "Expander"
    }
}
