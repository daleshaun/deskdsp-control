//! Speaker Cabinet Simulator using Voiced Parametric Biquad Cascades.
//! Emulates 1x12, 2x12, and 4x12 guitar speaker enclosures with zero heap allocations.

#![allow(dead_code)]

use super::biquad::{BiquadFilter, FilterType};
use super::DspNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CabType {
    None,
    OneByTwelve,
    TwoByTwelve,
    FourByTwelve,
}

#[derive(Debug, Clone)]
pub struct CabSim {
    pub cab_type: CabType,
    pub bypassed: bool,
    sample_rate: f32,

    // Speaker acoustic response stages:
    // 1. Bass cutoff / sub-rumble filter
    hpf: BiquadFilter,
    // 2. Speaker mechanical resonance peak (thump)
    bass_resonance: BiquadFilter,
    // 3. Cabinet box reflection / mid scoop
    mid_scoop: BiquadFilter,
    // 4. Speaker cone cone-breakup presence peak
    presence: BiquadFilter,
    // 5. Steep high-frequency speaker cone rolloff
    lpf1: BiquadFilter,
    lpf2: BiquadFilter,
}

impl CabSim {
    pub fn new(sample_rate: f32) -> Self {
        let mut cab = Self {
            cab_type: CabType::FourByTwelve,
            bypassed: false,
            sample_rate,
            hpf: BiquadFilter::new(FilterType::HighPass, 80.0, 0.0, sample_rate),
            bass_resonance: BiquadFilter::new(FilterType::Peaking { q: 2.2 }, 105.0, 4.0, sample_rate),
            mid_scoop: BiquadFilter::new(FilterType::Peaking { q: 1.0 }, 420.0, -3.5, sample_rate),
            presence: BiquadFilter::new(FilterType::Peaking { q: 1.8 }, 3200.0, 4.5, sample_rate),
            lpf1: BiquadFilter::new(FilterType::LowPass, 5000.0, 0.0, sample_rate),
            lpf2: BiquadFilter::new(FilterType::LowPass, 5500.0, 0.0, sample_rate),
        };
        cab.apply_cab_type(CabType::FourByTwelve);
        cab
    }

    pub fn set_cab_type(&mut self, cab_type: CabType) {
        self.cab_type = cab_type;
        self.apply_cab_type(cab_type);
    }

    fn apply_cab_type(&mut self, cab_type: CabType) {
        match cab_type {
            CabType::None => {
                // Flat passthrough
                self.hpf.set_cutoff(20.0);
                self.bass_resonance.set_gain_db(0.0);
                self.mid_scoop.set_gain_db(0.0);
                self.presence.set_gain_db(0.0);
                self.lpf1.set_cutoff(20000.0);
                self.lpf2.set_cutoff(20000.0);
            }
            CabType::OneByTwelve => {
                // Open-back 1x12 (bright, airy, snappy)
                self.hpf.set_cutoff(85.0);
                self.bass_resonance.set_cutoff(115.0);
                self.bass_resonance.set_gain_db(2.5);
                self.mid_scoop.set_cutoff(550.0);
                self.mid_scoop.set_gain_db(-2.0);
                self.presence.set_cutoff(3400.0);
                self.presence.set_gain_db(5.0);
                self.lpf1.set_cutoff(5600.0);
                self.lpf2.set_cutoff(6200.0);
            }
            CabType::TwoByTwelve => {
                // 2x12 British combo (punchy, warm, defined)
                self.hpf.set_cutoff(80.0);
                self.bass_resonance.set_cutoff(110.0);
                self.bass_resonance.set_gain_db(3.5);
                self.mid_scoop.set_cutoff(480.0);
                self.mid_scoop.set_gain_db(-3.0);
                self.presence.set_cutoff(3100.0);
                self.presence.set_gain_db(4.0);
                self.lpf1.set_cutoff(5200.0);
                self.lpf2.set_cutoff(5800.0);
            }
            CabType::FourByTwelve => {
                // Closed-back 4x12 Stack (deep thump, scooped low-mids, tight aggressive cut)
                self.hpf.set_cutoff(75.0);
                self.bass_resonance.set_cutoff(100.0);
                self.bass_resonance.set_gain_db(4.8);
                self.mid_scoop.set_cutoff(400.0);
                self.mid_scoop.set_gain_db(-4.0);
                self.presence.set_cutoff(2800.0);
                self.presence.set_gain_db(4.5);
                self.lpf1.set_cutoff(4800.0);
                self.lpf2.set_cutoff(5400.0);
            }
        }
    }
}

impl DspNode for CabSim {
    fn name(&self) -> &'static str {
        match self.cab_type {
            CabType::None => "Direct Cab (Bypass)",
            CabType::OneByTwelve => "1x12 Open Back Cab",
            CabType::TwoByTwelve => "2x12 Combo Cab",
            CabType::FourByTwelve => "4x12 Stack Cab",
        }
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed || self.cab_type == CabType::None
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.hpf.reset();
        self.bass_resonance.reset();
        self.mid_scoop.reset();
        self.presence.reset();
        self.lpf1.reset();
        self.lpf2.reset();
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.is_bypassed() {
            return input;
        }

        if !input.is_finite() {
            return 0.0;
        }

        let mut sample = input;
        sample = self.hpf.process_sample(sample);
        sample = self.bass_resonance.process_sample(sample);
        sample = self.mid_scoop.process_sample(sample);
        sample = self.presence.process_sample(sample);
        sample = self.lpf1.process_sample(sample);
        sample = self.lpf2.process_sample(sample);

        sample.clamp(-1.0, 1.0)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
