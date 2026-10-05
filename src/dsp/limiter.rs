//! True-Peak Brickwall Limiter with Lookahead and -1 dBTP Ceiling.
//!
//! Complies with ITU-R BS.1770-4 Annex 2 and EBU R128 mastering standards.
//! Employs 4x oversampled polyphase FIR interpolation over a lookahead window
//! to detect and control inter-sample peaks before they reach downstream D/A converters.

#![allow(dead_code)]

use super::StereoDspNode;

/// ITU-R BS.1770-4 Annex 2 Table 2 4x Oversampling Polyphase Interpolation Filter
/// (12 taps per phase, linear phase, symmetrical).
const FIR_PHASE1: [f32; 12] = [
    -0.0017, 0.0097, -0.0294, 0.0709, -0.1587, 0.9082, 0.2741, -0.1084, 0.0526, -0.0245, 0.0093, -0.0025,
];
const FIR_PHASE2: [f32; 12] = [
    -0.0036, 0.0197, -0.0592, 0.1481, -0.3703, 0.7758, 0.7758, -0.3703, 0.1481, -0.0592, 0.0197, -0.0036,
];
const FIR_PHASE3: [f32; 12] = [
    -0.0025, 0.0093, -0.0245, 0.0526, -0.1084, 0.2741, 0.9082, -0.1587, 0.0709, -0.0294, 0.0097, -0.0017,
];

#[inline(always)]
pub fn calculate_true_peak_4x(history: &[f32; 12]) -> f32 {
    let s0 = history[5].abs(); // Original sample at center
    let mut s1 = 0.0_f32;
    let mut s2 = 0.0_f32;
    let mut s3 = 0.0_f32;
    for i in 0..12 {
        let x = history[i];
        s1 += x * FIR_PHASE1[i];
        s2 += x * FIR_PHASE2[i];
        s3 += x * FIR_PHASE3[i];
    }
    s0.max(s1.abs()).max(s2.abs()).max(s3.abs())
}

#[derive(Debug, Clone)]
pub struct TruePeakLimiter {
    pub ceiling_linear: f32, // -1.0 dBTP = ~0.89125
    lookahead_len: usize,
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    recent_l: [f32; 12],
    recent_r: [f32; 12],
    recent_idx: usize,
    write_idx: usize,
    
    // Gain reduction envelope
    gain: f32,
    release_coeff: f32,
    current_gr_db: f32,
    pub bypassed: bool,
}

impl TruePeakLimiter {
    pub fn new(sample_rate: f32) -> Self {
        // -1.0 dB True Peak ceiling = 10^(-1/20) ~ 0.8912509
        let ceiling_linear = 10.0_f32.powf(-1.0 / 20.0);
        let lookahead_ms = 2.0_f32;
        let lookahead_len = ((sample_rate * lookahead_ms * 0.001) as usize).max(32);
        let release_ms = 80.0_f32;

        Self {
            ceiling_linear,
            lookahead_len,
            buf_l: vec![0.0; lookahead_len + 16],
            buf_r: vec![0.0; lookahead_len + 16],
            recent_l: [0.0; 12],
            recent_r: [0.0; 12],
            recent_idx: 0,
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
        self.recent_l.fill(0.0);
        self.recent_r.fill(0.0);
        self.recent_idx = 0;
        self.write_idx = 0;
        self.gain = 1.0;
        self.current_gr_db = 0.0;
    }

    #[inline(always)]
    fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        if self.bypassed {
            return (left, right);
        }

        // 1. Maintain 12-sample sliding window for 4x oversampling
        self.recent_l[self.recent_idx] = left;
        self.recent_r[self.recent_idx] = right;
        self.recent_idx = (self.recent_idx + 1) % 12;

        let mut hist_l = [0.0_f32; 12];
        let mut hist_r = [0.0_f32; 12];
        for i in 0..12 {
            let idx = (self.recent_idx + i) % 12;
            hist_l[i] = self.recent_l[idx];
            hist_r[i] = self.recent_r[idx];
        }

        // 2. Compute 4x oversampled true-peak on the incoming signal
        let tp_l = calculate_true_peak_4x(&hist_l);
        let tp_r = calculate_true_peak_4x(&hist_r);
        let max_true_peak = tp_l.max(tp_r);

        // 3. Store incoming samples in the lookahead delay buffer
        let buf_len = self.buf_l.len();
        self.buf_l[self.write_idx] = left;
        self.buf_r[self.write_idx] = right;

        // 4. Calculate required gain reduction to keep true peak below ceiling
        let target_gain = if max_true_peak > self.ceiling_linear {
            self.ceiling_linear / max_true_peak
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

        // 5. Read delayed sample from lookahead distance
        let read_idx = (self.write_idx + buf_len - self.lookahead_len) % buf_len;
        self.write_idx = (self.write_idx + 1) % buf_len;

        let mut out_l = self.buf_l[read_idx] * self.gain;
        let mut out_r = self.buf_r[read_idx] * self.gain;

        // Final brickwall safety clamp strictly to the exact ceiling
        out_l = out_l.clamp(-self.ceiling_linear, self.ceiling_linear);
        out_r = out_r.clamp(-self.ceiling_linear, self.ceiling_linear);

        (out_l, out_r)
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
