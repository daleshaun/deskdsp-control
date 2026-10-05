//! Vocal De-Esser using high-frequency sidechain detection.

use super::biquad::{BiquadFilter, FilterType};
use super::envelope::EnvelopeFollower;
use super::DspNode;

#[derive(Debug, Clone)]
pub struct DeEsser {
    pub threshold_db: f32,
    pub ratio: f32,
    pub frequency: f32,
    pub amount_db: f32,
    
    sidechain_filter: BiquadFilter,
    envelope: EnvelopeFollower,
    current_attenuation_db: f32,
    gain_smoothing: f32,
    current_gain: f32,
    pub bypassed: bool,
}

impl DeEsser {
    pub fn new(sample_rate: f32) -> Self {
        let frequency = 6500.0_f32; // Standard vocal sibilance center
        let sidechain_filter = BiquadFilter::new(FilterType::BandPass { q: 1.5 }, frequency, 0.0, sample_rate);
        let envelope = EnvelopeFollower::new(sample_rate, 0.5, 40.0); // fast attack, moderate release

        Self {
            threshold_db: -20.0,
            ratio: 4.0,
            frequency,
            amount_db: 4.0,
            sidechain_filter,
            envelope,
            current_attenuation_db: 0.0,
            gain_smoothing: 0.15,
            current_gain: 1.0,
            bypassed: false,
        }
    }

    pub fn set_amount(&mut self, amount: f32) {
        self.amount_db = amount.clamp(0.0, 12.0);
        self.threshold_db = -10.0 - (self.amount_db * 2.5);
    }

    pub fn set_params(&mut self, threshold_db: f32, ratio: f32, frequency: f32) {
        self.threshold_db = threshold_db.clamp(-50.0, 0.0);
        self.ratio = ratio.clamp(1.5, 12.0);
        self.frequency = frequency.clamp(3000.0, 11000.0);
        self.sidechain_filter.set_cutoff(self.frequency);
    }

    pub fn gain_reduction_db(&self) -> f32 {
        self.current_attenuation_db
    }
}

impl DspNode for DeEsser {
    fn name(&self) -> &'static str {
        "Vocal De-Esser"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.sidechain_filter.reset();
        self.envelope.reset();
        self.current_attenuation_db = 0.0;
        self.current_gain = 1.0;
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }

        // 1. Isolate sibilant band in sidechain
        let sc = self.sidechain_filter.process_sample(input);
        let env = self.envelope.process(sc);
        let env_db = if env > 1e-5 { 20.0 * env.log10() } else { -100.0 };

        // 2. Compute gain reduction when sibilance exceeds threshold
        let target_reduction_db = if env_db > self.threshold_db {
            let over = env_db - self.threshold_db;
            (1.0 - 1.0 / self.ratio) * over
        } else {
            0.0
        };

        self.current_attenuation_db = target_reduction_db;
        let target_gain = 10.0_f32.powf(-target_reduction_db / 20.0);
        self.current_gain += (target_gain - self.current_gain) * self.gain_smoothing;

        input * self.current_gain
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
