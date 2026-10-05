//! Vocal-focused Channel Strip with ordered node pipeline.
//!
//! Signal Order (as specified in brief):
//! 1. Input Trim & Phase Invert
//! 2. High-Pass Filter (Low-cut)
//! 3. Noise Gate / Downward Expander
//! 4. Vocal De-Esser
//! 5. 4-Band Parametric EQ (Low-shelf, Low-mid, High-mid, High-shelf)
//! 6. Vocal Compressor (Opto / FET feel with soft knee)
//! 7. Vocal Tuner (YIN pitch detection + scale quantizing + time-domain pitch shifting)
//! 8. Analog Saturation (Tube / Tape coloration + DC blocker)
//! 9. Output Trim & Peak Limiting

use super::biquad::{BiquadFilter, FilterType};
use super::compressor::VocalCompressor;
use super::deesser::DeEsser;
use super::gate::NoiseGate;
use super::saturation::Saturation;
use super::tuner::VocalTuner;
use super::DspNode;

#[derive(Debug, Clone)]
pub struct ParametricEq4Band {
    pub low_shelf: BiquadFilter,
    pub low_mid: BiquadFilter,
    pub high_mid: BiquadFilter,
    pub high_shelf: BiquadFilter,
    pub bypassed: bool,
}

impl ParametricEq4Band {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            low_shelf: BiquadFilter::new(FilterType::LowShelf { q: 0.707 }, 100.0, 0.0, sample_rate),
            low_mid: BiquadFilter::new(FilterType::Peaking { q: 1.0 }, 450.0, 0.0, sample_rate),
            high_mid: BiquadFilter::new(FilterType::Peaking { q: 1.2 }, 3200.0, 0.0, sample_rate),
            high_shelf: BiquadFilter::new(FilterType::HighShelf { q: 0.707 }, 10000.0, 0.0, sample_rate),
            bypassed: false,
        }
    }
}

impl DspNode for ParametricEq4Band {
    fn name(&self) -> &'static str {
        "4-Band Parametric EQ"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {
        self.low_shelf.reset();
        self.low_mid.reset();
        self.high_mid.reset();
        self.high_shelf.reset();
    }

    #[inline(always)]
    fn process_sample(&mut self, mut input: f32) -> f32 {
        if self.bypassed {
            return input;
        }
        input = self.low_shelf.process_sample(input);
        input = self.low_mid.process_sample(input);
        input = self.high_mid.process_sample(input);
        input = self.high_shelf.process_sample(input);
        input
    }
}

#[derive(Debug, Clone)]
pub struct ChannelStrip {
    // 1. Input Trim & Phase
    pub input_gain_db: f32,
    pub phase_invert: bool,

    // 2. High-Pass Filter
    pub high_pass: BiquadFilter,

    // 3. Noise Gate / Expander
    pub gate: NoiseGate,

    // 4. Vocal De-Esser
    pub deesser: DeEsser,

    // 5. 4-Band Parametric EQ
    pub eq: ParametricEq4Band,

    // 6. Vocal Compressor
    pub compressor: VocalCompressor,

    // 7. Vocal Tuner
    pub tuner: VocalTuner,

    // 8. Analog Saturation
    pub saturation: Saturation,

    // 9. Output Trim
    pub output_gain_db: f32,

    pub sample_rate: f32,
}

impl ChannelStrip {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            input_gain_db: 0.0,
            phase_invert: false,
            high_pass: BiquadFilter::new(FilterType::HighPass, 80.0, 0.0, sample_rate),
            gate: NoiseGate::new(sample_rate),
            deesser: DeEsser::new(sample_rate),
            eq: ParametricEq4Band::new(sample_rate),
            compressor: VocalCompressor::new(sample_rate),
            tuner: VocalTuner::new(sample_rate),
            saturation: Saturation::new(sample_rate),
            output_gain_db: 0.0,
            sample_rate,
        }
    }

    #[inline(always)]
    pub fn process(&mut self, mut sample: f32) -> f32 {
        // 1. Input Trim & Phase
        if self.phase_invert {
            sample = -sample;
        }
        if self.input_gain_db.abs() > 0.001 {
            sample *= 10.0_f32.powf(self.input_gain_db / 20.0);
        }

        // 2. High-Pass Filter
        sample = self.high_pass.process_sample(sample);

        // 3. Noise Gate / Expander
        sample = self.gate.process_sample(sample);

        // 4. De-Esser
        sample = self.deesser.process_sample(sample);

        // 5. Parametric EQ
        sample = self.eq.process_sample(sample);

        // 6. Vocal Compressor
        sample = self.compressor.process_sample(sample);

        // 7. Vocal Tuner
        sample = self.tuner.process_sample(sample);

        // 8. Analog Saturation
        sample = self.saturation.process_sample(sample);

        // 9. Output Trim
        if self.output_gain_db.abs() > 0.001 {
            sample *= 10.0_f32.powf(self.output_gain_db / 20.0);
        }

        // Safety peak limit
        sample.clamp(-0.999, 0.999)
    }

    pub fn reset_all(&mut self) {
        self.high_pass.reset();
        self.gate.reset();
        self.deesser.reset();
        self.eq.reset();
        self.compressor.reset();
        self.tuner.reset();
        self.saturation.reset();
    }
}
