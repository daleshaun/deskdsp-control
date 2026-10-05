//! DeskDSP Control: Modular Real-Time Audio DSP Engine.
//!
//! All processing is allocation-free in the audio thread.
//! Nodes implement `DspNode` (monophonic) or `StereoDspNode` (stereo).
//! Bypassed nodes guarantee bit-identical passthrough.

#![allow(dead_code, unused_imports)]

pub mod biquad;
pub mod dc_blocker;
pub mod envelope;
pub mod gate;
pub mod deesser;
pub mod compressor;
pub mod saturation;
pub mod tuner;
pub mod master_eq;
pub mod multiband;
pub mod stereo_width;
pub mod glue_compressor;
pub mod limiter;
pub mod lufs_meter;
pub mod rack;
pub mod channel_strip;
pub mod master_chain;

pub use channel_strip::ChannelStrip;
pub use master_chain::MasterChain;
pub use rack::{MonoRack, StereoRack};

/// Real-time node telemetry for UI and meters without heap allocations.
#[derive(Debug, Clone, Copy, Default)]
pub struct NodeTelemetry {
    pub gain_reduction_db: f32,
    pub detected_freq_hz: f32,
    pub cents_deviation: f32,
    pub true_peak_dbtp: f32,
    pub momentary_lufs: f32,
    pub short_term_lufs: f32,
    pub integrated_lufs: f32,
}

/// Trait for real-time monophonic DSP nodes.
pub trait DspNode: Send + 'static {
    /// Human-readable node name
    fn name(&self) -> &'static str;

    /// Whether this node is currently bypassed
    fn is_bypassed(&self) -> bool;

    /// Set bypass state. When bypassed, `process_sample` MUST return the input unmodified.
    fn set_bypassed(&mut self, bypassed: bool);

    /// Reset internal filter state and buffers
    fn reset(&mut self);

    /// Process a single audio sample (allocation-free).
    fn process_sample(&mut self, sample: f32) -> f32;

    /// In-place block processing.
    #[inline(always)]
    fn process_block(&mut self, buffer: &mut [f32]) {
        if self.is_bypassed() {
            return;
        }
        for s in buffer.iter_mut() {
            *s = self.process_sample(*s);
        }
    }

    /// Report real-time telemetry (GR, pitch, peak)
    fn telemetry(&self) -> NodeTelemetry {
        NodeTelemetry::default()
    }

    /// Downcast support
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// Trait for real-time stereo DSP nodes.
pub trait StereoDspNode: Send + 'static {
    fn name(&self) -> &'static str;
    fn is_bypassed(&self) -> bool;
    fn set_bypassed(&mut self, bypassed: bool);
    fn reset(&mut self);

    /// Process a stereo sample pair (allocation-free).
    fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32);

    #[inline(always)]
    fn process_stereo_block(&mut self, left_buf: &mut [f32], right_buf: &mut [f32]) {
        if self.is_bypassed() {
            return;
        }
        let len = left_buf.len().min(right_buf.len());
        for i in 0..len {
            let (l, r) = self.process_stereo(left_buf[i], right_buf[i]);
            left_buf[i] = l;
            right_buf[i] = r;
        }
    }

    /// Report real-time telemetry (GR, loudness, True-Peak)
    fn telemetry(&self) -> NodeTelemetry {
        NodeTelemetry::default()
    }

    /// Downcast support
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}
