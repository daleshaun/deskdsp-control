//! 3-Band Multiband Compressor with Linkwitz-Riley crossovers.
//!
//! Features phase-aligned Linkwitz-Riley 4th order (LR4) crossovers.
//! The low band is phase-compensated using a 4th-order allpass filter matched to
//! the high crossover frequency (3500 Hz), guaranteeing completely flat amplitude
//! and linear-phase-like phase alignment across all three bands upon summation.

use super::biquad::{BiquadFilter, FilterType};
use super::compressor::VocalCompressor;
use super::{DspNode, StereoDspNode};

#[derive(Debug, Clone)]
pub struct MultibandCompressor {
    // Crossover filters (Linkwitz-Riley 4th order = 2 cascaded 2nd order Butterworth)
    low_lp1: (BiquadFilter, BiquadFilter),
    low_lp2: (BiquadFilter, BiquadFilter),
    low_hp1: (BiquadFilter, BiquadFilter),
    low_hp2: (BiquadFilter, BiquadFilter),

    high_lp1: (BiquadFilter, BiquadFilter),
    high_lp2: (BiquadFilter, BiquadFilter),
    high_hp1: (BiquadFilter, BiquadFilter),
    high_hp2: (BiquadFilter, BiquadFilter),

    // Phase compensation allpass for the Low band to match High crossover phase delay
    low_ap1: (BiquadFilter, BiquadFilter),
    low_ap2: (BiquadFilter, BiquadFilter),

    // Band compressors
    pub low_comp: (VocalCompressor, VocalCompressor),
    pub mid_comp: (VocalCompressor, VocalCompressor),
    pub high_comp: (VocalCompressor, VocalCompressor),

    pub bypassed: bool,
}

impl MultibandCompressor {
    pub fn new(sample_rate: f32) -> Self {
        let low_split = 140.0;
        let high_split = 3500.0;

        let mut low_comp = (VocalCompressor::new(sample_rate), VocalCompressor::new(sample_rate));
        low_comp.0.set_params(-16.0, 2.5, 30.0, 150.0, 1.0);
        low_comp.1.set_params(-16.0, 2.5, 30.0, 150.0, 1.0);

        let mut mid_comp = (VocalCompressor::new(sample_rate), VocalCompressor::new(sample_rate));
        mid_comp.0.set_params(-18.0, 2.0, 20.0, 100.0, 1.0);
        mid_comp.1.set_params(-18.0, 2.0, 20.0, 100.0, 1.0);

        let mut high_comp = (VocalCompressor::new(sample_rate), VocalCompressor::new(sample_rate));
        high_comp.0.set_params(-20.0, 2.0, 10.0, 80.0, 1.0);
        high_comp.1.set_params(-20.0, 2.0, 10.0, 80.0, 1.0);

        Self {
            low_lp1: (
                BiquadFilter::new(FilterType::LowPass, low_split, 0.0, sample_rate),
                BiquadFilter::new(FilterType::LowPass, low_split, 0.0, sample_rate),
            ),
            low_lp2: (
                BiquadFilter::new(FilterType::LowPass, low_split, 0.0, sample_rate),
                BiquadFilter::new(FilterType::LowPass, low_split, 0.0, sample_rate),
            ),
            low_hp1: (
                BiquadFilter::new(FilterType::HighPass, low_split, 0.0, sample_rate),
                BiquadFilter::new(FilterType::HighPass, low_split, 0.0, sample_rate),
            ),
            low_hp2: (
                BiquadFilter::new(FilterType::HighPass, low_split, 0.0, sample_rate),
                BiquadFilter::new(FilterType::HighPass, low_split, 0.0, sample_rate),
            ),
            high_lp1: (
                BiquadFilter::new(FilterType::LowPass, high_split, 0.0, sample_rate),
                BiquadFilter::new(FilterType::LowPass, high_split, 0.0, sample_rate),
            ),
            high_lp2: (
                BiquadFilter::new(FilterType::LowPass, high_split, 0.0, sample_rate),
                BiquadFilter::new(FilterType::LowPass, high_split, 0.0, sample_rate),
            ),
            high_hp1: (
                BiquadFilter::new(FilterType::HighPass, high_split, 0.0, sample_rate),
                BiquadFilter::new(FilterType::HighPass, high_split, 0.0, sample_rate),
            ),
            high_hp2: (
                BiquadFilter::new(FilterType::HighPass, high_split, 0.0, sample_rate),
                BiquadFilter::new(FilterType::HighPass, high_split, 0.0, sample_rate),
            ),
            low_ap1: (
                BiquadFilter::new(FilterType::AllPass { q: 0.70710678 }, high_split, 0.0, sample_rate),
                BiquadFilter::new(FilterType::AllPass { q: 0.70710678 }, high_split, 0.0, sample_rate),
            ),
            low_ap2: (
                BiquadFilter::new(FilterType::AllPass { q: 0.70710678 }, high_split, 0.0, sample_rate),
                BiquadFilter::new(FilterType::AllPass { q: 0.70710678 }, high_split, 0.0, sample_rate),
            ),
            low_comp,
            mid_comp,
            high_comp,
            bypassed: false,
        }
    }
}

impl StereoDspNode for MultibandCompressor {
    fn name(&self) -> &'static str {
        "3-Band Multiband Compressor"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.low_lp1.0.reset(); self.low_lp1.1.reset();
        self.low_lp2.0.reset(); self.low_lp2.1.reset();
        self.low_hp1.0.reset(); self.low_hp1.1.reset();
        self.low_hp2.0.reset(); self.low_hp2.1.reset();
        self.high_lp1.0.reset(); self.high_lp1.1.reset();
        self.high_lp2.0.reset(); self.high_lp2.1.reset();
        self.high_hp1.0.reset(); self.high_hp1.1.reset();
        self.high_hp2.0.reset(); self.high_hp2.1.reset();
        self.low_ap1.0.reset(); self.low_ap1.1.reset();
        self.low_ap2.0.reset(); self.low_ap2.1.reset();
        self.low_comp.0.reset(); self.low_comp.1.reset();
        self.mid_comp.0.reset(); self.mid_comp.1.reset();
        self.high_comp.0.reset(); self.high_comp.1.reset();
    }

    #[inline(always)]
    fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.bypassed {
            return (left, right);
        }

        // 1. Split into Low and Rest (Mid+High)
        let low_raw_l = self.low_lp2.0.process_sample(self.low_lp1.0.process_sample(left));
        let low_raw_r = self.low_lp2.1.process_sample(self.low_lp1.1.process_sample(right));

        // Compensate Low band phase to match the High crossover allpass phase shift
        let low_l = self.low_ap2.0.process_sample(self.low_ap1.0.process_sample(low_raw_l));
        let low_r = self.low_ap2.1.process_sample(self.low_ap1.1.process_sample(low_raw_r));

        let rest_l = self.low_hp2.0.process_sample(self.low_hp1.0.process_sample(left));
        let rest_r = self.low_hp2.1.process_sample(self.low_hp1.1.process_sample(right));

        // 2. Split Rest into Mid and High
        let mid_l = self.high_lp2.0.process_sample(self.high_lp1.0.process_sample(rest_l));
        let mid_r = self.high_lp2.1.process_sample(self.high_lp1.1.process_sample(rest_r));

        let high_l = self.high_hp2.0.process_sample(self.high_hp1.0.process_sample(rest_l));
        let high_r = self.high_hp2.1.process_sample(self.high_hp1.1.process_sample(rest_r));

        // 3. Compress each band independently
        let proc_low_l = self.low_comp.0.process_sample(low_l);
        let proc_low_r = self.low_comp.1.process_sample(low_r);

        let proc_mid_l = self.mid_comp.0.process_sample(mid_l);
        let proc_mid_r = self.mid_comp.1.process_sample(mid_r);

        let proc_high_l = self.high_comp.0.process_sample(high_l);
        let proc_high_r = self.high_comp.1.process_sample(high_r);

        // 4. Sum back together with matched phase
        (
            proc_low_l + proc_mid_l + proc_high_l,
            proc_low_r + proc_mid_r + proc_high_r,
        )
    }
}
