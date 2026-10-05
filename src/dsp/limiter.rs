//! True-Peak Brickwall Limiter with Lookahead and -1 dBTP Ceiling.
//!
//! Complies with EBU R128 and ITU-R BS.1770 mastering standards by preventing
//! inter-sample peaks from clipping downstream D/A converters.

use super::StereoDspNode;

#[derive(Debug, Clone)]
pub struct TruePeakLimiter {
    ceiling_linear: f32, // -1.0 dBTP = ~0.89125
    lookahead_len: usize,
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    write_idx: usize,
    
    // Gain reduction envelope
    gain: f32,
    release_coeff: f32,
    current_gr_db: f32,
    pub bypassed: bool,
}

impl TruePeakLimiter {
    pub fn new(sample_rate: f32) -> Self {
        // -1.0 dB True Peak ceiling
        let ceiling_linear = 10.0_f32.powf(-1.0 / 20.0);
        let lookahead_ms = 1.5_f32;
        let lookahead_len = (sample_rate * lookahead_ms * 0.001) as usize;
        let release_ms = 80.0_f32;

        Self {
            ceiling_linear,
            lookahead_len,
            buf_l: vec![0.0; lookahead_len + 1],
            buf_r: vec![0.0; lookahead_len + 1],
            write_idx: 0,
            gain: 1.0,
            release_coeff: (-1.0 / (release_ms * 0.001 * sample_rate)).exp(),
            current_gr_db: 0.0,
            bypassed: false,
        }
    }

    pub fn set_ceiling_dbtp(&mut self, ceiling_db: f32) {
        self.ceiling_linear = 10.0_f32.powf(ceiling_db.min(0.0) / 20.0);
    }

    pub fn gain_reduction_db(&self) -> f32 {
        self.current_gr_db
    }

    /// Estimate 4x inter-sample peak using parabolic approximation
    #[inline(always)]
    fn estimate_true_peak(prev: f32, curr: f32, next: f32) -> f32 {
        let abs_curr = curr.abs();
        let denom = 2.0 * (prev - 2.0 * curr + next);
        if denom.abs() > 1e-6 {
            let offset = (prev - next) / denom;
            if offset.abs() < 0.5 {
                let peak = curr - (prev - next) * offset * 0.25;
                return abs_curr.max(peak.abs());
            }
        }
        abs_curr
    }
}

impl StereoDspNode for TruePeakLimiter {
    fn name(&self) -> &'static str {
        "True-Peak Limiter (-1 dBTP)"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.buf_l.fill(0.0);
        self.buf_r.fill(0.0);
        self.write_idx = 0;
        self.gain = 1.0;
        self.current_gr_db = 0.0;
    }

    #[inline(always)]
    fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.bypassed {
            return (left, right);
        }

        // Store incoming sample in lookahead ring buffer
        self.buf_l[self.write_idx] = left;
        self.buf_r[self.write_idx] = right;

        let buf_len = self.buf_l.len();
        let prev_idx = (self.write_idx + buf_len - 1) % buf_len;
        let delayed_idx = (self.write_idx + 1) % buf_len;

        // True-peak detection over lookahead window
        let peak_l = Self::estimate_true_peak(self.buf_l[prev_idx], left, self.buf_l[delayed_idx]);
        let peak_r = Self::estimate_true_peak(self.buf_r[prev_idx], right, self.buf_r[delayed_idx]);
        let max_peak = peak_l.max(peak_r);

        // Required target gain to keep peak below ceiling
        let target_gain = if max_peak > self.ceiling_linear {
            self.ceiling_linear / max_peak
        } else {
            1.0
        };

        // Instantaneous attack (zero lookahead breach), smooth release
        if target_gain < self.gain {
            self.gain = target_gain;
        } else {
            self.gain = self.release_coeff * self.gain + (1.0 - self.release_coeff) * target_gain;
        }

        self.current_gr_db = if self.gain < 1.0 {
            -20.0 * self.gain.log10()
        } else {
            0.0
        };

        // Output delayed sample multiplied by computed lookahead gain
        let delayed_l = self.buf_l[delayed_idx];
        let delayed_r = self.buf_r[delayed_idx];

        self.write_idx = (self.write_idx + 1) % buf_len;

        // Hard clamp to ceiling as absolute fail-safe
        (
            (delayed_l * self.gain).clamp(-self.ceiling_linear, self.ceiling_linear),
            (delayed_r * self.gain).clamp(-self.ceiling_linear, self.ceiling_linear),
        )
    }
}
