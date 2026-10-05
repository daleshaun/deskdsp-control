//! Preset Management for DeskDSP Control (Channel Strip and Master Chain configurations).

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use anyhow::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStripPreset {
    pub name: String,
    pub input_gain_db: f32,
    pub phase_invert: bool,
    pub hpf_cutoff_hz: f32,
    pub hpf_bypassed: bool,
    pub gate_threshold_db: f32,
    pub gate_ratio: f32,
    pub gate_bypassed: bool,
    pub deesser_threshold_db: f32,
    pub deesser_freq_hz: f32,
    pub deesser_bypassed: bool,
    pub eq_bypassed: bool,
    pub comp_threshold_db: f32,
    pub comp_ratio: f32,
    pub comp_makeup_db: f32,
    pub comp_bypassed: bool,
    pub tuner_key_root: u8,
    pub tuner_scale: u8,
    pub tuner_retune_speed_ms: f32,
    pub tuner_strength: f32,
    pub tuner_bypassed: bool,
    pub sat_drive: f32,
    pub sat_bypassed: bool,
    pub output_gain_db: f32,
}

impl Default for ChannelStripPreset {
    fn default() -> Self {
        Self {
            name: "Default Vocal Tracking".into(),
            input_gain_db: 0.0,
            phase_invert: false,
            hpf_cutoff_hz: 80.0,
            hpf_bypassed: false,
            gate_threshold_db: -55.0,
            gate_ratio: 4.0,
            gate_bypassed: false,
            deesser_threshold_db: -20.0,
            deesser_freq_hz: 6500.0,
            deesser_bypassed: false,
            eq_bypassed: false,
            comp_threshold_db: -18.0,
            comp_ratio: 4.0,
            comp_makeup_db: 3.0,
            comp_bypassed: false,
            tuner_key_root: 0, // C
            tuner_scale: 0,    // Chromatic
            tuner_retune_speed_ms: 20.0,
            tuner_strength: 0.85,
            tuner_bypassed: false,
            sat_drive: 2.0,
            sat_bypassed: false,
            output_gain_db: 0.0,
        }
    }
}

pub struct PresetManager;

impl PresetManager {
    pub fn save<P: AsRef<Path>>(preset: &ChannelStripPreset, path: P) -> Result<()> {
        let json = serde_json::to_string_pretty(preset)?;
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load<P: AsRef<Path>>(path: P) -> Result<ChannelStripPreset> {
        let content = fs::read_to_string(path)?;
        let preset: ChannelStripPreset = serde_json::from_str(&content)?;
        Ok(preset)
    }

    pub fn modern_pop_vocal() -> ChannelStripPreset {
        ChannelStripPreset {
            name: "Modern Pop Vocal".into(),
            input_gain_db: 0.0,
            phase_invert: false,
            hpf_cutoff_hz: 85.0,
            hpf_bypassed: false,
            gate_threshold_db: -50.0,
            gate_ratio: 3.5,
            gate_bypassed: false,
            deesser_threshold_db: -18.0,
            deesser_freq_hz: 6800.0,
            deesser_bypassed: false,
            eq_bypassed: false,
            comp_threshold_db: -20.0,
            comp_ratio: 4.5,
            comp_makeup_db: 4.0,
            comp_bypassed: false,
            tuner_key_root: 9, // A
            tuner_scale: 2,    // Natural Minor
            tuner_retune_speed_ms: 12.0, // tight pop tuning
            tuner_strength: 0.95,
            tuner_bypassed: false,
            sat_drive: 2.2,
            sat_bypassed: false,
            output_gain_db: 0.0,
        }
    }

    pub fn warm_tube_vocal() -> ChannelStripPreset {
        ChannelStripPreset {
            name: "Warm Tube Vocal".into(),
            input_gain_db: 0.0,
            phase_invert: false,
            hpf_cutoff_hz: 75.0,
            hpf_bypassed: false,
            gate_threshold_db: -60.0,
            gate_ratio: 2.0,
            gate_bypassed: false,
            deesser_threshold_db: -22.0,
            deesser_freq_hz: 6200.0,
            deesser_bypassed: false,
            eq_bypassed: false,
            comp_threshold_db: -14.0,
            comp_ratio: 3.0,
            comp_makeup_db: 2.5,
            comp_bypassed: false,
            tuner_key_root: 0,
            tuner_scale: 0,
            tuner_retune_speed_ms: 40.0, // gentle natural glide
            tuner_strength: 0.65,
            tuner_bypassed: false,
            sat_drive: 4.5, // rich tube saturation
            sat_bypassed: false,
            output_gain_db: -1.0,
        }
    }
}
