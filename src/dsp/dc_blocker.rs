//! One-pole DC Blocker filter (ported from desk-mic-forge).
//! Removes DC offset introduced by asymmetric nonlinearities.

use super::DspNode;

#[derive(Debug, Clone)]
pub struct DcBlocker {
    r: f32,
    prev_input: f32,
    prev_output: f32,
    bypassed: bool,
}

impl DcBlocker {
    pub fn new(sample_rate: f32) -> Self {
        let cutoff_hz = 15.0_f32;
        let r = (-2.0 * std::f32::consts::PI * cutoff_hz / sample_rate).exp();
        Self {
            r,
            prev_input: 0.0,
            prev_output: 0.0,
            bypassed: false,
        }
    }
}

impl DspNode for DcBlocker {
    fn name(&self) -> &'static str {
        "DC Blocker"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.prev_input = 0.0;
        self.prev_output = 0.0;
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }
        let output = input - self.prev_input + self.r * self.prev_output;
        self.prev_input = input;
        self.prev_output = output;
        output
    }
}
