//! Analog Tape & Tube Saturation with asymmetric harmonics and DC blocking.
//! Ported and extended from desk-mic-forge.

use super::dc_blocker::DcBlocker;
use super::DspNode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SaturationFlavor {
    Tube,
    Tape,
}

#[derive(Debug, Clone)]
pub struct Saturation {
    pub flavor: SaturationFlavor,
    pub drive: f32,
    pub asymmetry: f32,
    pub mix: f32,
    dc_blocker: DcBlocker,
    pub bypassed: bool,
}

impl Saturation {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            flavor: SaturationFlavor::Tube,
            drive: 1.0,      // Unity drive default
            asymmetry: 0.15, // Tube warmth with 2nd harmonic
            mix: 1.0,
            dc_blocker: DcBlocker::new(sample_rate),
            bypassed: false,
        }
    }

    pub fn set_flavor(&mut self, flavor: SaturationFlavor) {
        self.flavor = flavor;
        match flavor {
            SaturationFlavor::Tube => {
                self.asymmetry = 0.18; // More even harmonics
            }
            SaturationFlavor::Tape => {
                self.asymmetry = 0.05; // Predominantly 3rd harmonic symmetric compression
            }
        }
    }

    pub fn set_drive(&mut self, drive: f32) {
        self.drive = drive.clamp(1.0, 20.0);
    }

    pub fn set_params(&mut self, drive: f32, mix: f32) {
        self.drive = drive.clamp(1.0, 20.0);
        self.mix = mix.clamp(0.0, 1.0);
    }
}

impl DspNode for Saturation {
    fn name(&self) -> &'static str {
        match self.flavor {
            SaturationFlavor::Tube => "Tube Saturation",
            SaturationFlavor::Tape => "Tape Saturation",
        }
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.dc_blocker.reset();
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if !input.is_finite() {
            return 0.0;
        }
        if self.bypassed || (self.drive <= 1.001 && self.mix <= 0.001) {
            return input;
        }

        let clamped_in = input.clamp(-2.0, 2.0);

        // Apply asymmetric drive
        let driven = (clamped_in + self.asymmetry) * self.drive;
        
        // Soft clipping transfer curve
        let saturated = match self.flavor {
            SaturationFlavor::Tube => {
                // Tanh waveshaping with bias offset compensation
                driven.tanh() - self.asymmetry.tanh()
            }
            SaturationFlavor::Tape => {
                // Polynomial / cubic soft saturation curve: x - x^3 / 3
                let x = driven.clamp(-1.5, 1.5);
                x - (x * x * x) / 3.0
            }
        };

        // Remove any DC offset introduced by asymmetry
        let dc_cleaned = self.dc_blocker.process_sample(saturated);

        // Compensate perceived volume so high drive doesn't violently peak
        let compensated = dc_cleaned / (1.0 + (self.drive - 1.0) * 0.4);

        // Dry/wet mix
        let out = (1.0 - self.mix) * clamped_in + self.mix * compensated;
        out.clamp(-1.5, 1.5)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
