//! Real-time Monophonic Time-Domain Pitch Shifter.
//! Uses flat-top dual-tap grain synthesis with raised-cosine crossfading
//! and 4-point cubic Hermite interpolation. Eliminates comb filtering
//! and boundary click artifacts for transparent vocal pitch modification.

#![allow(dead_code)]

#[derive(Debug, Clone)]
pub struct PitchShifter {
    sample_rate: f32,
    buffer: Vec<f32>,
    buf_size: usize,
    write_idx: usize,

    // Master normalized grain phase [0.0, 1.0)
    phi: f32,
    grain_len: f32,
    base_delay: f32,

    // Pitch ratio tracking and smoothing
    current_ratio: f32,
    target_ratio: f32,
    smoothing_coeff: f32,
}

impl PitchShifter {
    pub fn new(sample_rate: f32) -> Self {
        let buf_size = 8192;
        // ~25ms optimal grain window for transparent vocal formants
        let grain_len = (sample_rate * 0.025).clamp(800.0, 1600.0);
        Self {
            sample_rate,
            buffer: vec![0.0; buf_size],
            buf_size,
            write_idx: 0,
            phi: 0.35, // Initialize safely in the single-tap flat region
            grain_len,
            base_delay: 512.0, // Fixed lookahead headroom (~10.7ms at 48kHz)
            current_ratio: 1.0,
            target_ratio: 1.0,
            smoothing_coeff: 0.05,
        }
    }

    pub fn set_ratio(&mut self, ratio: f32, retune_speed_ms: f32) {
        // Clamp pitch shift ratio to musical vocal correction range (+/- 1 octave)
        self.target_ratio = ratio.clamp(0.5, 2.0);

        let speed = retune_speed_ms.max(0.5);
        self.smoothing_coeff = 1.0 - (-1.0 / (0.001 * speed * self.sample_rate)).exp();
    }

    /// Sets the pitch period in samples (kept for API compatibility).
    pub fn set_pitch_period(&mut self, _period_samples: f32) {
        // Optimal fixed vocal grain size avoids granular flutter and wrap clicks.
    }

    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_idx = 0;
        self.phi = 0.35;
        self.current_ratio = 1.0;
        self.target_ratio = 1.0;
    }

    /// 4-point, 3rd-order cubic Hermite interpolation for transparent delay line reading.
    #[inline(always)]
    fn read_cubic(&self, delay_samples: f32) -> f32 {
        let delay = delay_samples.clamp(2.0, (self.buf_size - 4) as f32);
        let read_pos = self.write_idx as f32 + self.buf_size as f32 - delay;
        let idx1 = (read_pos.floor() as usize) & (self.buf_size - 1);
        let frac = read_pos - read_pos.floor();

        let idx0 = (idx1 + self.buf_size - 1) & (self.buf_size - 1);
        let idx2 = (idx1 + 1) & (self.buf_size - 1);
        let idx3 = (idx1 + 2) & (self.buf_size - 1);

        let y0 = self.buffer[idx0];
        let y1 = self.buffer[idx1];
        let y2 = self.buffer[idx2];
        let y3 = self.buffer[idx3];

        let c0 = y1;
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);

        ((c3 * frac + c2) * frac + c1) * frac + c0
    }

    #[inline(always)]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        self.buffer[self.write_idx] = input;
        self.write_idx = (self.write_idx + 1) & (self.buf_size - 1);

        // Exponential smoothing of the pitch ratio
        self.current_ratio += (self.target_ratio - self.current_ratio) * self.smoothing_coeff;

        // Pitch shift delta: dPhi/dt = (1 - ratio) / grain_len
        let ratio_dev = 1.0 - self.current_ratio;
        if ratio_dev.abs() > 0.001 {
            let d_phi = ratio_dev / self.grain_len;
            self.phi += d_phi;
            if self.phi >= 1.0 {
                self.phi -= 1.0;
            } else if self.phi < 0.0 {
                self.phi += 1.0;
            }
        } else {
            // When ratio is 1.0 (on pitch / unvoiced), smoothly guide phi to 0.35
            // where Tap 1 is 100% active and Tap 2 is 0% active (zero comb filtering).
            let target_idle_phi = 0.35_f32;
            self.phi += (target_idle_phi - self.phi) * 0.005;
        }

        // Tap 1 phase is phi; Tap 2 phase is (phi + 0.5) % 1.0
        let phi1 = self.phi;
        let phi2 = if self.phi < 0.5 { self.phi + 0.5 } else { self.phi - 0.5 };

        // Flat-top window with raised-cosine crossfade.
        // Crossfade zone width = 0.10 (10% of half-cycle)
        const XFADE_WIDTH: f32 = 0.10;
        let (w1, w2) = if self.phi < 0.5 {
            if self.phi < XFADE_WIDTH {
                let frac = self.phi / XFADE_WIDTH;
                let s = (frac * std::f32::consts::FRAC_PI_2).sin();
                (s * s, 1.0 - s * s)
            } else {
                (1.0, 0.0)
            }
        } else {
            let psi = self.phi - 0.5;
            if psi < XFADE_WIDTH {
                let frac = psi / XFADE_WIDTH;
                let s = (frac * std::f32::consts::FRAC_PI_2).sin();
                (1.0 - s * s, s * s)
            } else {
                (0.0, 1.0)
            }
        };

        let delay1 = self.base_delay + phi1 * self.grain_len;
        let delay2 = self.base_delay + phi2 * self.grain_len;

        if w1 >= 0.9999 {
            self.read_cubic(delay1)
        } else if w2 >= 0.9999 {
            self.read_cubic(delay2)
        } else {
            let s1 = self.read_cubic(delay1);
            let s2 = self.read_cubic(delay2);
            s1 * w1 + s2 * w2
        }
    }
}
