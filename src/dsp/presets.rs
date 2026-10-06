//! Per-Channel Instrument Presets & Hot-Swappable Processing Chains.
//!
//! Provides factory processing rack templates for various recording sources:
//! - Vocal (HPF -> Gate -> De-Esser -> EQ -> Comp -> Tuner -> Saturation)
//! - Electric Guitar (Gate -> Amp -> Cab -> EQ -> Comp)
//! - Acoustic Guitar (HPF -> Gate -> EQ -> Comp -> Exciter -> Reverb)
//! - Bass (Gate -> Comp -> Drive -> EQ -> Limiter)
//! - Keys / Synth (EQ -> Comp -> Exciter -> Chorus)
//! - Piano / EP (EQ -> Comp -> Saturation -> Reverb)
//! - Program Thru (Empty rack for clean bit-identical passthrough)
//!
//! All racks are built OFF the audio thread and swapped atomically via `swap_rack`.

use serde::{Deserialize, Serialize};
use antelope_protocol::PreampMode;

use super::biquad::{BiquadFilter, FilterType};
use super::cab_sim::{CabSim, CabType};
use super::channel_strip::ParametricEq4Band;
use super::chorus::ChorusNode;
use super::compressor::{CompressorFlavor, VocalCompressor};
use super::deesser::DeEsser;
use super::drive::DriveNode;
use super::exciter::HarmonicExciter;
use super::gate::NoiseGate;
use super::guitar_amp::GuitarAmp;
use super::rack::MonoRack;
use super::reverb::ReverbNode;
use super::saturation::{Saturation, SaturationFlavor};
use super::tuner::VocalTuner;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentPreset {
    #[serde(alias = "vocal", alias = "mic")]
    Vocal,
    #[serde(alias = "eguitar", alias = "electric_guitar", alias = "guitar")]
    ElectricGuitar,
    #[serde(alias = "aguitar", alias = "acoustic_guitar", alias = "acoustic")]
    AcousticGuitar,
    #[serde(alias = "bass")]
    Bass,
    #[serde(alias = "keys", alias = "synth")]
    Keys,
    #[serde(alias = "piano", alias = "ep")]
    Piano,
    #[serde(alias = "program", alias = "program_thru", alias = "thru", alias = "bypass")]
    ProgramThru,
}

impl InstrumentPreset {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Vocal => "vocal",
            Self::ElectricGuitar => "eguitar",
            Self::AcousticGuitar => "aguitar",
            Self::Bass => "bass",
            Self::Keys => "keys",
            Self::Piano => "piano",
            Self::ProgramThru => "program",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Vocal => "Vocal",
            Self::ElectricGuitar => "Electric Guitar",
            Self::AcousticGuitar => "Acoustic Guitar",
            Self::Bass => "Bass",
            Self::Keys => "Keys / Synth",
            Self::Piano => "Piano / EP",
            Self::ProgramThru => "Program (Direct)",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "vocal" | "mic" => Some(Self::Vocal),
            "eguitar" | "electric_guitar" | "guitar" | "electric" => Some(Self::ElectricGuitar),
            "aguitar" | "acoustic_guitar" | "acoustic" => Some(Self::AcousticGuitar),
            "bass" => Some(Self::Bass),
            "keys" | "synth" => Some(Self::Keys),
            "piano" | "ep" => Some(Self::Piano),
            "program" | "program_thru" | "thru" | "bypass" => Some(Self::ProgramThru),
            _ => None,
        }
    }
}

/// Builds the node rack OFF the audio thread. Allocations are strictly isolated here.
pub fn build_rack(preset: InstrumentPreset, sample_rate: f32) -> MonoRack {
    match preset {
        InstrumentPreset::Vocal => {
            let mut rack = MonoRack::with_capacity(16);
            let mut hpf = BiquadFilter::new(FilterType::HighPass, 20.0, 0.0, sample_rate);
            hpf.bypassed = true;
            rack.push(hpf); // 0: HPF

            let mut gate = NoiseGate::new(sample_rate);
            gate.bypassed = true;
            rack.push(gate); // 1: Gate

            let mut deesser = DeEsser::new(sample_rate);
            deesser.bypassed = true;
            rack.push(deesser); // 2: De-Esser

            rack.push(ParametricEq4Band::new(sample_rate)); // 3: 4-Band EQ

            let mut comp = VocalCompressor::new(sample_rate);
            comp.bypassed = true;
            rack.push(comp); // 4: Compressor

            let mut tuner = VocalTuner::new(sample_rate);
            tuner.bypassed = true;
            rack.push(tuner); // 5: Tuner

            let mut sat = Saturation::new(sample_rate);
            sat.bypassed = true;
            rack.push(sat); // 6: Saturation

            rack
        }
        InstrumentPreset::ElectricGuitar => {
            let mut rack = MonoRack::with_capacity(16);
            // 0: Noise Gate (active to suppress pickup hum)
            let mut gate = NoiseGate::new(sample_rate);
            gate.threshold_db = -60.0;
            gate.bypassed = false;
            rack.push(gate);

            // 1: Guitar Amp (active)
            let mut amp = GuitarAmp::new(sample_rate);
            amp.set_params(4.0, 0.0, 0.0, 0.0, -6.0);
            amp.bypassed = false;
            rack.push(amp);

            // 2: Cab Sim (active 4x12 Stack)
            let mut cab = CabSim::new(sample_rate);
            cab.set_cab_type(CabType::FourByTwelve);
            cab.bypassed = false;
            rack.push(cab);

            // 3: 4-Band EQ (active, flat)
            let eq = ParametricEq4Band::new(sample_rate);
            rack.push(eq);

            // 4: Studio Compressor (bypassed by default)
            let mut comp = VocalCompressor::new(sample_rate);
            comp.bypassed = true;
            rack.push(comp);

            rack
        }
        InstrumentPreset::AcousticGuitar => {
            let mut rack = MonoRack::with_capacity(16);
            // 0: High-Pass Filter (cut body rumble below 75 Hz)
            let mut hpf = BiquadFilter::new(FilterType::HighPass, 75.0, 0.0, sample_rate);
            hpf.bypassed = false;
            rack.push(hpf);

            // 1: Noise Gate (gentle, bypassed)
            let mut gate = NoiseGate::new(sample_rate);
            gate.threshold_db = -65.0;
            gate.bypassed = true;
            rack.push(gate);

            // 2: 4-Band EQ (gentle de-boom and piezo smoothing)
            let mut eq = ParametricEq4Band::new(sample_rate);
            eq.low_shelf.set_gain_db(-2.0);
            eq.high_shelf.set_gain_db(1.5);
            eq.bypassed = false;
            rack.push(eq);

            // 3: Compressor (gentle 2.5:1 acoustic leveling)
            let mut comp = VocalCompressor::new(sample_rate);
            comp.threshold_db = -18.0;
            comp.ratio = 2.5;
            comp.attack_ms = 15.0;
            comp.release_ms = 100.0;
            comp.bypassed = false;
            rack.push(comp);

            // 4: Harmonic Exciter (subtle acoustic brilliance, bypassed by default)
            let mut exc = HarmonicExciter::new(sample_rate);
            exc.bypassed = true;
            rack.push(exc);

            // 5: Reverb (studio room ambiance)
            let mut rev = ReverbNode::new(sample_rate);
            rev.set_params(0.5, 0.35, 0.15, 12.0);
            rev.bypassed = false;
            rack.push(rev);

            rack
        }
        InstrumentPreset::Bass => {
            let mut rack = MonoRack::with_capacity(16);
            // 0: Noise Gate (bypassed)
            let mut gate = NoiseGate::new(sample_rate);
            gate.threshold_db = -65.0;
            gate.bypassed = true;
            rack.push(gate);

            // 1: Punchy Compressor (early dynamics control)
            let mut comp = VocalCompressor::new(sample_rate);
            comp.threshold_db = -16.0;
            comp.ratio = 4.0;
            comp.attack_ms = 8.0;
            comp.release_ms = 80.0;
            comp.bypassed = false;
            rack.push(comp);

            // 2: Analog Drive (warm low-end saturation with clean blend)
            let mut drive = DriveNode::new(sample_rate);
            drive.set_params(2.2, -0.2, 0.45, -1.0);
            drive.bypassed = false;
            rack.push(drive);

            // 3: 4-Band EQ (warm low-end boost)
            let mut eq = ParametricEq4Band::new(sample_rate);
            eq.low_shelf.set_gain_db(2.0);
            eq.bypassed = false;
            rack.push(eq);

            // 4: Peak Limiter / Catcher (bypassed by default)
            let mut lim = VocalCompressor::new(sample_rate);
            lim.set_flavor(CompressorFlavor::Fet);
            lim.threshold_db = -6.0;
            lim.ratio = 8.0;
            lim.attack_ms = 1.0;
            lim.release_ms = 40.0;
            lim.bypassed = true;
            rack.push(lim);

            rack
        }
        InstrumentPreset::Keys => {
            let mut rack = MonoRack::with_capacity(16);
            // 0: 4-Band EQ (open top-end)
            let mut eq = ParametricEq4Band::new(sample_rate);
            eq.high_shelf.set_gain_db(1.5);
            eq.bypassed = false;
            rack.push(eq);

            // 1: Compressor (gentle leveling, bypassed)
            let mut comp = VocalCompressor::new(sample_rate);
            comp.threshold_db = -20.0;
            comp.ratio = 2.0;
            comp.bypassed = true;
            rack.push(comp);

            // 2: Harmonic Exciter (sheen, bypassed)
            let mut exc = HarmonicExciter::new(sample_rate);
            exc.bypassed = true;
            rack.push(exc);

            // 3: Analog Chorus (dimensional width)
            let mut chorus = ChorusNode::new(sample_rate);
            chorus.set_params(1.0, 3.5, 0.35, 0.1);
            chorus.bypassed = false;
            rack.push(chorus);

            rack
        }
        InstrumentPreset::Piano => {
            let mut rack = MonoRack::with_capacity(16);
            // 0: 4-Band EQ (clean)
            let eq = ParametricEq4Band::new(sample_rate);
            rack.push(eq);

            // 1: Transparent Compressor (gentle 1.8:1)
            let mut comp = VocalCompressor::new(sample_rate);
            comp.threshold_db = -22.0;
            comp.ratio = 1.8;
            comp.attack_ms = 25.0;
            comp.release_ms = 250.0;
            comp.bypassed = false;
            rack.push(comp);

            // 2: Tape Saturation Warmth (bypassed by default)
            let mut sat = Saturation::new(sample_rate);
            sat.set_flavor(SaturationFlavor::Tape);
            sat.set_params(1.2, 0.6);
            sat.bypassed = true;
            rack.push(sat);

            // 3: Studio Reverb
            let mut rev = ReverbNode::new(sample_rate);
            rev.set_params(0.55, 0.3, 0.18, 15.0);
            rev.bypassed = false;
            rack.push(rev);

            rack
        }
        InstrumentPreset::ProgramThru => {
            // Clean empty rack: passes signal bit-identically directly to master chain
            MonoRack::with_capacity(4)
        }
    }
}

/// Suggested preamp input mode and phantom power setting for the source.
/// UI hint only; hardware mode is never changed without explicit user confirmation.
pub fn suggested_input(preset: InstrumentPreset) -> (PreampMode, bool) {
    match preset {
        InstrumentPreset::Vocal => (PreampMode::Mic, true),
        InstrumentPreset::ElectricGuitar => (PreampMode::HiZ, false),
        InstrumentPreset::AcousticGuitar => (PreampMode::HiZ, false),
        InstrumentPreset::Bass => (PreampMode::HiZ, false),
        InstrumentPreset::Keys => (PreampMode::Line, false),
        InstrumentPreset::Piano => (PreampMode::Line, false),
        InstrumentPreset::ProgramThru => (PreampMode::Line, false),
    }
}
