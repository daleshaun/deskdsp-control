//! Classic VCA Mix-Bus Glue Compressor with Sidechain HPF and Tape Warmth.

#![allow(dead_code)]

use super::biquad::{BiquadFilter, FilterType};
use super::saturation::{Saturation, SaturationFlavor};
use super::{DspNode, StereoDspNode};

#[derive(Debug, Clone)]
pub struct GlueCompressorSaturator {
    pub threshold_db: f32,
    pub ratio: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_db: f32,
    
    // Sidechain high-pass filter (prevents heavy sub-bass from pumping the mix)
    sc_hp: (BiquadFilter, BiquadFilter),
    
    // Envelope follower
    envelope: f32,
    attack_coeff: f32,
    release_coeff: f32,
    current_gr_db: f32,

    // Mix bus tape saturation
    tape_sat: (Saturation, Saturation),
    
    sample_rate: f32,
    pub bypassed: bool,
}

impl GlueCompressorSaturator {
    pub fn new(sample_rate: f32) -> Self {
        let mut glue = Self {
            threshold_db: -12.0,
            ratio: 2.5,
            attack_ms: 30.0,  // classic slow 30ms punch attack
            release_ms: 100.0,
            makeup_db: 1.5,
            sc_hp: (
                BiquadFilter::new(FilterType::HighPass, 100.0, 0.0, sample_rate),
                BiquadFilter::new(FilterType::HighPass, 100.0, 0.0, sample_rate),
            ),
            envelope: 0.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            current_gr_db: 0.0,
            tape_sat: {
                let mut s1 = Saturation::new(sample_rate);
                s1.set_flavor(SaturationFlavor::Tape);
                s1.set_params(1.2, 0.5); // subtle mix-bus glue drive
                let mut s2 = Saturation::new(sample_rate);
                s2.set_flavor(SaturationFlavor::Tape);
                s2.set_params(1.2, 0.5);
                (s1, s2)
            },
            sample_rate,
            bypassed: false,
        };
        glue.recalculate();
        glue
    }

    pub fn set_params(&mut self, threshold_db: f32, ratio: f32, attack_ms: f32, release_ms: f32, makeup_db: f32) {
        self.threshold_db = threshold_db.clamp(-40.0, 0.0);
        self.ratio = ratio.clamp(1.5, 10.0);
        self.attack_ms = attack_ms.clamp(1.0, 100.0);
        self.release_ms = release_ms.clamp(10.0, 1200.0);
        self.makeup_db = makeup_db.clamp(0.0, 18.0);
        self.recalculate();
    }

    fn recalculate(&mut self) {
        self.attack_coeff = (-1.0 / (self.attack_ms * 0.001 * self.sample_rate)).exp();
        self.release_coeff = (-1.0 / (self.release_ms * 0.001 * self.sample_rate)).exp();
    }

    pub fn gain_reduction_db(&self) -> f32 {
        self.current_gr_db
    }
}

impl StereoDspNode for GlueCompressorSaturator {
    fn name(&self) -> &'static str {
        "Glue Compressor & Saturation"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.envelope = 0.0;
        self.current_gr_db = 0.0;
        self.sc_hp.0.reset();
        self.sc_hp.1.reset();
        self.tape_sat.0.reset();
        self.tape_sat.1.reset();
    }

    #[inline(always)]
    fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.bypassed {
            return (left, right);
        }

        // 1. Sidechain signal with High-Pass filtering
        let sc_l = self.sc_hp.0.process_sample(left);
        let sc_r = self.sc_hp.1.process_sample(right);
        let sc_level = sc_l.abs().max(sc_r.abs());

        // 2. VCA envelope tracking
        if sc_level > self.envelope {
            self.envelope = self.attack_coeff * self.envelope + (1.0 - self.attack_coeff) * sc_level;
        } else {
            self.envelope = self.release_coeff * self.envelope + (1.0 - self.release_coeff) * sc_level;
        }

        let env_db = if self.envelope > 1e-6 {
            20.0 * self.envelope.log10()
        } else {
            -120.0
        };

        // 3. Compression curve
        let target_gr_db = if env_db > self.threshold_db {
            (1.0 / self.ratio - 1.0) * (env_db - self.threshold_db)
        } else {
            0.0
        };

        self.current_gr_db = -target_gr_db;

        // Apply gain reduction + makeup
        let linear_gain = 10.0_f32.powf((target_gr_db + self.makeup_db) / 20.0);
        let comp_l = left * linear_gain;
        let comp_r = right * linear_gain;

        // 4. Subtle mix-bus tape saturation
        let sat_l = self.tape_sat.0.process_sample(comp_l);
        let sat_r = self.tape_sat.1.process_sample(comp_r);

        (sat_l, sat_r)
    }

    fn telemetry(&self) -> super::NodeTelemetry {
        super::NodeTelemetry {
            gain_reduction_db: self.current_gr_db,
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
