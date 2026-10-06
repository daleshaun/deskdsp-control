//! Tier A: Lightweight Microphone Voicing Profiles for the Vocal Preset.
//!
//! Provides curated parameter setups for the Vocal chain's HPF, Noise Gate,
//! De-Esser, 4-Band EQ, and Compressor. Pure EQ/dynamics shaping — zero new DSP,
//! zero runtime allocations, instantaneous off-thread parameter updates.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

use super::biquad::{BiquadFilter, FilterType};
use super::channel_strip::ParametricEq4Band;
use super::compressor::{CompressorFlavor, VocalCompressor};
use super::deesser::DeEsser;
use super::gate::NoiseGate;
use super::rack::MonoRack;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MicVoicing {
    #[default]
    Flat,
    WarmCondenser,
    BroadcastDynamic,
    DeskUsbMic,
    Ribbon,
    LavHeadset,
}

impl MicVoicing {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::WarmCondenser => "warm_condenser",
            Self::BroadcastDynamic => "broadcast_dynamic",
            Self::DeskUsbMic => "desk_usb_mic",
            Self::Ribbon => "ribbon",
            Self::LavHeadset => "lav_headset",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Flat => "Flat / Neutral",
            Self::WarmCondenser => "Warm Condenser",
            Self::BroadcastDynamic => "Broadcast Dynamic (SM7B)",
            Self::DeskUsbMic => "Desk / USB Mic",
            Self::Ribbon => "Smooth Ribbon",
            Self::LavHeadset => "Lavalier / Headset",
        }
    }

    pub fn parse_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "flat" | "neutral" | "default" | "none" => Some(Self::Flat),
            "warm_condenser" | "condenser" | "warm" => Some(Self::WarmCondenser),
            "broadcast_dynamic" | "broadcast" | "dynamic" | "sm7b" => Some(Self::BroadcastDynamic),
            "desk_usb_mic" | "desk" | "usb" => Some(Self::DeskUsbMic),
            "ribbon" => Some(Self::Ribbon),
            "lav_headset" | "lav" | "headset" => Some(Self::LavHeadset),
            _ => None,
        }
    }
}

/// Applies a Tier A microphone voicing profile to the Vocal rack.
/// Safe and allocation-free: modifies existing parameters via typed node queries.
pub fn apply_mic_voicing(rack: &mut MonoRack, voicing: MicVoicing, _sample_rate: f32) {
    match voicing {
        MicVoicing::Flat => {
            if let Some(hpf) = rack.find_node_mut::<BiquadFilter>() {
                hpf.filter_type = FilterType::HighPass;
                hpf.set_cutoff(20.0);
                hpf.bypassed = true;
            }
            if let Some(gate) = rack.find_node_mut::<NoiseGate>() {
                gate.threshold_db = -65.0;
                gate.bypassed = true;
            }
            if let Some(deesser) = rack.find_node_mut::<DeEsser>() {
                deesser.set_params(-24.0, deesser.ratio, 6000.0);
                deesser.bypassed = true;
            }
            if let Some(eq) = rack.find_node_mut::<ParametricEq4Band>() {
                eq.low_shelf.set_cutoff(100.0);
                eq.low_shelf.set_gain_db(0.0);
                eq.low_mid.set_cutoff(450.0);
                eq.low_mid.set_gain_db(0.0);
                eq.high_mid.set_cutoff(3200.0);
                eq.high_mid.set_gain_db(0.0);
                eq.high_shelf.set_cutoff(10000.0);
                eq.high_shelf.set_gain_db(0.0);
                eq.bypassed = false;
            }
            if let Some(comp) = rack.find_node_mut::<VocalCompressor>() {
                comp.set_params(-18.0, 3.0, comp.attack_ms, comp.release_ms, comp.makeup_db);
                comp.bypassed = true;
            }
        }
        MicVoicing::WarmCondenser => {
            if let Some(hpf) = rack.find_node_mut::<BiquadFilter>() {
                hpf.filter_type = FilterType::HighPass;
                hpf.set_cutoff(75.0); // Gentle low rumble cut
                hpf.bypassed = false;
            }
            if let Some(gate) = rack.find_node_mut::<NoiseGate>() {
                gate.threshold_db = -62.0;
                gate.bypassed = true;
            }
            if let Some(deesser) = rack.find_node_mut::<DeEsser>() {
                deesser.set_params(-22.0, deesser.ratio, 6500.0);
                deesser.bypassed = false; // Condensers have bright high-end
            }
            if let Some(eq) = rack.find_node_mut::<ParametricEq4Band>() {
                eq.low_shelf.set_cutoff(120.0);
                eq.low_shelf.set_gain_db(1.0); // Warm body
                eq.low_mid.set_cutoff(400.0);
                eq.low_mid.set_gain_db(-1.0); // Clear mud
                eq.high_mid.set_cutoff(3500.0);
                eq.high_mid.set_gain_db(0.5);
                eq.high_shelf.set_cutoff(10000.0);
                eq.high_shelf.set_gain_db(2.0); // Open airy sheen
                eq.bypassed = false;
            }
            if let Some(comp) = rack.find_node_mut::<VocalCompressor>() {
                comp.set_flavor(CompressorFlavor::Opto);
                comp.set_params(-18.0, 2.5, 15.0, 120.0, comp.makeup_db);
                comp.bypassed = false;
            }
        }
        MicVoicing::BroadcastDynamic => {
            if let Some(hpf) = rack.find_node_mut::<BiquadFilter>() {
                hpf.filter_type = FilterType::HighPass;
                hpf.set_cutoff(110.0); // Tight low-cut for close broadcast speech
                hpf.bypassed = false;
            }
            if let Some(gate) = rack.find_node_mut::<NoiseGate>() {
                gate.threshold_db = -58.0;
                gate.bypassed = false;
            }
            if let Some(deesser) = rack.find_node_mut::<DeEsser>() {
                deesser.set_params(-20.0, deesser.ratio, 5500.0);
                deesser.bypassed = false;
            }
            if let Some(eq) = rack.find_node_mut::<ParametricEq4Band>() {
                eq.low_shelf.set_cutoff(150.0);
                eq.low_shelf.set_gain_db(-2.0); // Tame heavy proximity boom
                eq.low_mid.set_cutoff(350.0);
                eq.low_mid.set_gain_db(-2.5); // De-boxiness
                eq.high_mid.set_cutoff(4500.0);
                eq.high_mid.set_gain_db(3.5); // Broadcast speech presence
                eq.high_shelf.set_cutoff(10000.0);
                eq.high_shelf.set_gain_db(2.0);
                eq.bypassed = false;
            }
            if let Some(comp) = rack.find_node_mut::<VocalCompressor>() {
                comp.set_flavor(CompressorFlavor::Fet);
                comp.set_params(-16.0, 4.0, 8.0, 80.0, comp.makeup_db);
                comp.bypassed = false;
            }
        }
        MicVoicing::DeskUsbMic => {
            if let Some(hpf) = rack.find_node_mut::<BiquadFilter>() {
                hpf.filter_type = FilterType::HighPass;
                hpf.set_cutoff(130.0); // Aggressive cut for desk thumps & AC rumble
                hpf.bypassed = false;
            }
            if let Some(gate) = rack.find_node_mut::<NoiseGate>() {
                gate.threshold_db = -52.0; // Strong gate for keyboard clicks
                gate.bypassed = false;
            }
            if let Some(deesser) = rack.find_node_mut::<DeEsser>() {
                deesser.set_params(-18.0, deesser.ratio, 6000.0);
                deesser.bypassed = false;
            }
            if let Some(eq) = rack.find_node_mut::<ParametricEq4Band>() {
                eq.low_shelf.set_cutoff(200.0);
                eq.low_shelf.set_gain_db(-3.0); // Desk resonance notch
                eq.low_mid.set_cutoff(500.0);
                eq.low_mid.set_gain_db(-3.0); // Hollow room boxiness cut
                eq.high_mid.set_cutoff(3000.0);
                eq.high_mid.set_gain_db(1.5); // Speech clarity
                eq.high_shelf.set_cutoff(8000.0);
                eq.high_shelf.set_gain_db(-1.5); // Tame harsh cheap capsule top
                eq.bypassed = false;
            }
            if let Some(comp) = rack.find_node_mut::<VocalCompressor>() {
                comp.set_flavor(CompressorFlavor::Fet);
                comp.set_params(-15.0, 3.0, 10.0, 100.0, comp.makeup_db);
                comp.bypassed = false;
            }
        }
        MicVoicing::Ribbon => {
            if let Some(hpf) = rack.find_node_mut::<BiquadFilter>() {
                hpf.filter_type = FilterType::HighPass;
                hpf.set_cutoff(60.0);
                hpf.bypassed = false;
            }
            if let Some(gate) = rack.find_node_mut::<NoiseGate>() {
                gate.threshold_db = -65.0;
                gate.bypassed = true;
            }
            if let Some(deesser) = rack.find_node_mut::<DeEsser>() {
                deesser.bypassed = true; // Ribbons are naturally gentle on sibilance
            }
            if let Some(eq) = rack.find_node_mut::<ParametricEq4Band>() {
                eq.low_shelf.set_cutoff(100.0);
                eq.low_shelf.set_gain_db(-1.5); // Counter massive figure-8 proximity
                eq.low_mid.set_cutoff(450.0);
                eq.low_mid.set_gain_db(0.0);
                eq.high_mid.set_cutoff(3500.0);
                eq.high_mid.set_gain_db(2.0); // Natural vocal articulation
                eq.high_shelf.set_cutoff(12000.0);
                eq.high_shelf.set_gain_db(4.5); // Sweet top shelf countering ribbon roll-off
                eq.bypassed = false;
            }
            if let Some(comp) = rack.find_node_mut::<VocalCompressor>() {
                comp.set_flavor(CompressorFlavor::Opto);
                comp.set_params(-20.0, 2.0, 20.0, 150.0, comp.makeup_db);
                comp.bypassed = false;
            }
        }
        MicVoicing::LavHeadset => {
            if let Some(hpf) = rack.find_node_mut::<BiquadFilter>() {
                hpf.filter_type = FilterType::HighPass;
                hpf.set_cutoff(120.0);
                hpf.bypassed = false;
            }
            if let Some(gate) = rack.find_node_mut::<NoiseGate>() {
                gate.threshold_db = -50.0; // Strong gate for stage/room noise
                gate.bypassed = false;
            }
            if let Some(deesser) = rack.find_node_mut::<DeEsser>() {
                deesser.set_params(-18.0, deesser.ratio, 5000.0);
                deesser.bypassed = false;
            }
            if let Some(eq) = rack.find_node_mut::<ParametricEq4Band>() {
                eq.low_shelf.set_cutoff(200.0);
                eq.low_shelf.set_gain_db(2.5); // Body compensation for tiny electret capsule
                eq.low_mid.set_cutoff(800.0);
                eq.low_mid.set_gain_db(-2.0); // Nasal cut
                eq.high_mid.set_cutoff(3500.0);
                eq.high_mid.set_gain_db(-2.5); // Harsh ear-canal resonance cut
                eq.high_shelf.set_cutoff(9000.0);
                eq.high_shelf.set_gain_db(1.5);
                eq.bypassed = false;
            }
            if let Some(comp) = rack.find_node_mut::<VocalCompressor>() {
                comp.set_flavor(CompressorFlavor::Fet);
                comp.set_params(-14.0, 3.5, 5.0, 60.0, comp.makeup_db);
                comp.bypassed = false;
            }
        }
    }
}
