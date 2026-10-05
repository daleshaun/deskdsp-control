//! Real-time Monophonic Time-Domain Pitch Shifter (2-Tap Granular / WSOLA Windowed Delay Line).
//! Uses dual windowed crossfading taps for low-latency real-time pitch modification.

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
    phase: f32,
    
    // Smoothing
    current_ratio: f32,
    target_ratio: f32,
    smoothing_coeff: f32,
}

impl PitchShifter {
    pub fn new(sample_rate: f32) -> Self {
        let buf_size = 4096;
        let window_length = sample_rate * 0.035; // ~35ms pitch grain window
        Self {
            sample_rate,
            buffer: vec![0.0; buf_size],
            buf_size,
            write_idx: 0,
            tap1: 0.0,
            tap2: window_length * 0.5,
            window_length,
            phase: 0.0,
            current_ratio: 1.0,
            target_ratio: 1.0,
            smoothing_coeff: 0.05,
        }
    }

    pub fn set_ratio(&mut self, ratio: f32, retune_speed_ms: f32) {
        // Clamp pitch shift ratio to +/- 1 octave (0.5x to 2.0x)
        self.target_ratio = ratio.clamp(0.5, 2.0);
        
        let speed = retune_speed_ms.max(0.1);
        self.smoothing_coeff = 1.0 - (-1.0 / (0.001 * speed * self.sample_rate)).exp();
    }

    pub fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_idx = 0;
        self.tap1 = 0.0;
        self.tap2 = self.window_length * 0.5;
        self.phase = 0.0;
        self.current_ratio = 1.0;
        self.target_ratio = 1.0;
    }

    #[inline(always)]
    fn read_fractional(&self, delay_samples: f32) -> f32 {
        let delay = delay_samples.clamp(1.0, (self.buf_size - 4) as f32);
        let read_pos = (self.write_idx as f32 + self.buf_size as f32 - delay) % self.buf_size as f32;
        let idx0 = read_pos.floor() as usize;
        let frac = read_pos - idx0 as f32;
        let idx1 = (idx0 + 1) % self.buf_size;

        // Linear interpolation
        self.buffer[idx0] * (1.0 - frac) + self.buffer[idx1] * frac
    }

    #[inline(always)]
    pub fn process_sample(&mut self, input: f32) -> f32 {
        self.buffer[self.write_idx] = input;
        self.write_idx = (self.write_idx + 1) % self.buf_size;

        // Smooth toward target ratio
        self.current_ratio += (self.target_ratio - self.current_ratio) * self.smoothing_coeff;

        if (self.current_ratio - 1.0).abs() < 0.002 {
            // Direct transparent passthrough if ratio is unity
            return input;
        }

        // Modulation delta: dDelay/dt = 1 - ratio
        let delay_rate = 1.0 - self.current_ratio;
        self.tap1 += delay_rate;
        self.tap2 += delay_rate;

        // Wrap taps within window
        while self.tap1 >= self.window_length { self.tap1 -= self.window_length; }
        while self.tap1 < 0.0 { self.tap1 += self.window_length; }
        while self.tap2 >= self.window_length { self.tap2 -= self.window_length; }
        while self.tap2 < 0.0 { self.tap2 += self.window_length; }

        // Equal-power Hann-based crossfade between Tap 1 and Tap 2
        let fade = (self.tap1 / self.window_length) * std::f32::consts::PI;
        let w1 = (fade.sin()).abs();
        let w2 = (fade.cos()).abs();

        let s1 = self.read_fractional(self.tap1 + 64.0);
        let s2 = self.read_fractional(self.tap2 + 64.0);

        s1 * w1 + s2 * w2
    }
}
