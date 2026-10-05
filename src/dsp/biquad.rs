//! Standard RBJ Audio EQ Cookbook Biquad Filter Implementation.
//! Implements Direct Form II Transposed structure for numerical stability.

use super::DspNode;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterType {
    HighPass,
    LowPass,
    BandPass { q: f32 },
    Notch { q: f32 },
    Peaking { q: f32 },
    LowShelf { q: f32 },
    HighShelf { q: f32 },
}

#[derive(Debug, Clone)]
pub struct BiquadFilter {
    pub b0: f32,
    pub b1: f32,
    pub b2: f32,
    pub a1: f32,
    pub a2: f32,
    s1: f32,
    s2: f32,
    pub sample_rate: f32,
    pub cutoff: f32,
    pub gain_db: f32,
    pub filter_type: FilterType,
    pub bypassed: bool,
}

impl BiquadFilter {
    pub fn new(filter_type: FilterType, cutoff: f32, gain_db: f32, sample_rate: f32) -> Self {
        let mut filter = Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            s1: 0.0,
            s2: 0.0,
            sample_rate,
            cutoff,
            gain_db,
            filter_type,
            bypassed: false,
        };
        filter.recalculate();
        filter
    }

    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    #[inline(always)]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }
        let output = self.b0 * input + self.s1;
        self.s1 = self.b1 * input - self.a1 * output + self.s2;
        self.s2 = self.b2 * input - self.a2 * output;
        output
    }

    pub fn set_cutoff(&mut self, cutoff: f32) {
        self.cutoff = cutoff.clamp(10.0, self.sample_rate * 0.49);
        self.recalculate();
    }

    pub fn set_gain_db(&mut self, gain_db: f32) {
        self.gain_db = gain_db.clamp(-36.0, 36.0);
        self.recalculate();
    }

    pub fn recalculate(&mut self) {
        let omega = 2.0 * std::f32::consts::PI * self.cutoff / self.sample_rate;
        let sn = omega.sin();
        let cs = omega.cos();
        let a = 10.0_f32.powf(self.gain_db / 40.0);

        match self.filter_type {
            FilterType::HighPass => {
                let q = 0.70710678; // Butterworth Q
                let alpha = sn / (2.0 * q);
                let a0 = 1.0 + alpha;
                self.b0 = ((1.0 + cs) / 2.0) / a0;
                self.b1 = (-(1.0 + cs)) / a0;
                self.b2 = ((1.0 + cs) / 2.0) / a0;
                self.a1 = (-2.0 * cs) / a0;
                self.a2 = (1.0 - alpha) / a0;
            }
            FilterType::LowPass => {
                let q = 0.70710678;
                let alpha = sn / (2.0 * q);
                let a0 = 1.0 + alpha;
                self.b0 = ((1.0 - cs) / 2.0) / a0;
                self.b1 = (1.0 - cs) / a0;
                self.b2 = ((1.0 - cs) / 2.0) / a0;
                self.a1 = (-2.0 * cs) / a0;
                self.a2 = (1.0 - alpha) / a0;
            }
            FilterType::BandPass { q } => {
                let alpha = sn / (2.0 * q.max(0.01));
                let a0 = 1.0 + alpha;
                self.b0 = (sn / 2.0) / a0;
                self.b1 = 0.0;
                self.b2 = (-sn / 2.0) / a0;
                self.a1 = (-2.0 * cs) / a0;
                self.a2 = (1.0 - alpha) / a0;
            }
            FilterType::Notch { q } => {
                let alpha = sn / (2.0 * q.max(0.01));
                let a0 = 1.0 + alpha;
                self.b0 = 1.0 / a0;
                self.b1 = (-2.0 * cs) / a0;
                self.b2 = 1.0 / a0;
                self.a1 = (-2.0 * cs) / a0;
                self.a2 = (1.0 - alpha) / a0;
            }
            FilterType::Peaking { q } => {
                let alpha = sn / (2.0 * q.max(0.01));
                let a0 = 1.0 + alpha / a;
                self.b0 = (1.0 + alpha * a) / a0;
                self.b1 = (-2.0 * cs) / a0;
                self.b2 = (1.0 - alpha * a) / a0;
                self.a1 = (-2.0 * cs) / a0;
                self.a2 = (1.0 - alpha / a) / a0;
            }
            FilterType::LowShelf { q } => {
                let beta = (a + 1.0 / a).sqrt();
                let alpha = sn / 2.0 * beta / q.max(0.01);
                let ap1 = a + 1.0;
                let am1 = a - 1.0;
                let a0 = ap1 + am1 * cs + 2.0 * a.sqrt() * alpha;
                self.b0 = (a * (ap1 - am1 * cs + 2.0 * a.sqrt() * alpha)) / a0;
                self.b1 = (2.0 * a * (am1 - ap1 * cs)) / a0;
                self.b2 = (a * (ap1 - am1 * cs - 2.0 * a.sqrt() * alpha)) / a0;
                self.a1 = (-2.0 * (am1 + ap1 * cs)) / a0;
                self.a2 = (ap1 + am1 * cs - 2.0 * a.sqrt() * alpha) / a0;
            }
            FilterType::HighShelf { q } => {
                let beta = (a + 1.0 / a).sqrt();
                let alpha = sn / 2.0 * beta / q.max(0.01);
                let ap1 = a + 1.0;
                let am1 = a - 1.0;
                let a0 = ap1 - am1 * cs + 2.0 * a.sqrt() * alpha;
                self.b0 = (a * (ap1 + am1 * cs + 2.0 * a.sqrt() * alpha)) / a0;
                self.b1 = (-2.0 * a * (am1 + ap1 * cs)) / a0;
                self.b2 = (a * (ap1 + am1 * cs - 2.0 * a.sqrt() * alpha)) / a0;
                self.a1 = (2.0 * (am1 - ap1 * cs)) / a0;
                self.a2 = (ap1 - am1 * cs - 2.0 * a.sqrt() * alpha) / a0;
            }
        }
    }
}

impl DspNode for BiquadFilter {
    fn name(&self) -> &'static str {
        match self.filter_type {
            FilterType::HighPass => "High-Pass Filter",
            FilterType::LowPass => "Low-Pass Filter",
            FilterType::BandPass { .. } => "Band-Pass Filter",
            FilterType::Notch { .. } => "Notch Filter",
            FilterType::Peaking { .. } => "Peaking EQ Band",
            FilterType::LowShelf { .. } => "Low-Shelf EQ",
            FilterType::HighShelf { .. } => "High-Shelf EQ",
        }
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }
        let output = self.b0 * input + self.s1;
        self.s1 = self.b1 * input - self.a1 * output + self.s2;
        self.s2 = self.b2 * input - self.a2 * output;
        output
    }
}
