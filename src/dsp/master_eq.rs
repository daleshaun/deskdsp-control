//! 5-Band Minimum-Phase Mastering Equalizer.

use super::biquad::{BiquadFilter, FilterType};
use super::StereoDspNode;

#[derive(Debug, Clone)]
pub struct MasterEq {
    pub low_cut: (BiquadFilter, BiquadFilter),
    pub low_shelf: (BiquadFilter, BiquadFilter),
    pub low_mid: (BiquadFilter, BiquadFilter),
    pub high_mid: (BiquadFilter, BiquadFilter),
    pub high_shelf: (BiquadFilter, BiquadFilter),
    pub bypassed: bool,
}

impl MasterEq {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            low_cut: (
                BiquadFilter::new(FilterType::HighPass, 25.0, 0.0, sample_rate),
                BiquadFilter::new(FilterType::HighPass, 25.0, 0.0, sample_rate),
            ),
            low_shelf: (
                BiquadFilter::new(FilterType::LowShelf { q: 0.707 }, 80.0, 0.0, sample_rate),
                BiquadFilter::new(FilterType::LowShelf { q: 0.707 }, 80.0, 0.0, sample_rate),
            ),
            low_mid: (
                BiquadFilter::new(FilterType::Peaking { q: 0.8 }, 400.0, 0.0, sample_rate),
                BiquadFilter::new(FilterType::Peaking { q: 0.8 }, 400.0, 0.0, sample_rate),
            ),
            high_mid: (
                BiquadFilter::new(FilterType::Peaking { q: 0.9 }, 3000.0, 0.0, sample_rate),
                BiquadFilter::new(FilterType::Peaking { q: 0.9 }, 3000.0, 0.0, sample_rate),
            ),
            high_shelf: (
                BiquadFilter::new(FilterType::HighShelf { q: 0.707 }, 12000.0, 0.0, sample_rate),
                BiquadFilter::new(FilterType::HighShelf { q: 0.707 }, 12000.0, 0.0, sample_rate),
            ),
            bypassed: false,
        }
    }
}

impl StereoDspNode for MasterEq {
    fn name(&self) -> &'static str {
        "5-Band Master EQ"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.low_cut.0.reset();
        self.low_cut.1.reset();
        self.low_shelf.0.reset();
        self.low_shelf.1.reset();
        self.low_mid.0.reset();
        self.low_mid.1.reset();
        self.high_mid.0.reset();
        self.high_mid.1.reset();
        self.high_shelf.0.reset();
        self.high_shelf.1.reset();
    }

    #[inline(always)]
    fn process_stereo(&mut self, mut left: f32, mut right: f32) -> (f32, f32) {
        if self.bypassed {
            return (left, right);
        }

        left = self.low_cut.0.process_sample(left);
        right = self.low_cut.1.process_sample(right);

        left = self.low_shelf.0.process_sample(left);
        right = self.low_shelf.1.process_sample(right);

        left = self.low_mid.0.process_sample(left);
        right = self.low_mid.1.process_sample(right);

        left = self.high_mid.0.process_sample(left);
        right = self.high_mid.1.process_sample(right);

        left = self.high_shelf.0.process_sample(left);
        right = self.high_shelf.1.process_sample(right);

        (left, right)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
