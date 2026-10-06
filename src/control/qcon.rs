//! iCON QCon Pro G2 Control Surface Driver (Mackie Control Universal / MCU).
//!
//! Full Bidirectional Implementation:
//! - 8 Motorized Faders + Master Fader (Inbound 14-bit Pitch Bend & Outbound Motor Feedback)
//! - 8 V-Pots (Relative 2's-complement continuous encoders & Outbound LED Ring Feedback)
//! - Channel & Transport Buttons: Mute, Select, Solo, Rec, Play, Stop, Cycle
//! - Outbound LED Feedback (Note On 0x7F / 0x00 for hardware buttons)

#![allow(dead_code)]

use super::{ControlSurface, SurfaceCommand, SurfaceState, Target};
use crate::dsp::InstrumentPreset;

/// iCON QCon Pro G2 MCU Driver.
#[derive(Debug, Clone)]
pub struct QConSurface {
    pub selected_channel: Target,
    pub ch1_bypassed: bool,
    pub ch2_bypassed: bool,
    pub global_bypassed: bool,
    pub source_mode: String,

    // Cached state to prevent redundant MIDI traffic / motor hunting
    last_fader_pitch_bend: [u16; 9],
    last_led_state: [i8; 128],
    last_vpot_led: [i8; 8],

    // Cached values for 8 V-pot parameters on the active channel
    vpot_values: [f32; 8],
}

impl Default for QConSurface {
    fn default() -> Self {
        Self {
            selected_channel: Target::Channel1,
            ch1_bypassed: false,
            ch2_bypassed: false,
            global_bypassed: false,
            source_mode: "vocal".into(),
            last_fader_pitch_bend: [0xFFFF; 9], // Force initial feedback
            last_led_state: [-1; 128],          // -1 = uninitialized
            last_vpot_led: [-1; 8],             // -1 = uninitialized
            vpot_values: [
                -30.0, // Gate threshold (-60..0)
                -18.0, // Comp threshold (-50..0)
                0.0,   // Sat drive (0..24)
                0.0,   // EQ low gain (-12..12)
                0.0,   // EQ lmid gain (-12..12)
                0.0,   // EQ hmid gain (-12..12)
                0.0,   // EQ hi gain (-12..12)
                2.0,   // Amp drive / Tuner speed (0..10)
            ],
        }
    }
}

impl QConSurface {
    pub const VPOT_DESCRIPTORS: [(&'static str, f32, f32, f32); 8] = [
        ("gate_threshold", -60.0, 0.0, 1.0),
        ("comp_threshold", -50.0, 0.0, 1.0),
        ("sat_drive", 0.0, 24.0, 0.5),
        ("eq_low_gain", -12.0, 12.0, 0.5),
        ("eq_lmid_gain", -12.0, 12.0, 0.5),
        ("eq_hmid_gain", -12.0, 12.0, 0.5),
        ("eq_hi_gain", -12.0, 12.0, 0.5),
        ("amp_drive", 0.0, 10.0, 0.25),
    ];

    /// Scales gain dB (0..65 dB) into MCU 14-bit pitch bend (0..16383).
    pub fn gain_to_pitch_bend(gain_db: u8) -> u16 {
        let norm = (gain_db.min(65) as f32) / 65.0;
        (norm * 16383.0).round() as u16
    }

    /// Converts MCU 14-bit pitch bend into preamp gain dB (0..65 dB).
    pub fn pitch_bend_to_gain(pb: u16) -> u8 {
        let norm = (pb.min(16383) as f32) / 16383.0;
        (norm * 65.0).round() as u8
    }

    /// Converts monitor volume step (0 = Unity 0dB, 127 = Mute) to 14-bit pitch bend.
    pub fn monitor_vol_to_pitch_bend(step: u8) -> u16 {
        let inv_norm = 1.0 - (step.min(127) as f32 / 127.0);
        (inv_norm * 16383.0).round() as u16
    }

    /// Converts 14-bit pitch bend to monitor volume step (0..127).
    pub fn pitch_bend_to_monitor_vol(pb: u16) -> u8 {
        let norm = (pb.min(16383) as f32) / 16383.0;
        127 - (norm * 127.0).round() as u8
    }

    /// Decodes relative 2's-complement / sign-magnitude encoder value.
    /// Clockwise: 0x01..0x3F (+1..+63), Counter-clockwise: 0x41..0x7F (-1..-63).
    pub fn decode_vpot_delta(val: u8) -> i32 {
        if val & 0x40 != 0 {
            // Negative increment
            -((val & 0x3F) as i32)
        } else {
            // Positive increment
            (val & 0x3F) as i32
        }
    }
}

impl ControlSurface for QConSurface {
    fn name(&self) -> &'static str {
        "iCON QCon Pro G2 (Mackie Control)"
    }

    fn on_midi(&mut self, msg: &[u8]) -> Vec<SurfaceCommand> {
        if msg.len() < 3 {
            return Vec::new();
        }

        let status = msg[0];
        let d1 = msg[1];
        let d2 = msg[2];

        let mut commands = Vec::new();

        // 1. Motorized Faders: Inbound Pitch Bend on channels 0..8
        if (0xE0..=0xE8).contains(&status) {
            let fader_idx = (status - 0xE0) as usize;
            let lsb = d1 as u16;
            let msb = d2 as u16;
            let pb_val = (msb << 7) | lsb;

            match fader_idx {
                // Fader 0: Channel 1 Preamp Gain
                0 => {
                    let gain_db = Self::pitch_bend_to_gain(pb_val);
                    commands.push(SurfaceCommand::Gain { input: 1, gain_db });
                }
                // Fader 1: Channel 2 Preamp Gain
                1 => {
                    let gain_db = Self::pitch_bend_to_gain(pb_val);
                    commands.push(SurfaceCommand::Gain { input: 2, gain_db });
                }
                // Faders 2..7: Key DSP parameters on active channel
                2 => {
                    let norm = (pb_val as f32) / 16383.0;
                    let comp_thresh = -50.0 + norm * 50.0; // -50 dB .. 0 dB
                    commands.push(SurfaceCommand::DspParam {
                        target: self.selected_channel,
                        param_id: "comp_threshold",
                        value: comp_thresh,
                    });
                }
                3 => {
                    let norm = (pb_val as f32) / 16383.0;
                    let sat_drive = norm * 24.0; // 0 dB .. 24 dB
                    commands.push(SurfaceCommand::DspParam {
                        target: self.selected_channel,
                        param_id: "sat_drive",
                        value: sat_drive,
                    });
                }
                4 => {
                    let norm = (pb_val as f32) / 16383.0;
                    let eq_low = -12.0 + norm * 24.0; // -12 dB .. +12 dB
                    commands.push(SurfaceCommand::DspParam {
                        target: self.selected_channel,
                        param_id: "eq_low_gain",
                        value: eq_low,
                    });
                }
                5 => {
                    let norm = (pb_val as f32) / 16383.0;
                    let eq_mid = -12.0 + norm * 24.0;
                    commands.push(SurfaceCommand::DspParam {
                        target: self.selected_channel,
                        param_id: "eq_hmid_gain",
                        value: eq_mid,
                    });
                }
                6 => {
                    let norm = (pb_val as f32) / 16383.0;
                    let eq_hi = -12.0 + norm * 24.0;
                    commands.push(SurfaceCommand::DspParam {
                        target: self.selected_channel,
                        param_id: "eq_hi_gain",
                        value: eq_hi,
                    });
                }
                7 => {
                    let norm = (pb_val as f32) / 16383.0;
                    let amp = norm * 10.0;
                    commands.push(SurfaceCommand::DspParam {
                        target: self.selected_channel,
                        param_id: "amp_drive",
                        value: amp,
                    });
                }
                // Master Fader (Channel 8 / 0xE8): Monitor Volume
                8 => {
                    let step = Self::pitch_bend_to_monitor_vol(pb_val);
                    commands.push(SurfaceCommand::MonitorVolume(step));
                }
                _ => {}
            }
        }

        // 2. V-Pots (Continuous Rotary Encoders): Inbound CC 0x10..0x17
        else if status == 0xB0 {
            let cc = d1;
            let val = d2;

            if (0x10..=0x17).contains(&cc) {
                let vpot_idx = (cc - 0x10) as usize;
                let delta = Self::decode_vpot_delta(val);
                let (param_id, min_val, max_val, step_size) = Self::VPOT_DESCRIPTORS[vpot_idx];

                let cur_val = self.vpot_values[vpot_idx];
                let new_val = (cur_val + (delta as f32 * step_size)).clamp(min_val, max_val);
                self.vpot_values[vpot_idx] = new_val;

                commands.push(SurfaceCommand::DspParam {
                    target: self.selected_channel,
                    param_id,
                    value: new_val,
                });
            }
        }

        // 3. Channel & Transport Buttons: Inbound Note On
        else if status == 0x90 {
            let note = d1;
            let velocity = d2;

            // Only act on button press (velocity > 0)
            if velocity > 0 {
                match note {
                    // Mute Buttons: Notes 0x10..0x17 (16..23)
                    0x10 => {
                        self.ch1_bypassed = !self.ch1_bypassed;
                        commands.push(SurfaceCommand::NodeBypass {
                            target: Target::Channel1,
                            node: "channel",
                            bypassed: self.ch1_bypassed,
                        });
                    }
                    0x11 => {
                        self.ch2_bypassed = !self.ch2_bypassed;
                        commands.push(SurfaceCommand::NodeBypass {
                            target: Target::Channel2,
                            node: "channel",
                            bypassed: self.ch2_bypassed,
                        });
                    }
                    0x12 => commands.push(SurfaceCommand::NodeBypass {
                        target: self.selected_channel,
                        node: "tuner",
                        bypassed: true,
                    }),
                    0x13 => commands.push(SurfaceCommand::NodeBypass {
                        target: self.selected_channel,
                        node: "gate",
                        bypassed: true,
                    }),
                    0x14 => commands.push(SurfaceCommand::NodeBypass {
                        target: self.selected_channel,
                        node: "eq",
                        bypassed: true,
                    }),
                    0x15 => commands.push(SurfaceCommand::NodeBypass {
                        target: self.selected_channel,
                        node: "comp",
                        bypassed: true,
                    }),
                    0x16 => commands.push(SurfaceCommand::NodeBypass {
                        target: self.selected_channel,
                        node: "drive",
                        bypassed: true,
                    }),
                    0x17 => commands.push(SurfaceCommand::NodeBypass {
                        target: self.selected_channel,
                        node: "reverb",
                        bypassed: true,
                    }),

                    // Select Buttons: Notes 0x18..0x1F (24..31)
                    0x18 => self.selected_channel = Target::Channel1,
                    0x19 => self.selected_channel = Target::Channel2,

                    // Transport Buttons
                    0x5E => {
                        // Play: Toggle Source Mode (Program vs Vocal)
                        let new_mode = if self.source_mode == "program" { "vocal" } else { "program" };
                        self.source_mode = new_mode.into();
                        commands.push(SurfaceCommand::SourceMode(new_mode.into()));
                    }
                    0x5D => {
                        // Stop: Toggle Global Bypass
                        self.global_bypassed = !self.global_bypassed;
                        commands.push(SurfaceCommand::GlobalBypass(self.global_bypassed));
                    }
                    0x56 => {
                        // Cycle: Next Instrument Preset
                        commands.push(SurfaceCommand::ChannelPreset {
                            target: self.selected_channel,
                            preset: InstrumentPreset::Vocal, // Runner handles stateful step
                        });
                    }

                    _ => {}
                }
            }
        }

        commands
    }

    fn feedback(&mut self, state: &SurfaceState) -> Vec<Vec<u8>> {
        let mut messages = Vec::new();

        // 1. Motorized Fader Feedback (Outbound Pitch Bend on channels 0..8)
        let ch1_pb = Self::gain_to_pitch_bend(state.ch1_gain_db);
        let ch2_pb = Self::gain_to_pitch_bend(state.ch2_gain_db);
        let master_pb = Self::monitor_vol_to_pitch_bend(state.monitor_step);

        let target_faders = [
            ch1_pb,
            ch2_pb,
            self.last_fader_pitch_bend[2].min(16383),
            self.last_fader_pitch_bend[3].min(16383),
            self.last_fader_pitch_bend[4].min(16383),
            self.last_fader_pitch_bend[5].min(16383),
            self.last_fader_pitch_bend[6].min(16383),
            self.last_fader_pitch_bend[7].min(16383),
            master_pb,
        ];

        for i in 0..9 {
            let target_val = target_faders[i];
            let last_val = self.last_fader_pitch_bend[i];

            // 16-count deadband prevents servo motor jitter
            if last_val == 0xFFFF || (target_val as i32 - last_val as i32).abs() > 16 {
                let status = 0xE0 + i as u8;
                let lsb = (target_val & 0x7F) as u8;
                let msb = ((target_val >> 7) & 0x7F) as u8;
                messages.push(vec![status, lsb, msb]);
                self.last_fader_pitch_bend[i] = target_val;
            }
        }

        // 2. Button LED Feedback (Outbound Note On 0x90, note, velocity 0x7F / 0x00)
        let led_targets: &[(u8, bool)] = &[
            (0x10, state.ch1_bypassed),              // Mute 1
            (0x11, state.ch2_bypassed),              // Mute 2
            (0x18, state.selected_channel == 0),     // Select 1
            (0x19, state.selected_channel == 1),     // Select 2
            (0x5D, state.global_bypass),             // Stop / Global Bypass
            (0x5E, state.source_mode == "program"),  // Play / Program Mode
        ];

        for &(note, active) in led_targets {
            let idx = note as usize;
            let current_val = if active { 1 } else { 0 };
            if self.last_led_state[idx] != current_val {
                let velocity = if active { 0x7F } else { 0x00 };
                messages.push(vec![0x90, note, velocity]);
                self.last_led_state[idx] = current_val;
            }
        }

        // 3. V-Pot LED Ring Feedback (Outbound CC 0x30..0x37)
        for i in 0..8 {
            let (min_val, max_val) = (Self::VPOT_DESCRIPTORS[i].1, Self::VPOT_DESCRIPTORS[i].2);
            let cur = self.vpot_values[i];
            let norm = ((cur - min_val) / (max_val - min_val)).clamp(0.0, 1.0);
            let ring_val = (norm * 11.0).round() as u8; // 11 LED arc positions

            if self.last_vpot_led[i] != ring_val as i8 {
                let cc = 0x30 + i as u8;
                messages.push(vec![0xB0, cc, ring_val]);
                self.last_vpot_led[i] = ring_val as i8;
            }
        }

        messages
    }
}
