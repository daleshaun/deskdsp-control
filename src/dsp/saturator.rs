//! Analog Tape & Tube Saturation with asymmetric harmonics and DC blocking.
#![allow(dead_code)]

#[derive(Debug, Clone)]
pub struct Saturator {
    drive: f32,
    asymmetry: f32,
    mix: f32,
    dc_x1: f32,
    dc_y1: f32,
}

impl Saturator {
    pub fn new() -> Self {
        Self {
            drive: 1.0,
            asymmetry: 0.1, // subtle 2nd harmonic
            mix: 1.0,
            dc_x1: 0.0,
            dc_y1: 0.0,
        }
    }

    pub fn set_drive(&mut self, drive: f32) {
        self.drive = drive.clamp(1.0, 20.0);
    }

    pub fn set_asymmetry(&mut self, asymmetry: f32) {
        self.asymmetry = asymmetry.clamp(0.0, 0.5);
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    #[inline(always)]
    pub fn process(&mut self, input: f32) -> f32 {
        if self.drive <= 1.001 && self.mix <= 0.001 {
            return input;
        }

        // Apply asymmetric drive
        let driven = (input + self.asymmetry) * self.drive;
        
        // Soft clipping transfer function (analog tube saturation curve)
        let saturated = driven.tanh() - self.asymmetry.tanh();

        // DC Blocker filter (y[n] = x[n] - x[n-1] + 0.995 * y[n-1])
        let dc_blocked = saturated - self.dc_x1 + 0.995 * self.dc_y1;
        self.dc_x1 = saturated;
        self.dc_y1 = dc_blocked;

        // Level compensation so high drive doesn't violently blow out levels
        let compensated = dc_blocked / (self.drive.sqrt());

        // Dry/wet blend
        (1.0 - self.mix) * input + self.mix * compensated
    }
}
