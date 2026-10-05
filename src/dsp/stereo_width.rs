//! Stereo Width & Mid-Side Processor with Mono-Bass filtering.

use super::biquad::{BiquadFilter, FilterType};
use super::StereoDspNode;

#[derive(Debug, Clone)]
pub struct StereoWidthMidSide {
    pub width: f32, // 0.0 = mono, 1.0 = normal, 2.0 = ultra-wide
    pub mono_bass_hz: f32,
    mono_bass_filter: BiquadFilter, // High-pass on the Side channel to keep bass mono
    pub bypassed: bool,
}

impl StereoWidthMidSide {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            width: 1.15, // Subtle mastering stereo enhancement
            mono_bass_hz: 90.0,
            mono_bass_filter: BiquadFilter::new(FilterType::HighPass, 90.0, 0.0, sample_rate),
            bypassed: false,
        }
    }

    pub fn set_params(&mut self, width: f32, mono_bass_hz: f32) {
        self.width = width.clamp(0.0, 2.5);
        self.mono_bass_hz = mono_bass_hz.clamp(20.0, 300.0);
        self.mono_bass_filter.set_cutoff(self.mono_bass_hz);
    }
}

impl StereoDspNode for StereoWidthMidSide {
    fn name(&self) -> &'static str {
        "Stereo Width / Mid-Side"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.mono_bass_filter.reset();
    }

    #[inline(always)]
    fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.bypassed {
            return (left, right);
        }

        let inv_sqrt2 = 0.70710678_f32;

        // 1. Mid-Side Encode
        let mid = (left + right) * inv_sqrt2;
        let mut side = (left - right) * inv_sqrt2;

        // 2. High-pass filter on Side channel (keeps low frequencies centered mono)
        side = self.mono_bass_filter.process_sample(side);

        // 3. Apply width factor
        side *= self.width;

        // 4. Mid-Side Decode back to L/R
        let out_l = (mid + side) * inv_sqrt2;
        let out_r = (mid - side) * inv_sqrt2;

        (out_l, out_r)
    }
}
