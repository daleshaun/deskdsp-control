//! Microphone Imaging DSP Node (`MicImage`).
//!
//! Transforms an incoming vocal signal toward a target microphone's measured
//! on-axis magnitude and phase response using partitioned-FFT convolution (`ConvEngine`).
//!
//! ### Scope & Fidelity Note:
//! An impulse response captures the **linear on-axis transfer function** (the bulk
//! of a microphone's tonal fingerprint). It does NOT reproduce polar pattern off-axis
//! rejection, physical proximity effect, capsule saturation/nonlinearity, or self-noise.
//!
//! ### Transfer IR Requirement:
//! Loaded impulse responses must be **transfer IRs** (`target ÷ reference`), i.e. the
//! target microphone deconvolved by the reference capture microphone. Loading a raw target
//! IR will double-apply the capture microphone's native coloration.

#![allow(dead_code)]

use super::conv_engine::ConvEngine;
use super::DspNode;

#[derive(Clone)]
pub struct MicImage {
    pub engine: ConvEngine,
    pub dry_wet: f32, // 0.0 (100% dry) to 1.0 (100% wet)
    pub bypassed: bool,
    pub ir_name: String,
    pub has_ir: bool,

    // Latency-compensation delay line for phase-aligned dry/wet blending
    dry_delay_buf: Vec<f32>,
    dry_write_pos: usize,
    latency_samples: usize,
}

impl MicImage {
    /// Creates a new MicImage node with an impulse response.
    /// IR parsing and FFT partitioning occur strictly off the audio thread.
    pub fn new(ir: &[f32], partition_len: usize, ir_name: impl Into<String>) -> Self {
        let engine = ConvEngine::new(ir, partition_len);
        let latency = engine.partition_len;
        let has_ir = !ir.is_empty();

        Self {
            engine,
            dry_wet: 1.0, // 100% wet by default
            bypassed: true, // Safe default: off until activated
            ir_name: ir_name.into(),
            has_ir,
            dry_delay_buf: vec![0.0; latency * 2],
            dry_write_pos: 0,
            latency_samples: latency,
        }
    }

    /// Creates an empty MicImage node without an IR loaded.
    pub fn empty(sample_rate: f32) -> Self {
        let partition = if sample_rate > 88200.0 { 256 } else { 128 };
        Self::new(&[], partition, "None")
    }

    pub fn set_dry_wet(&mut self, dry_wet: f32) {
        self.dry_wet = dry_wet.clamp(0.0, 1.0);
    }
}

impl DspNode for MicImage {
    fn name(&self) -> &'static str {
        "Mic Image (IR)"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed || !self.has_ir
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.engine.reset();
        self.dry_delay_buf.fill(0.0);
        self.dry_write_pos = 0;
    }

    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.is_bypassed() {
            return input;
        }

        // Store into latency compensation ring for phase-aligned dry/wet blend
        let buf_len = self.dry_delay_buf.len();
        self.dry_delay_buf[self.dry_write_pos] = input;
        let dry_read_pos = (self.dry_write_pos + buf_len - self.latency_samples) % buf_len;
        let delayed_dry = self.dry_delay_buf[dry_read_pos];
        self.dry_write_pos = (self.dry_write_pos + 1) % buf_len;

        let wet = self.engine.process_sample(input);

        if (self.dry_wet - 1.0).abs() < 1e-4 {
            wet
        } else if self.dry_wet < 1e-4 {
            delayed_dry
        } else {
            delayed_dry * (1.0 - self.dry_wet) + wet * self.dry_wet
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
