//! Master Chain (Mix Bus / Mastering) with Dynamic Stereo Node Rack.
//!
//! Signal Order (by default):
//! 1. 5-Band Minimum-Phase Mastering EQ
//! 2. 3-Band Multiband Compressor (Linkwitz-Riley crossover)
//! 3. Stereo Width / Mid-Side Processor with Mono-Bass filtering
//! 4. VCA Glue Compressor + Tape Saturation
//! 5. True-Peak Brickwall Limiter (Ceiling -1.0 dBTP)
//! 6. EBU R128 LUFS Loudness & True-Peak Metering
//!
//! Nodes are contained in an ordered `StereoRack` (`Vec<Box<dyn StereoDspNode>>`)
//! supporting runtime reordering, bypass toggling, and addition/removal without
//! audio-thread allocations.

#![allow(dead_code)]

use super::glue_compressor::GlueCompressorSaturator;
use super::limiter::TruePeakLimiter;
use super::lufs_meter::LufsMeter;
use super::master_eq::MasterEq;
use super::multiband::MultibandCompressor;
use super::rack::StereoRack;
use super::stereo_width::StereoWidthMidSide;

pub struct MasterChain {
    pub rack: StereoRack,
    pub sample_rate: f32,
}

impl MasterChain {
    pub fn new(sample_rate: f32) -> Self {
        let mut rack = StereoRack::with_capacity(16);
        // Default ordered mastering chain
        rack.push(MasterEq::new(sample_rate));               // 0: Master EQ
        rack.push(MultibandCompressor::new(sample_rate));    // 1: Multiband Compressor
        rack.push(StereoWidthMidSide::new(sample_rate));     // 2: Stereo Width / M-S
        rack.push(GlueCompressorSaturator::new(sample_rate));// 3: Glue Comp + Tape
        rack.push(TruePeakLimiter::new(sample_rate));        // 4: True-Peak Limiter
        rack.push(LufsMeter::new(sample_rate));              // 5: EBU R128 Meter

        Self {
            rack,
            sample_rate,
        }
    }

    #[inline(always)]
    pub fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        self.rack.process_stereo(left, right)
    }

    pub fn reset_all(&mut self) {
        self.rack.reset();
    }

    /// Safely swap the active master rack with a pre-built rack constructed off-thread.
    /// Returns the old rack so it is dropped on the caller thread, avoiding
    /// any audio-thread allocations or deallocations.
    pub fn swap_rack(&mut self, mut new_rack: StereoRack) -> StereoRack {
        std::mem::swap(&mut self.rack, &mut new_rack);
        new_rack
    }

    // Typed node accessors for telemetry & UI control
    pub fn eq(&self) -> Option<&MasterEq> {
        self.rack.find_node::<MasterEq>()
    }
    pub fn eq_mut(&mut self) -> Option<&mut MasterEq> {
        self.rack.find_node_mut::<MasterEq>()
    }

    pub fn multiband(&self) -> Option<&MultibandCompressor> {
        self.rack.find_node::<MultibandCompressor>()
    }
    pub fn multiband_mut(&mut self) -> Option<&mut MultibandCompressor> {
        self.rack.find_node_mut::<MultibandCompressor>()
    }

    pub fn stereo_width(&self) -> Option<&StereoWidthMidSide> {
        self.rack.find_node::<StereoWidthMidSide>()
    }
    pub fn stereo_width_mut(&mut self) -> Option<&mut StereoWidthMidSide> {
        self.rack.find_node_mut::<StereoWidthMidSide>()
    }

    pub fn glue(&self) -> Option<&GlueCompressorSaturator> {
        self.rack.find_node::<GlueCompressorSaturator>()
    }
    pub fn glue_mut(&mut self) -> Option<&mut GlueCompressorSaturator> {
        self.rack.find_node_mut::<GlueCompressorSaturator>()
    }

    pub fn limiter(&self) -> Option<&TruePeakLimiter> {
        self.rack.find_node::<TruePeakLimiter>()
    }
    pub fn limiter_mut(&mut self) -> Option<&mut TruePeakLimiter> {
        self.rack.find_node_mut::<TruePeakLimiter>()
    }

    pub fn meter(&self) -> Option<&LufsMeter> {
        self.rack.find_node::<LufsMeter>()
    }
    pub fn meter_mut(&mut self) -> Option<&mut LufsMeter> {
        self.rack.find_node_mut::<LufsMeter>()
    }
}
