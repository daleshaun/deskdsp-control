//! Real-time Vocal Pitch Detector using YIN difference function with parabolic interpolation
//! and octave-jump suppression for solid, natural pitch tracking down to 65 Hz.

#[derive(Debug, Clone)]
pub struct PitchDetector {
    sample_rate: f32,
    window_size: usize,
    min_period: usize,
    max_period: usize,
    threshold: f32,
    buffer: Vec<f32>,
    write_pos: usize,
    diff_buf: Vec<f32>,
    last_stable_period: Option<f32>,
}

impl PitchDetector {
    pub fn new(sample_rate: f32) -> Self {
        // Target full vocal range: 65 Hz (C2) to 900 Hz (A5)
        let window_size = 2048;
        let min_period = (sample_rate / 900.0).max(10.0) as usize;
        let max_period = (sample_rate / 65.0).min(1000.0) as usize;

        Self {
            sample_rate,
            window_size,
            min_period,
            max_period,
            threshold: 0.15, // Standard YIN confidence threshold
            buffer: vec![0.0; window_size],
            write_pos: 0,
            diff_buf: vec![0.0; max_period + 2],
            last_stable_period: None,
        }
    }

    #[inline(always)]
    pub fn push_sample(&mut self, sample: f32) {
        self.buffer[self.write_pos] = sample;
        self.write_pos = (self.write_pos + 1) % self.window_size;
    }

    /// Reset pitch tracking history
    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
        self.diff_buf.fill(0.0);
        self.last_stable_period = None;
    }

    /// Detect fundamental pitch (F0 in Hz) and confidence (0.0 to 1.0).
    pub fn detect_pitch(&mut self) -> (Option<f32>, f32) {
        let half_w = self.window_size / 2; // 1024 samples
        let mut running_sum = 0.0_f32;

        self.diff_buf[0] = 1.0;

        // Step 1: Difference Function
        for tau in 1..=self.max_period {
            let mut sum = 0.0_f32;
            for j in 0..half_w {
                let idx1 = (self.write_pos + self.window_size - half_w + j) % self.window_size;
                let idx2 = (self.write_pos + self.window_size - half_w + j - tau) % self.window_size;
                let diff = self.buffer[idx1] - self.buffer[idx2];
                sum += diff * diff;
            }

            // Step 2: Cumulative Mean Normalized Difference
            running_sum += sum;
            self.diff_buf[tau] = if running_sum > 1e-6 {
                sum * (tau as f32) / running_sum
            } else {
                1.0
            };
        }

        // Step 3: Absolute Thresholding
        let mut best_tau = 0;
        for tau in self.min_period..=self.max_period {
            if self.diff_buf[tau] < self.threshold {
                // Find local minimum
                let mut candidate = tau;
                while candidate + 1 <= self.max_period && self.diff_buf[candidate + 1] < self.diff_buf[candidate] {
                    candidate += 1;
                }
                best_tau = candidate;
                break;
            }
        }

        if best_tau == 0 {
            // Global minimum fallback if nothing under threshold
            let mut min_val = 1.0_f32;
            for tau in self.min_period..=self.max_period {
                if self.diff_buf[tau] < min_val {
                    min_val = self.diff_buf[tau];
                    best_tau = tau;
                }
            }
            if min_val > 0.35 {
                // Unvoiced / noise
                return (None, 0.0);
            }
        }

        // Octave continuity check: avoid jumping by an octave when previous pitch is known
        if let Some(last_p) = self.last_stable_period {
            let tau_ratio = best_tau as f32 / last_p;
            // If candidate is an octave higher (tau halved), check if full tau is also good
            if (tau_ratio - 0.5).abs() < 0.12 {
                let full_tau = (last_p.round() as usize).clamp(self.min_period, self.max_period);
                if self.diff_buf[full_tau] < 0.25 {
                    best_tau = full_tau;
                }
            }
            // If candidate is an octave lower (tau doubled), check if half tau was good
            else if (tau_ratio - 2.0).abs() < 0.20 {
                let half_tau = ((last_p * 0.5).round() as usize).clamp(self.min_period, self.max_period);
                if self.diff_buf[half_tau] < 0.20 {
                    best_tau = half_tau;
                }
            }
        }

        // Step 4: Parabolic Interpolation for exact fractional period
        let tau_f = if best_tau > self.min_period && best_tau < self.max_period {
            let s0 = self.diff_buf[best_tau - 1];
            let s1 = self.diff_buf[best_tau];
            let s2 = self.diff_buf[best_tau + 1];
            let denom = 2.0 * (s0 - 2.0 * s1 + s2);
            if denom.abs() > 1e-6 {
                let delta = (s0 - s2) / denom;
                best_tau as f32 + delta.clamp(-0.5, 0.5)
            } else {
                best_tau as f32
            }
        } else {
            best_tau as f32
        };

        if tau_f > 0.0 {
            let freq = self.sample_rate / tau_f;
            let confidence = (1.0 - self.diff_buf[best_tau]).clamp(0.0, 1.0);
            if confidence >= 0.70 {
                self.last_stable_period = Some(tau_f);
            }
            (Some(freq), confidence)
        } else {
            (None, 0.0)
        }
    }
}
