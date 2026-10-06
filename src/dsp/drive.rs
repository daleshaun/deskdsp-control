//! Analog Overdrive & Bass Grit with Tilt Tone and Clean Blend.
//! All processing is allocation-free in the audio thread.

#![allow(dead_code)]

use super::biquad::{BiquadFilter, FilterType};
use super::dc_blocker::DcBlocker;
use super::DspNode;

#[derive(Debug, Clone)]
pub struct DriveNode {
    pub drive: f32,
    pub tone: f32, // -1.0 (warm/bassy) to +1.0 (bright/cutting)
    pub blend: f32, // Clean/dirty blend (0.0 to 1.0)
    pub level_db: f32,
    pub bypassed: bool,

    tilt_low: BiquadFilter,
    tilt_high: BiquadFilter,
    dc_blocker: DcBlocker,
}

impl DriveNode {
    pub fn new(sample_rate: f32) -> Self {
        let mut node = Self {
            drive: 2.5,
            tone: 0.0,
            blend: 0.85,
            level_db: -2.0,
            bypassed: false,
            tilt_low: BiquadFilter::new(FilterType::LowShelf { q: 0.707 }, 300.0, 0.0, sample_rate),
            tilt_high: BiquadFilter::new(FilterType::HighShelf { q: 0.707 }, 2000.0, 0.0, sample_rate),
            dc_blocker: DcBlocker::new(sample_rate),
        };
        node.apply_tone();
        node
    }

    pub fn set_params(&mut self, drive: f32, tone: f32, blend: f32, level_db: f32) {
        self.drive = drive.clamp(1.0, 20.0);
        self.tone = tone.clamp(-1.0, 1.0);
        self.blend = blend.clamp(0.0, 1.0);
        self.level_db = level_db.clamp(-24.0, 12.0);
        self.apply_tone();
    }

    fn apply_tone(&mut self) {
        // Tilt EQ: tone > 0 boosts highs & cuts lows; tone < 0 boosts lows & cuts highs
        let high_gain = self.tone * 6.0;
        let low_gain = -self.tone * 6.0;
        self.tilt_low.set_gain_db(low_gain);
        self.tilt_high.set_gain_db(high_gain);
    }
}

impl DspNode for DriveNode {
    fn name(&self) -> &'static str {
        "Analog Overdrive"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.tilt_low.reset();
        self.tilt_high.reset();
        self.dc_blocker.reset();
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }

        if !input.is_finite() {
            return 0.0;
        }

        let clean = input;

        // Asymmetric warm soft-clipping
        let biased = input * self.drive + 0.12;
        let clipped = if biased > 0.0 {
            biased.tanh() - 0.12_f32.tanh()
        } else {
            let v = biased * 0.85;
            (v / (1.0 + v * v).sqrt()) - 0.12_f32.tanh()
        };

        // Remove DC
        let mut dirty = self.dc_blocker.process_sample(clipped);

        // Tilt tone shaping
        dirty = self.tilt_low.process_sample(dirty);
        dirty = self.tilt_high.process_sample(dirty);

        // Clean / dirty parallel blend
        let mixed = clean * (1.0 - self.blend) + dirty * self.blend;

        let linear_gain = 10.0_f32.powf(self.level_db / 20.0);
        (mixed * linear_gain).clamp(-1.0, 1.0)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
