//! Vocal-focused Channel Strip with Dynamic Node Rack.
//!
//! Signal Order (by default):
//! 1. Input Trim & Phase Invert
//! 2. High-Pass Filter (Low-cut 80 Hz)
//! 3. Noise Gate / Downward Expander
//! 4. Vocal De-Esser
//! 5. 4-Band Parametric EQ (Low-shelf, Low-mid, High-mid, High-shelf)
//! 6. Vocal Compressor (Opto / FET feel with soft knee)
//! 7. Vocal Tuner (YIN pitch detection + scale quantizing + granular pitch shifting)
//! 8. Analog Saturation (Tube / Tape coloration + DC blocker)
//! 9. Output Trim & Peak Limiting
//!
//! Nodes are contained in an ordered `MonoRack` (`Vec<Box<dyn DspNode>>`)
//! supporting runtime reordering, bypass toggling, and addition/removal without
//! audio-thread allocations.

#![allow(dead_code)]

use super::biquad::{BiquadFilter, FilterType};
use super::cab_sim::CabSim;
use super::chorus::ChorusNode;
use super::compressor::VocalCompressor;
use super::deesser::DeEsser;
use super::drive::DriveNode;
use super::exciter::HarmonicExciter;
use super::gate::NoiseGate;
use super::guitar_amp::GuitarAmp;
use super::rack::MonoRack;
use super::reverb::ReverbNode;
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

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

pub struct ChannelStrip {
    pub rack: MonoRack,
    pub input_gain_db: f32,
    pub phase_invert: bool,
    pub output_gain_db: f32,
    pub sample_rate: f32,
}

impl ChannelStrip {
    pub fn new(sample_rate: f32) -> Self {
        let mut rack = MonoRack::with_capacity(16);
        // Default ordered tracking chain - safe defaults:
        // High-pass, Gate, De-Esser, Comp, Tuner, and Saturation are bypassed by default
        // so music and program material pass bit-identically clean until explicitly activated.
        let mut hpf = BiquadFilter::new(FilterType::HighPass, 20.0, 0.0, sample_rate);
        hpf.bypassed = true;
        rack.push(hpf); // 0: HPF (20Hz safe sub-audio cutoff)

        let mut gate = NoiseGate::new(sample_rate);
        gate.bypassed = true;
        rack.push(gate); // 1: Noise Gate

        let mut deesser = DeEsser::new(sample_rate);
        deesser.bypassed = true;
        rack.push(deesser); // 2: De-Esser

        rack.push(ParametricEq4Band::new(sample_rate)); // 3: 4-Band EQ (flat 0dB)

        let mut comp = VocalCompressor::new(sample_rate);
        comp.bypassed = true;
        rack.push(comp); // 4: Compressor

        let mut tuner = VocalTuner::new(sample_rate);
        tuner.bypassed = true;
        rack.push(tuner); // 5: Vocal Tuner

        let mut sat = Saturation::new(sample_rate);
        sat.bypassed = true;
        rack.push(sat); // 6: Saturation

        Self {
            rack,
            input_gain_db: 0.0,
            phase_invert: false,
            output_gain_db: 0.0,
            sample_rate,
        }
    }

    #[inline(always)]
    pub fn process(&mut self, mut sample: f32) -> f32 {
        // 1. Input Trim & Phase Invert
        if self.phase_invert {
            sample = -sample;
        }
        if self.input_gain_db.abs() > 0.001 {
            sample *= 10.0_f32.powf(self.input_gain_db / 20.0);
        }

        // 2. Ordered Rack Processing (zero allocation)
        sample = self.rack.process(sample);

        // 3. Output Trim & Safety Peak Limit
        if self.output_gain_db.abs() > 0.001 {
            sample *= 10.0_f32.powf(self.output_gain_db / 20.0);
        }

        sample.clamp(-0.999, 0.999)
    }

    pub fn reset_all(&mut self) {
        self.rack.reset();
    }

    /// Safely swap the active rack with a pre-built rack constructed off-thread.
    /// Returns the old rack so it is dropped on the caller thread, avoiding
    /// any audio-thread allocations or deallocations.
    pub fn swap_rack(&mut self, mut new_rack: MonoRack) -> MonoRack {
        std::mem::swap(&mut self.rack, &mut new_rack);
        new_rack
    }

    // Typed node accessors for telemetry & hotkey adjustments
    pub fn hpf(&self) -> Option<&BiquadFilter> {
        self.rack.find_node::<BiquadFilter>()
    }
    pub fn hpf_mut(&mut self) -> Option<&mut BiquadFilter> {
        self.rack.find_node_mut::<BiquadFilter>()
    }

    pub fn gate(&self) -> Option<&NoiseGate> {
        self.rack.find_node::<NoiseGate>()
    }
    pub fn gate_mut(&mut self) -> Option<&mut NoiseGate> {
        self.rack.find_node_mut::<NoiseGate>()
    }

    pub fn deesser(&self) -> Option<&DeEsser> {
        self.rack.find_node::<DeEsser>()
    }
    pub fn deesser_mut(&mut self) -> Option<&mut DeEsser> {
        self.rack.find_node_mut::<DeEsser>()
    }

    pub fn eq(&self) -> Option<&ParametricEq4Band> {
        self.rack.find_node::<ParametricEq4Band>()
    }
    pub fn eq_mut(&mut self) -> Option<&mut ParametricEq4Band> {
        self.rack.find_node_mut::<ParametricEq4Band>()
    }

    pub fn compressor(&self) -> Option<&VocalCompressor> {
        self.rack.find_node::<VocalCompressor>()
    }
    pub fn compressor_mut(&mut self) -> Option<&mut VocalCompressor> {
        self.rack.find_node_mut::<VocalCompressor>()
    }

    pub fn tuner(&self) -> Option<&VocalTuner> {
        self.rack.find_node::<VocalTuner>()
    }
    pub fn tuner_mut(&mut self) -> Option<&mut VocalTuner> {
        self.rack.find_node_mut::<VocalTuner>()
    }

    pub fn saturation(&self) -> Option<&Saturation> {
        self.rack.find_node::<Saturation>()
    }
    pub fn saturation_mut(&mut self) -> Option<&mut Saturation> {
        self.rack.find_node_mut::<Saturation>()
    }

    pub fn amp(&self) -> Option<&GuitarAmp> {
        self.rack.find_node::<GuitarAmp>()
    }
    pub fn amp_mut(&mut self) -> Option<&mut GuitarAmp> {
        self.rack.find_node_mut::<GuitarAmp>()
    }

    pub fn cab(&self) -> Option<&CabSim> {
        self.rack.find_node::<CabSim>()
    }
    pub fn cab_mut(&mut self) -> Option<&mut CabSim> {
        self.rack.find_node_mut::<CabSim>()
    }

    pub fn drive(&self) -> Option<&DriveNode> {
        self.rack.find_node::<DriveNode>()
    }
    pub fn drive_mut(&mut self) -> Option<&mut DriveNode> {
        self.rack.find_node_mut::<DriveNode>()
    }

    pub fn chorus(&self) -> Option<&ChorusNode> {
        self.rack.find_node::<ChorusNode>()
    }
    pub fn chorus_mut(&mut self) -> Option<&mut ChorusNode> {
        self.rack.find_node_mut::<ChorusNode>()
    }

    pub fn reverb(&self) -> Option<&ReverbNode> {
        self.rack.find_node::<ReverbNode>()
    }
    pub fn reverb_mut(&mut self) -> Option<&mut ReverbNode> {
        self.rack.find_node_mut::<ReverbNode>()
    }

    pub fn exciter(&self) -> Option<&HarmonicExciter> {
        self.rack.find_node::<HarmonicExciter>()
    }
    pub fn exciter_mut(&mut self) -> Option<&mut HarmonicExciter> {
        self.rack.find_node_mut::<HarmonicExciter>()
    }
}
