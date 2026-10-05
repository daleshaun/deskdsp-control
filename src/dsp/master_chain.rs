//! Master Chain (Mix Bus / Mastering) with ordered node pipeline.
//!
//! Signal Order (as specified in brief):
//! 1. 5-Band Minimum-Phase Mastering EQ
//! 2. 3-Band Multiband Compressor (Linkwitz-Riley crossover)
//! 3. Stereo Width / Mid-Side Processor with Mono-Bass filtering
//! 4. VCA Glue Compressor + Tape Saturation
//! 5. True-Peak Brickwall Limiter (Ceiling -1.0 dBTP)
//! 6. EBU R128 LUFS Loudness & True-Peak Metering

use super::glue_compressor::GlueCompressorSaturator;
use super::limiter::TruePeakLimiter;
use super::lufs_meter::LufsMeter;
use super::master_eq::MasterEq;
use super::multiband::MultibandCompressor;
use super::stereo_width::StereoWidthMidSide;
use super::StereoDspNode;

#[derive(Debug, Clone)]
pub struct MasterChain {
    // 1. Minimum-phase EQ
    pub eq: MasterEq,

    // 2. 3-Band Multiband Compressor
    pub multiband: MultibandCompressor,

    // 3. Stereo Width / Mid-Side
    pub stereo_width: StereoWidthMidSide,

    // 4. Glue Compressor + Saturation
    pub glue: GlueCompressorSaturator,

    // 5. True-Peak Brickwall Limiter (-1 dBTP)
    pub limiter: TruePeakLimiter,

    // 6. EBU R128 LUFS Metering
    pub meter: LufsMeter,

    pub sample_rate: f32,
}

impl MasterChain {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            eq: MasterEq::new(sample_rate),
            multiband: MultibandCompressor::new(sample_rate),
            stereo_width: StereoWidthMidSide::new(sample_rate),
            glue: GlueCompressorSaturator::new(sample_rate),
            limiter: TruePeakLimiter::new(sample_rate),
            meter: LufsMeter::new(sample_rate),
            sample_rate,
        }
    }

    #[inline(always)]
    pub fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32) {
        // 1. Master EQ
        let (eq_l, eq_r) = self.eq.process_stereo(left, right);

        // 2. Multiband Compressor
        let (mb_l, mb_r) = self.multiband.process_stereo(eq_l, eq_r);

        // 3. Stereo Width / Mid-Side
        let (w_l, w_r) = self.stereo_width.process_stereo(mb_l, mb_r);

        // 4. Glue Compressor & Tape Saturation
        let (glue_l, glue_r) = self.glue.process_stereo(w_l, w_r);

        // 5. True-Peak Limiter (-1.0 dBTP ceiling)
        let (out_l, out_r) = self.limiter.process_stereo(glue_l, glue_r);

        // 6. Metering (EBU R128 LUFS and True-Peak)
        self.meter.process_sample(out_l, out_r);

        (out_l, out_r)
    }

    pub fn reset_all(&mut self) {
        self.eq.reset();
        self.multiband.reset();
        self.stereo_width.reset();
        self.glue.reset();
        self.limiter.reset();
        self.meter.reset();
    }
}
