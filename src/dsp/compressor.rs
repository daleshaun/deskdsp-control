//! Studio Dynamic Range Compressor with Soft-Knee and Opto/FET mode behavior.

use super::DspNode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CompressorFlavor {
    Opto,
    Fet,
    Clean,
}

#[derive(Debug, Clone)]
pub struct VocalCompressor {
    pub flavor: CompressorFlavor,
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub knee_width_db: f32,
    pub makeup_db: f32,
    
    sample_rate: f32,
    envelope: f32,
    envelope_slow: f32,
    attack_coeff: f32,
    release_coeff: f32,
    current_gain_reduction_db: f32,
    pub bypassed: bool,
}

impl VocalCompressor {
    pub fn new(sample_rate: f32) -> Self {
        let mut comp = Self {
            flavor: CompressorFlavor::Opto,
            threshold_db: -18.0,
            ratio: 4.0,
            attack_ms: 10.0,
            release_ms: 120.0,
            knee_width_db: 6.0, // wide soft knee
            makeup_db: 3.0,
            sample_rate,
            envelope: 0.0,
            envelope_slow: 0.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            current_gain_reduction_db: 0.0,
            bypassed: false,
        };
        comp.recalculate();
        comp
    }

    pub fn set_flavor(&mut self, flavor: CompressorFlavor) {
        self.flavor = flavor;
        match flavor {
            CompressorFlavor::Opto => {
                self.attack_ms = 10.0;
                self.release_ms = 180.0;
                self.ratio = 3.5;
                self.knee_width_db = 8.0;
            }
            CompressorFlavor::Fet => {
                self.attack_ms = 0.8;
                self.release_ms = 60.0;
                self.ratio = 4.0;
                self.knee_width_db = 3.0;
            }
            CompressorFlavor::Clean => {
                self.attack_ms = 5.0;
                self.release_ms = 100.0;
                self.ratio = 4.0;
                self.knee_width_db = 4.0;
            }
        }
        self.recalculate();
    }

    pub fn set_params(&mut self, threshold_db: f32, ratio: f32, attack_ms: f32, release_ms: f32, makeup_db: f32) {
        self.threshold_db = threshold_db.clamp(-60.0, 0.0);
        self.ratio = ratio.clamp(1.0, 20.0);
        self.attack_ms = attack_ms.clamp(0.1, 500.0);
        self.release_ms = release_ms.clamp(5.0, 2000.0);
        self.makeup_db = makeup_db.clamp(0.0, 36.0);
        self.recalculate();
    }

    fn recalculate(&mut self) {
        self.attack_coeff = (-1.0 / (self.attack_ms * 0.001 * self.sample_rate)).exp();
        self.release_coeff = (-1.0 / (self.release_ms * 0.001 * self.sample_rate)).exp();
    }

    pub fn gain_reduction_db(&self) -> f32 {
        self.current_gain_reduction_db
    }
}

impl DspNode for VocalCompressor {
    fn name(&self) -> &'static str {
        match self.flavor {
            CompressorFlavor::Opto => "Vocal Compressor (Opto)",
            CompressorFlavor::Fet => "Vocal Compressor (FET)",
            CompressorFlavor::Clean => "Vocal Compressor (Clean)",
        }
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.envelope_slow = 0.0;
        self.current_gain_reduction_db = 0.0;
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }

        let input_level = input.abs();

        // Envelope detection with flavor dynamics
        if input_level > self.envelope {
            self.envelope = self.attack_coeff * self.envelope + (1.0 - self.attack_coeff) * input_level;
        } else {
            let rel_coeff = if self.flavor == CompressorFlavor::Opto {
                // Two-stage release for Opto (fast initial release, slower tail)
                let slow_coeff = (-1.0 / (self.release_ms * 0.003 * self.sample_rate)).exp();
                self.envelope_slow = slow_coeff * self.envelope_slow + (1.0 - slow_coeff) * input_level;
                self.release_coeff * 0.6 + slow_coeff * 0.4
            } else {
                self.release_coeff
            };
            self.envelope = rel_coeff * self.envelope + (1.0 - rel_coeff) * input_level;
        }

        let env_db = if self.envelope > 1e-6 {
            20.0 * self.envelope.log10()
        } else {
            -120.0
        };

        // Static characteristic with soft knee
        let half_knee = self.knee_width_db * 0.5;
        let delta = env_db - self.threshold_db;

        let target_gain_db = if delta < -half_knee {
            0.0
        } else if delta > half_knee {
            (1.0 / self.ratio - 1.0) * delta
        } else {
            // Soft-knee parabola
            let k = delta + half_knee;
            (1.0 / self.ratio - 1.0) * (k * k) / (2.0 * self.knee_width_db)
        };

        self.current_gain_reduction_db = -target_gain_db;

        // Apply gain reduction + makeup
        let total_gain_db = target_gain_db + self.makeup_db;
        let linear_gain = 10.0_f32.powf(total_gain_db / 20.0);

        input * linear_gain
    }

    fn telemetry(&self) -> super::NodeTelemetry {
        super::NodeTelemetry {
            gain_reduction_db: self.gain_reduction_db(),
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
