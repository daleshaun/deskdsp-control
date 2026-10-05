//! Studio Noise Gate / Downward Expander for Vocal Tracking.

use super::envelope::EnvelopeFollower;
use super::DspNode;

#[derive(Debug, Clone)]
pub struct NoiseGate {
    pub threshold_db: f32,
    pub ratio: f32,
    pub floor_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub hold_samples: usize,
    
    envelope: EnvelopeFollower,
    hold_counter: usize,
    current_gain: f32,
    gain_coeff: f32,
    pub bypassed: bool,
    sample_rate: f32,
}

impl NoiseGate {
    pub fn new(sample_rate: f32) -> Self {
        let mut gate = Self {
            threshold_db: -55.0,
            ratio: 4.0,
            floor_db: -60.0,
            attack_ms: 1.0,
            release_ms: 80.0,
            hold_samples: (sample_rate * 0.02) as usize, // 20ms hold
            envelope: EnvelopeFollower::new(sample_rate, 1.0, 50.0),
            hold_counter: 0,
            current_gain: 1.0,
            gain_coeff: 0.0,
            bypassed: false,
            sample_rate,
        };
        gate.recalculate();
        gate
    }

    pub fn set_params(&mut self, threshold_db: f32, ratio: f32, attack_ms: f32, release_ms: f32, floor_db: f32) {
        self.threshold_db = threshold_db.clamp(-90.0, -10.0);
        self.ratio = ratio.clamp(1.0, 20.0);
        self.attack_ms = attack_ms.clamp(0.1, 50.0);
        self.release_ms = release_ms.clamp(5.0, 1000.0);
        self.floor_db = floor_db.clamp(-96.0, 0.0);
        self.recalculate();
    }

    fn recalculate(&mut self) {
        self.envelope.set_times(self.attack_ms, self.release_ms);
        self.gain_coeff = 1.0 - (-1.0 / (0.001 * self.release_ms * self.sample_rate)).exp();
    }

    pub fn current_reduction_db(&self) -> f32 {
        if self.current_gain > 1e-4 {
            -20.0 * self.current_gain.log10()
        } else {
            -self.floor_db
        }
    }
}

impl DspNode for NoiseGate {
    fn name(&self) -> &'static str {
        "Noise Gate / Expander"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.envelope.reset();
        self.hold_counter = 0;
        self.current_gain = 1.0;
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }

        let env = self.envelope.process(input);
        let env_db = if env > 1e-5 { 20.0 * env.log10() } else { -100.0 };

        let target_gain = if env_db >= self.threshold_db {
            self.hold_counter = self.hold_samples;
            1.0
        } else if self.hold_counter > 0 {
            self.hold_counter -= 1;
            1.0
        } else {
            // Downward expansion below threshold
            let delta = env_db - self.threshold_db;
            let reduction_db = delta * (self.ratio - 1.0);
            let clamped_reduction = reduction_db.max(self.floor_db);
            10.0_f32.powf(clamped_reduction / 20.0)
        };

        // Smooth gain transition
        self.current_gain += (target_gain - self.current_gain) * self.gain_coeff;
        input * self.current_gain
    }

    fn telemetry(&self) -> super::NodeTelemetry {
        super::NodeTelemetry {
            gain_reduction_db: self.current_reduction_db(),
            ..Default::default()
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
