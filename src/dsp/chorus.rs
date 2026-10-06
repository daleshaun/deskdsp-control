//! Analog-Voiced Multi-Voice Chorus / Ensemble Effect.
//! Uses an LFO-modulated delay line with 4-point cubic Hermite interpolation.
//! All buffers allocated in `::new`; zero allocations on audio thread.

#![allow(dead_code)]

use super::DspNode;

#[derive(Debug, Clone)]
pub struct ChorusNode {
    sample_rate: f32,
    buffer: Vec<f32>,
    buf_size: usize,
    write_idx: usize,

    // LFO parameters
    pub rate_hz: f32,
    pub depth_ms: f32,
    pub mix: f32,
    pub feedback: f32,
    pub bypassed: bool,

    lfo_phase: f32,
}

impl ChorusNode {
    pub fn new(sample_rate: f32) -> Self {
        let buf_size = 4096;
        Self {
            sample_rate,
            buffer: vec![0.0; buf_size],
            buf_size,
            write_idx: 0,
            rate_hz: 1.2,
            depth_ms: 3.5,
            mix: 0.5,
            feedback: 0.15,
            bypassed: false,
            lfo_phase: 0.0,
        }
    }

    pub fn set_params(&mut self, rate_hz: f32, depth_ms: f32, mix: f32, feedback: f32) {
        self.rate_hz = rate_hz.clamp(0.1, 5.0);
        self.depth_ms = depth_ms.clamp(0.5, 10.0);
        self.mix = mix.clamp(0.0, 1.0);
        self.feedback = feedback.clamp(0.0, 0.7);
    }

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
}

impl DspNode for ChorusNode {
    fn name(&self) -> &'static str {
        "Stereo Chorus"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.buffer.fill(0.0);
        self.write_idx = 0;
        self.lfo_phase = 0.0;
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed {
            return input;
        }

        if !input.is_finite() {
            return 0.0;
        }

        // LFO sine modulation
        let lfo = (self.lfo_phase * 2.0 * std::f32::consts::PI).sin();
        self.lfo_phase += self.rate_hz / self.sample_rate;
        if self.lfo_phase >= 1.0 {
            self.lfo_phase -= 1.0;
        }

        // Modulate delay between ~8ms and ~22ms
        let base_delay_samples = self.sample_rate * 0.012; // 12ms nominal
        let mod_depth_samples = (self.depth_ms * 0.001) * self.sample_rate;
        let delay_samples = base_delay_samples + lfo * mod_depth_samples;

        let wet = self.read_cubic(delay_samples);

        // Feedback into delay buffer
        let to_buffer = (input + wet * self.feedback).clamp(-2.0, 2.0);
        self.buffer[self.write_idx] = to_buffer;
        self.write_idx = (self.write_idx + 1) & (self.buf_size - 1);

        // Dry/wet blend
        (input * (1.0 - self.mix) + wet * self.mix).clamp(-1.0, 1.0)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
