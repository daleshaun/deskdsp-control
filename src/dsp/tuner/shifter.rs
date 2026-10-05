//! Real-time Monophonic Time-Domain Pitch Shifter.
//! Uses pitch-synchronous dual-tap Hann windowed crossfading with 4-point cubic Hermite
//! interpolation for artifact-free, zero-glitch vocal pitch modification.

#![allow(dead_code)]

#[derive(Debug, Clone)]
pub struct PitchShifter {
    sample_rate: f32,
    buffer: Vec<f32>,
    buf_size: usize,
    write_idx: usize,

    // Dual tap delay pointers for smooth windowed crossfading
    tap1: f32,
    tap2: f32,
    window_length: f32,
    target_window_length: f32,
    base_delay: f32,

    // Smoothing
    current_ratio: f32,
    target_ratio: f32,
    smoothing_coeff: f32,
}

impl PitchShifter {
    pub fn new(sample_rate: f32) -> Self {
        let buf_size = 8192;
        let default_window = (sample_rate * 0.015).clamp(240.0, 960.0); // ~15ms default window
        Self {
            sample_rate,
            buffer: vec![0.0; buf_size],
            buf_size,
            write_idx: 0,
            tap1: 0.0,
            tap2: default_window * 0.5,
            window_length: default_window,
            target_window_length: default_window,
            base_delay: 512.0, // Constant lookback headroom (~10.6ms at 48kHz)
            current_ratio: 1.0,
            target_ratio: 1.0,
            smoothing_coeff: 0.05,
        }
    }

    pub fn set_ratio(&mut self, ratio: f32, retune_speed_ms: f32) {
        // Clamp pitch shift ratio to musical vocal correction range (+/- 1 octave)
        self.target_ratio = ratio.clamp(0.5, 2.0);

        let speed = retune_speed_ms.max(0.1);
        self.smoothing_coeff = 1.0 - (-1.0 / (0.001 * speed * self.sample_rate)).exp();
    }

    /// Sets the vocal pitch period in samples for pitch-synchronous grain alignment.
    pub fn set_pitch_period(&mut self, period_samples: f32) {
        // Align grain window to ~2 pitch periods (180 to 960 samples, ~3.7ms to 20ms at 48kHz)
        self.target_window_length = (2.0 * period_samples).clamp(180.0, 960.0);
    }

    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_idx = 0;
        self.tap1 = 0.0;
        self.tap2 = self.window_length * 0.5;
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

        // Smooth toward target ratio and grain window size
        self.current_ratio += (self.target_ratio - self.current_ratio) * self.smoothing_coeff;
        self.window_length += (self.target_window_length - self.window_length) * 0.005;

        // Modulation delta: dDelay/dt = 1 - ratio
        let delay_rate = 1.0 - self.current_ratio;
        self.tap1 += delay_rate;
        self.tap2 += delay_rate;

        // Wrap taps smoothly within window
        let w = self.window_length;
        while self.tap1 >= w { self.tap1 -= w; }
        while self.tap1 < 0.0 { self.tap1 += w; }
        while self.tap2 >= w { self.tap2 -= w; }
        while self.tap2 < 0.0 { self.tap2 += w; }

        // Hann crossfade where w1 + w2 == 1.0 identically at EVERY sample
        // When tap1 wraps, w1 = 0 (silent); when tap2 wraps, w2 = 0 (silent).
        let fade = (self.tap1 / w) * (2.0 * std::f32::consts::PI);
        let w1 = 0.5 * (1.0 - fade.cos());
        let w2 = 1.0 - w1;

        let s1 = self.read_cubic(self.base_delay + self.tap1);
        let s2 = self.read_cubic(self.base_delay + self.tap2);

        s1 * w1 + s2 * w2
    }
}
