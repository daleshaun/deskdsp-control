//! Studio Reverb Node based on Schroeder-Freeverb Topology.
//! Uses 8 parallel feedback comb filters with frequency-dependent damping
//! and 4 cascaded allpass diffusers with optional pre-delay.
//! All buffers preallocated in `::new`; zero allocations on audio thread.

#![allow(dead_code)]

use super::DspNode;

#[derive(Debug, Clone)]
struct CombFilter {
    buffer: Vec<f32>,
    buf_size: usize,
    index: usize,
    filter_store: f32,
    feedback: f32,
    damp1: f32,
    damp2: f32,
}

impl CombFilter {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            buf_size: size,
            index: 0,
            filter_store: 0.0,
            feedback: 0.8,
            damp1: 0.4,
            damp2: 0.6,
        }
    }

    fn set_damp(&mut self, val: f32) {
        self.damp1 = val.clamp(0.0, 1.0);
        self.damp2 = 1.0 - self.damp1;
    }

    fn set_feedback(&mut self, val: f32) {
        self.feedback = val.clamp(0.0, 0.98);
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
        self.filter_store = 0.0;
    }

    #[inline(always)]
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.index];
        self.filter_store = output * self.damp2 + self.filter_store * self.damp1;
        self.buffer[self.index] = input + self.filter_store * self.feedback;
        self.index = (self.index + 1) % self.buf_size;
        output
    }
}

#[derive(Debug, Clone)]
struct AllpassFilter {
    buffer: Vec<f32>,
    buf_size: usize,
    index: usize,
    feedback: f32,
}

impl AllpassFilter {
    fn new(size: usize) -> Self {
        Self {
            buffer: vec![0.0; size],
            buf_size: size,
            index: 0,
            feedback: 0.5,
        }
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.index = 0;
    }

    #[inline(always)]
    fn process(&mut self, input: f32) -> f32 {
        let buf_out = self.buffer[self.index];
        let output = -input + buf_out;
        self.buffer[self.index] = input + buf_out * self.feedback;
        self.index = (self.index + 1) % self.buf_size;
        output
    }
}

#[derive(Debug, Clone)]
pub struct ReverbNode {
    sample_rate: f32,
    pub room_size: f32,
    pub damping: f32,
    pub pre_delay_ms: f32,
    pub mix: f32,
    pub bypassed: bool,

    combs: Vec<CombFilter>,
    allpasses: Vec<AllpassFilter>,
    pre_delay_buf: Vec<f32>,
    pre_delay_idx: usize,
}

impl ReverbNode {
    pub fn new(sample_rate: f32) -> Self {
        let scale = sample_rate / 44100.0;

        // Mutually prime comb lengths scaled to sample rate
        let comb_tunings = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
        let allpass_tunings = [556, 441, 341, 225];

        let combs = comb_tunings
            .iter()
            .map(|&t| CombFilter::new((t as f32 * scale).round() as usize))
            .collect();

        let allpasses = allpass_tunings
            .iter()
            .map(|&t| AllpassFilter::new((t as f32 * scale).round() as usize))
            .collect();

        let max_predelay = (sample_rate * 0.15) as usize; // up to 150ms pre-delay

        let mut reverb = Self {
            sample_rate,
            room_size: 0.78,
            damping: 0.35,
            pre_delay_ms: 12.0,
            mix: 0.28,
            bypassed: false,
            combs,
            allpasses,
            pre_delay_buf: vec![0.0; max_predelay],
            pre_delay_idx: 0,
        };
        reverb.update_params();
        reverb
    }

    pub fn set_params(&mut self, room_size: f32, damping: f32, pre_delay_ms: f32, mix: f32) {
        self.room_size = room_size.clamp(0.0, 1.0);
        self.damping = damping.clamp(0.0, 1.0);
        self.pre_delay_ms = pre_delay_ms.clamp(0.0, 120.0);
        self.mix = mix.clamp(0.0, 1.0);
        self.update_params();
    }

    pub fn set_mix(&mut self, mix: f32) {
        self.mix = mix.clamp(0.0, 1.0);
    }

    fn update_params(&mut self) {
        let feedback = 0.7 + self.room_size * 0.26;
        for c in &mut self.combs {
            c.set_feedback(feedback);
            c.set_damp(self.damping);
        }
    }
}

impl DspNode for ReverbNode {
    fn name(&self) -> &'static str {
        "Studio Reverb"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        for c in &mut self.combs { c.reset(); }
        for a in &mut self.allpasses { a.reset(); }
        self.pre_delay_buf.fill(0.0);
        self.pre_delay_idx = 0;
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }

        if !input.is_finite() {
            return 0.0;
        }

        // 1. Pre-delay
        let pre_delay_samples = ((self.pre_delay_ms * 0.001 * self.sample_rate) as usize)
            .min(self.pre_delay_buf.len() - 1);
        let read_idx = (self.pre_delay_idx + self.pre_delay_buf.len() - pre_delay_samples) % self.pre_delay_buf.len();
        let delayed_in = self.pre_delay_buf[read_idx];
        self.pre_delay_buf[self.pre_delay_idx] = input;
        self.pre_delay_idx = (self.pre_delay_idx + 1) % self.pre_delay_buf.len();

        // 2. Parallel Comb Filters
        let mut wet = 0.0_f32;
        let comb_input = delayed_in * 0.015; // Gain scaling for comb sum
        for c in &mut self.combs {
            wet += c.process(comb_input);
        }

        // 3. Series Allpass Diffusers
        for a in &mut self.allpasses {
            wet = a.process(wet);
        }

        // 4. Dry/Wet Mix
        (input * (1.0 - self.mix) + wet * self.mix * 2.0).clamp(-1.0, 1.0)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
