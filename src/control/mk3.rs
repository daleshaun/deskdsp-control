//! Native Instruments Maschine MK3 Control Surface Driver (MIDI Mode).
//!
//! Input-only controller mapping:
//! - 16 Pads (Notes 36–51): Real-time bypass toggles & triggers
//! - 8 Knobs (CC 16–23): 3-bank paged DSP parameter continuous controls
//! - Buttons: Channel selection (CH1/CH2), Bank cycle, Global Bypass, Preset stepping.

#![allow(dead_code)]

use super::{ControlSurface, SurfaceCommand, SurfaceState, Target};
use crate::dsp::InstrumentPreset;

/// Descriptor for a controllable DSP parameter on a rotary knob.
#[derive(Debug, Clone, Copy)]
pub struct ParamDescriptor {
    pub id: &'static str,
    pub min: f32,
    pub max: f32,
}

impl ParamDescriptor {
    pub const fn new(id: &'static str, min: f32, max: f32) -> Self {
        Self { id, min, max }
    }

    #[inline]
    pub fn scale(&self, val_7bit: u8) -> f32 {
        let norm = (val_7bit.min(127) as f32) / 127.0;
        self.min + norm * (self.max - self.min)
    }
}

/// Maschine MK3 Control Surface driver.
#[derive(Debug, Clone)]
pub struct Mk3Surface {
    pub selected_channel: Target,
    pub current_bank: usize,
    pub global_bypassed: bool,
    pub node_bypasses: [bool; 10], // Tracks bypass states for pads 0..9
}

impl Default for Mk3Surface {
    fn default() -> Self {
        Self {
            selected_channel: Target::Channel1,
            current_bank: 0,
            global_bypassed: false,
            node_bypasses: [false; 10],
        }
    }
}

impl Mk3Surface {
    pub const NUM_BANKS: usize = 3;

    /// Parameter mapping table for 8 knobs across 3 banks.
    pub const BANK_PARAMS: [[ParamDescriptor; 8]; Self::NUM_BANKS] = [
        // Bank 0: Dynamics, Gate & Pitch Tracking
        [
            ParamDescriptor::new("gate_threshold", -60.0, 0.0),
            ParamDescriptor::new("comp_threshold", -50.0, 0.0),
            ParamDescriptor::new("comp_ratio", 1.0, 20.0),
            ParamDescriptor::new("comp_attack", 0.1, 200.0),
            ParamDescriptor::new("comp_release", 10.0, 1000.0),
            ParamDescriptor::new("deess_amount", 0.0, 12.0),
            ParamDescriptor::new("sat_drive", 0.0, 24.0),
            ParamDescriptor::new("tuner_retune", 1.0, 100.0),
        ],
        // Bank 1: 4-Band EQ & Filtering
        [
            ParamDescriptor::new("hpf_freq", 20.0, 300.0),
            ParamDescriptor::new("eq_low_gain", -12.0, 12.0),
            ParamDescriptor::new("eq_lmid_gain", -12.0, 12.0),
            ParamDescriptor::new("eq_hmid_gain", -12.0, 12.0),
            ParamDescriptor::new("eq_hi_gain", -12.0, 12.0),
            ParamDescriptor::new("master_eq_low", -6.0, 6.0),
            ParamDescriptor::new("master_eq_mid", -6.0, 6.0),
            ParamDescriptor::new("master_eq_high", -6.0, 6.0),
        ],
        // Bank 2: Amps, Drives & Modulation FX
        [
            ParamDescriptor::new("amp_drive", 0.0, 10.0),
            ParamDescriptor::new("amp_level", 0.0, 10.0),
            ParamDescriptor::new("drive_gain", 0.0, 10.0),
            ParamDescriptor::new("drive_blend", 0.0, 1.0),
            ParamDescriptor::new("chorus_mix", 0.0, 1.0),
            ParamDescriptor::new("reverb_mix", 0.0, 1.0),
            ParamDescriptor::new("mic_dry_wet", 0.0, 1.0),
            ParamDescriptor::new("stereo_width", 0.0, 2.0),
        ],
    ];

    /// Returns the target DSP node ID for pads 0..9.
    pub fn pad_node_id(pad_idx: usize) -> Option<&'static str> {
        match pad_idx {
            0 => Some("tuner"),
            1 => Some("gate"),
            2 => Some("deesser"),
            3 => Some("eq"),
            4 => Some("comp"),
            5 => Some("sat"),
            6 => Some("drive"),
            7 => Some("chorus"),
            8 => Some("reverb"),
            9 => Some("mic_image"),
            _ => None,
        }
    }

    /// Cycles through instrument presets sequentially.
    pub fn next_preset(current: InstrumentPreset) -> InstrumentPreset {
        match current {
            InstrumentPreset::Vocal => InstrumentPreset::AcousticGuitar,
            InstrumentPreset::AcousticGuitar => InstrumentPreset::ElectricGuitar,
            InstrumentPreset::ElectricGuitar => InstrumentPreset::Bass,
            InstrumentPreset::Bass => InstrumentPreset::Keys,
            InstrumentPreset::Keys => InstrumentPreset::Piano,
            InstrumentPreset::Piano => InstrumentPreset::ProgramThru,
            InstrumentPreset::ProgramThru => InstrumentPreset::Vocal,
        }
    }
}

impl ControlSurface for Mk3Surface {
    fn name(&self) -> &'static str {
        "Native Instruments Maschine MK3"
    }

    fn on_midi(&mut self, msg: &[u8]) -> Vec<SurfaceCommand> {
        if msg.len() < 3 {
            return Vec::new();
        }

        let status = msg[0] & 0xF0;
        let d1 = msg[1];
        let d2 = msg[2];

        let mut commands = Vec::new();

        match status {
            // Note On (Pad or Button pressed)
            0x90 => {
                let velocity = d2;
                if velocity > 0 {
                    // Pads 0..15: Notes 36 to 51 (0x24 to 0x33)
                    if (36..=51).contains(&d1) {
                        let pad_idx = (d1 - 36) as usize;
                        commands.push(SurfaceCommand::Trigger {
                            pad: pad_idx as u8,
                            velocity,
                        });

                        // Pads 0..9 toggle node bypass
                        if let Some(node) = Self::pad_node_id(pad_idx) {
                            self.node_bypasses[pad_idx] = !self.node_bypasses[pad_idx];
                            commands.push(SurfaceCommand::NodeBypass {
                                target: self.selected_channel,
                                node,
                                bypassed: self.node_bypasses[pad_idx],
                            });
                        }
                    }
                    // Button: Note 52 (0x34) -> Toggle Selected Channel
                    else if d1 == 52 {
                        self.selected_channel = match self.selected_channel {
                            Target::Channel1 => Target::Channel2,
                            _ => Target::Channel1,
                        };
                    }
                    // Button: Note 53 (0x35) -> Cycle Bank
                    else if d1 == 53 {
                        self.current_bank = (self.current_bank + 1) % Self::NUM_BANKS;
                    }
                    // Button: Note 54 (0x36) -> Toggle Global Bypass
                    else if d1 == 54 {
                        self.global_bypassed = !self.global_bypassed;
                        commands.push(SurfaceCommand::GlobalBypass(self.global_bypassed));
                    }
                    // Button: Note 55 (0x37) -> Preset Step
                    else if d1 == 55 {
                        commands.push(SurfaceCommand::ChannelPreset {
                            target: self.selected_channel,
                            preset: InstrumentPreset::Vocal, // Runner will handle stateful cycling
                        });
                    }
                }
            }

            // Control Change (Rotary Knobs & Shift Buttons)
            0xB0 => {
                let cc = d1;
                let val = d2;

                // 8 Knobs: CC 16 to 23 (0x10 to 0x17)
                if (16..=23).contains(&cc) {
                    let knob_idx = (cc - 16) as usize;
                    let desc = &Self::BANK_PARAMS[self.current_bank][knob_idx];
                    let scaled_val = desc.scale(val);

                    commands.push(SurfaceCommand::DspParam {
                        target: self.selected_channel,
                        param_id: desc.id,
                        value: scaled_val,
                    });
                }
                // Alternative CC Buttons
                else if cc == 28 && val > 63 {
                    self.selected_channel = match self.selected_channel {
                        Target::Channel1 => Target::Channel2,
                        _ => Target::Channel1,
                    };
                } else if cc == 29 && val > 63 {
                    self.current_bank = (self.current_bank + 1) % Self::NUM_BANKS;
                } else if cc == 30 && val > 63 {
                    self.global_bypassed = !self.global_bypassed;
                    commands.push(SurfaceCommand::GlobalBypass(self.global_bypassed));
                }
            }

            _ => {}
        }

        commands
    }

    fn feedback(&mut self, _state: &SurfaceState) -> Vec<Vec<u8>> {
        // MK3 in generic MIDI mode is input-only
        Vec::new()
    }
}
