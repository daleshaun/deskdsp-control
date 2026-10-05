//! Harmonic Exciter / Air Enhancement DSP Node.
//!
//! Enhances clarity, presence, and vocal "air" by filtering high frequencies
//! through an asymmetric non-linear saturation curve to generate subtle 2nd and 3rd
//! harmonic overtones, blending them back with the dry signal.
//!
//! Real-Time Guarantees:
//! - 100% allocation-free in `process_sample()`.
//! - Bit-identical passthrough when bypassed.
//! - High-pass sidechain implemented with 64-bit biquad accumulator precision.

use super::biquad::{BiquadFilter, FilterType};
use super::{DspNode, NodeTelemetry};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExciterFlavor {
    /// Tube-style warmth with prominent 2nd harmonic (even overtones).
    Tube,
    /// Tape-style brilliance with odd harmonics.
    Tape,
    /// High-frequency "Air" band exciter for vocal presence above 5 kHz.
    Air,
}

#[derive(Debug, Clone)]
pub struct HarmonicExciter {
    pub frequency_hz: f32,
    pub drive: f32,
    pub blend: f32, // 0.0 (dry) to 1.0 (maximum excitation)
    pub flavor: ExciterFlavor,
    pub bypassed: bool,
    hpf: BiquadFilter,
    sample_rate: f32,
}

impl HarmonicExciter {
    pub fn new(sample_rate: f32) -> Self {
        let default_freq = 4000.0_f32;
        Self {
            frequency_hz: default_freq,
            drive: 2.0,
            blend: 0.25,
            flavor: ExciterFlavor::Air,
            bypassed: false,
            hpf: BiquadFilter::new(FilterType::HighPass, default_freq, 0.0, sample_rate),
            sample_rate,
        }
    }

    pub fn set_params(&mut self, frequency_hz: f32, drive: f32, blend: f32) {
        self.frequency_hz = frequency_hz.clamp(1000.0, 16000.0);
        self.drive = drive.clamp(0.5, 10.0);
        self.blend = blend.clamp(0.0, 1.0);
        self.hpf.set_cutoff(self.frequency_hz);
    }

    pub fn set_flavor(&mut self, flavor: ExciterFlavor) {
        self.flavor = flavor;
    }
}

impl DspNode for HarmonicExciter {
    fn name(&self) -> &'static str {
        "Harmonic Exciter"
    }

    #[inline(always)]
    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.hpf.reset();
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        // 1. Bit-identical bypass guarantee
        if self.bypassed || self.blend < 1e-4 {
            return input;
        }

        // 2. High-pass sidechain (f64 filter state)
        let hp = self.hpf.process_sample(input);

        // 3. Harmonic generation via non-linear polynomial saturation
        let x = hp * self.drive;
        let harmonics = match self.flavor {
            ExciterFlavor::Tube => {
                // Asymmetric quadratic (even 2nd harmonic) + mild cubic (odd 3rd harmonic)
                (x + 0.35 * x * x - 0.12 * x * x * x).clamp(-1.5, 1.5)
            }
            ExciterFlavor::Tape => {
                // Symmetric soft-saturation: 3rd and 5th harmonics
                x / (1.0 + x * x).sqrt()
            }
            ExciterFlavor::Air => {
                // High-frequency exciter: gentle soft-clipping with subtle octave overtone
                let overtone = 0.25 * x * x;
                (x + overtone).tanh()
            }
        };

        // 4. Blend excited harmonics back with the dry signal
        input + harmonics * self.blend * 0.4
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn telemetry(&self) -> NodeTelemetry {
        NodeTelemetry::default()
    }
}
