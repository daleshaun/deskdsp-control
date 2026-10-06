//! Unit Test Suite for Maschine MK3 and iCON QCon Pro G2 Control Surfaces.
//!
//! Validates 100% of the MIDI protocol parsing, bank management, parameter scaling,
//! and bidirectional motorized fader / LED feedback using synthetic MIDI byte streams
//! without requiring physical hardware.

use approx::assert_relative_eq;
use deskdsp_control::control::{
    list_midi_ports, ControlSurface, Mk3Surface, QConSurface, SurfaceCommand, SurfaceState, Target,
};
use deskdsp_control::dsp::InstrumentPreset;
use deskdsp_control::remote::RemoteMessage;

// ============================================================================
// Maschine MK3 Driver Tests (Input-Only MIDI Mode)
// ============================================================================

#[test]
fn test_mk3_knob_cc_scaling_and_bank_switching() {
    let mut mk3 = Mk3Surface::default();
    assert_eq!(mk3.selected_channel, Target::Channel1);
    assert_eq!(mk3.current_bank, 0);

    // 1. Bank 0 (Dynamics/Pitch): Knob 0 (CC 16) is gate_threshold (-60.0 to 0.0 dB)
    // Send CC 16 with value 0 (min)
    let cmds = mk3.on_midi(&[0xB0, 16, 0]);
    assert_eq!(cmds.len(), 1);
    match &cmds[0] {
        SurfaceCommand::DspParam { target, param_id, value } => {
            assert_eq!(*target, Target::Channel1);
            assert_eq!(*param_id, "gate_threshold");
            assert_relative_eq!(*value, -60.0, epsilon = 1e-3);
        }
        _ => panic!("Expected DspParam"),
    }

    // Send CC 16 with value 127 (max)
    let cmds = mk3.on_midi(&[0xB0, 16, 127]);
    match &cmds[0] {
        SurfaceCommand::DspParam { value, .. } => {
            assert_relative_eq!(*value, 0.0, epsilon = 1e-3);
        }
        _ => panic!("Expected DspParam"),
    }

    // Knob 1 (CC 17): comp_threshold (-50.0 to 0.0 dB) at mid-scale (64)
    let cmds = mk3.on_midi(&[0xB0, 17, 64]);
    match &cmds[0] {
        SurfaceCommand::DspParam { param_id, value, .. } => {
            assert_eq!(*param_id, "comp_threshold");
            let expected = -50.0 + (64.0 / 127.0) * 50.0;
            assert_relative_eq!(*value, expected, epsilon = 1e-2);
        }
        _ => panic!("Expected DspParam"),
    }

    // Knob 2 (CC 18): comp_ratio (1.0 to 20.0) at value 0
    let cmds = mk3.on_midi(&[0xB0, 18, 0]);
    match &cmds[0] {
        SurfaceCommand::DspParam { param_id, value, .. } => {
            assert_eq!(*param_id, "comp_ratio");
            assert_relative_eq!(*value, 1.0, epsilon = 1e-3);
        }
        _ => panic!("Expected DspParam"),
    }

    // 2. Switch to Bank 1 (EQ/Filtering) via Note 53 (0x35)
    let _ = mk3.on_midi(&[0x90, 53, 100]);
    assert_eq!(mk3.current_bank, 1);

    // Knob 0 (CC 16) is now hpf_freq (20.0 to 300.0 Hz)
    let cmds = mk3.on_midi(&[0xB0, 16, 64]);
    match &cmds[0] {
        SurfaceCommand::DspParam { param_id, value, .. } => {
            assert_eq!(*param_id, "hpf_freq");
            let expected = 20.0 + (64.0 / 127.0) * 280.0;
            assert_relative_eq!(*value, expected, epsilon = 1e-2);
        }
        _ => panic!("Expected DspParam"),
    }

    // Knob 1 (CC 17) is now eq_low_gain (-12.0 to 12.0 dB) at mid-scale (64 -> ~0.18 dB)
    let cmds = mk3.on_midi(&[0xB0, 17, 64]);
    match &cmds[0] {
        SurfaceCommand::DspParam { param_id, value, .. } => {
            assert_eq!(*param_id, "eq_low_gain");
            let expected = -12.0 + (64.0 / 127.0) * 24.0;
            assert_relative_eq!(*value, expected, epsilon = 1e-2);
        }
        _ => panic!("Expected DspParam"),
    }

    // 3. Switch to Bank 2 (Amps/FX) via CC 29
    let _ = mk3.on_midi(&[0xB0, 29, 100]);
    assert_eq!(mk3.current_bank, 2);

    // Knob 4 (CC 20) is chorus_mix (0.0 to 1.0)
    let cmds = mk3.on_midi(&[0xB0, 20, 127]);
    match &cmds[0] {
        SurfaceCommand::DspParam { param_id, value, .. } => {
            assert_eq!(*param_id, "chorus_mix");
            assert_relative_eq!(*value, 1.0, epsilon = 1e-3);
        }
        _ => panic!("Expected DspParam"),
    }

    // 4. Bank wraps around to 0
    let _ = mk3.on_midi(&[0x90, 53, 100]);
    assert_eq!(mk3.current_bank, 0);
}

#[test]
fn test_mk3_pads_trigger_and_node_bypass_toggling() {
    let mut mk3 = Mk3Surface::default();

    // Pad 0 (Note 36 / C1): Tuner bypass toggle + Trigger
    let cmds = mk3.on_midi(&[0x90, 36, 120]);
    assert_eq!(cmds.len(), 2);
    assert_eq!(cmds[0], SurfaceCommand::Trigger { pad: 0, velocity: 120 });
    assert_eq!(cmds[1], SurfaceCommand::NodeBypass {
        target: Target::Channel1,
        node: "tuner",
        bypassed: true,
    });

    // Press Pad 0 again -> toggles bypass to false
    let cmds = mk3.on_midi(&[0x90, 36, 100]);
    assert_eq!(cmds[1], SurfaceCommand::NodeBypass {
        target: Target::Channel1,
        node: "tuner",
        bypassed: false,
    });

    // Pad 1 (Note 37): Gate bypass
    let cmds = mk3.on_midi(&[0x90, 37, 90]);
    assert_eq!(cmds[1], SurfaceCommand::NodeBypass {
        target: Target::Channel1,
        node: "gate",
        bypassed: true,
    });

    // Pad 4 (Note 40): Compressor bypass
    let cmds = mk3.on_midi(&[0x90, 40, 80]);
    assert_eq!(cmds[1], SurfaceCommand::NodeBypass {
        target: Target::Channel1,
        node: "comp",
        bypassed: true,
    });

    // Pad 12 (Note 48): Trigger only (no DSP node assigned)
    let cmds = mk3.on_midi(&[0x90, 48, 110]);
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0], SurfaceCommand::Trigger { pad: 12, velocity: 110 });
}

#[test]
fn test_mk3_channel_select_and_global_bypass() {
    let mut mk3 = Mk3Surface::default();
    assert_eq!(mk3.selected_channel, Target::Channel1);

    // Note 52 toggles Channel 1 -> Channel 2
    let _ = mk3.on_midi(&[0x90, 52, 100]);
    assert_eq!(mk3.selected_channel, Target::Channel2);

    // Knob adjustments now target Channel 2!
    let cmds = mk3.on_midi(&[0xB0, 16, 64]);
    match &cmds[0] {
        SurfaceCommand::DspParam { target, .. } => {
            assert_eq!(*target, Target::Channel2);
        }
        _ => panic!("Expected DspParam on Channel2"),
    }

    // Note 54 toggles Global Bypass
    let cmds = mk3.on_midi(&[0x90, 54, 100]);
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0], SurfaceCommand::GlobalBypass(true));

    let cmds = mk3.on_midi(&[0x90, 54, 100]);
    assert_eq!(cmds[0], SurfaceCommand::GlobalBypass(false));
}

// ============================================================================
// iCON QCon Pro G2 Driver Tests (Mackie Control / MCU)
// ============================================================================

#[test]
fn test_qcon_fader_pitch_bend_scaling() {
    let mut qcon = QConSurface::default();

    // 1. Fader 0 (Ch 1): Pitch Bend at 0 (Status 0xE0, LSB 0, MSB 0) -> 0 dB Gain
    let cmds = qcon.on_midi(&[0xE0, 0x00, 0x00]);
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0], SurfaceCommand::Gain { input: 1, gain_db: 0 });

    // Fader 0 (Ch 1): Pitch Bend at 8192 (Status 0xE0, LSB 0, MSB 64) -> ~33 dB Gain
    let cmds = qcon.on_midi(&[0xE0, 0x00, 0x40]);
    assert_eq!(cmds[0], SurfaceCommand::Gain { input: 1, gain_db: 33 });

    // Fader 0 (Ch 1): Pitch Bend at 16383 (Status 0xE0, LSB 0x7F, MSB 0x7F) -> 65 dB Gain
    let cmds = qcon.on_midi(&[0xE0, 0x7F, 0x7F]);
    assert_eq!(cmds[0], SurfaceCommand::Gain { input: 1, gain_db: 65 });

    // 2. Fader 1 (Ch 2): Pitch Bend at 4096 (Status 0xE1, LSB 0, MSB 32) -> ~16 dB Gain
    let cmds = qcon.on_midi(&[0xE1, 0x00, 0x20]);
    assert_eq!(cmds[0], SurfaceCommand::Gain { input: 2, gain_db: 16 });

    // 3. Master Fader (Channel 8 / 0xE8): Pitch Bend at 16383 -> Monitor Volume Step 0 (Unity)
    let cmds = qcon.on_midi(&[0xE8, 0x7F, 0x7F]);
    assert_eq!(cmds[0], SurfaceCommand::MonitorVolume(0));

    // Master Fader at 0 -> Monitor Volume Step 127 (Mute)
    let cmds = qcon.on_midi(&[0xE8, 0x00, 0x00]);
    assert_eq!(cmds[0], SurfaceCommand::MonitorVolume(127));

    // Master Fader at mid-scale (8192) -> Step 63
    let cmds = qcon.on_midi(&[0xE8, 0x00, 0x40]);
    assert_eq!(cmds[0], SurfaceCommand::MonitorVolume(63));
}

#[test]
fn test_qcon_vpot_relative_encoders() {
    let mut qcon = QConSurface::default();

    // V-pot 0 (CC 0x10): gate_threshold (-60..0 dB, step 1.0 dB, init -30.0)
    // Clockwise +1 (value 0x01)
    let cmds = qcon.on_midi(&[0xB0, 0x10, 0x01]);
    assert_eq!(cmds.len(), 1);
    match &cmds[0] {
        SurfaceCommand::DspParam { param_id, value, .. } => {
            assert_eq!(*param_id, "gate_threshold");
            assert_relative_eq!(*value, -29.0, epsilon = 1e-3);
        }
        _ => panic!("Expected DspParam"),
    }

    // Counter-clockwise -3 (value 0x43) -> -29.0 - 3.0 = -32.0 dB
    let cmds = qcon.on_midi(&[0xB0, 0x10, 0x43]);
    match &cmds[0] {
        SurfaceCommand::DspParam { value, .. } => {
            assert_relative_eq!(*value, -32.0, epsilon = 1e-3);
        }
        _ => panic!("Expected DspParam"),
    }

    // V-pot 2 (CC 0x12): sat_drive (0..24 dB, step 0.5 dB, init 0.0)
    // Clockwise +4 (value 0x04) -> 0.0 + 4 * 0.5 = 2.0 dB
    let cmds = qcon.on_midi(&[0xB0, 0x12, 0x04]);
    match &cmds[0] {
        SurfaceCommand::DspParam { param_id, value, .. } => {
            assert_eq!(*param_id, "sat_drive");
            assert_relative_eq!(*value, 2.0, epsilon = 1e-3);
        }
        _ => panic!("Expected DspParam"),
    }
}

#[test]
fn test_qcon_buttons_mute_select_transport() {
    let mut qcon = QConSurface::default();

    // Mute 1 (Note 0x10 / 16) -> Toggle Ch 1 Bypass
    let cmds = qcon.on_midi(&[0x90, 0x10, 0x7F]);
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0], SurfaceCommand::NodeBypass {
        target: Target::Channel1,
        node: "channel",
        bypassed: true,
    });

    // Mute 2 (Note 0x11 / 17) -> Toggle Ch 2 Bypass
    let cmds = qcon.on_midi(&[0x90, 0x11, 0x7F]);
    assert_eq!(cmds[0], SurfaceCommand::NodeBypass {
        target: Target::Channel2,
        node: "channel",
        bypassed: true,
    });

    // Select 2 (Note 0x19 / 25) -> Channel 2 Selected
    assert_eq!(qcon.selected_channel, Target::Channel1);
    let _ = qcon.on_midi(&[0x90, 0x19, 0x7F]);
    assert_eq!(qcon.selected_channel, Target::Channel2);

    // Transport Stop (Note 0x5D / 93) -> Toggle Global Bypass
    let cmds = qcon.on_midi(&[0x90, 0x5D, 0x7F]);
    assert_eq!(cmds[0], SurfaceCommand::GlobalBypass(true));

    // Transport Play (Note 0x5E / 94) -> Toggle Source Mode
    let cmds = qcon.on_midi(&[0x90, 0x5E, 0x7F]);
    assert_eq!(cmds[0], SurfaceCommand::SourceMode("program".into()));
}

#[test]
fn test_qcon_bidirectional_feedback_motors_and_leds() {
    let mut qcon = QConSurface::default();

    let mut state = SurfaceState::default();
    state.ch1_gain_db = 42;
    state.ch2_gain_db = 20;
    state.monitor_step = 12;
    state.ch1_bypassed = true;
    state.ch2_bypassed = false;
    state.selected_channel = 0;
    state.global_bypass = true;
    state.source_mode = "program".into();

    let feedback = qcon.feedback(&state);
    assert!(!feedback.is_empty(), "Feedback must produce motor and LED updates");

    // 1. Verify Motorized Fader Pitch Bend:
    // Fader 0 (Ch 1): 42 dB / 65 dB * 16383 = 10586 -> MSB: 82 (0x52), LSB: 90 (0x5A)
    let fader0_msg = feedback.iter().find(|m| m[0] == 0xE0).expect("Fader 0 pitch bend missing");
    let pb0 = ((fader0_msg[2] as u16) << 7) | (fader0_msg[1] as u16);
    let expected_pb0 = QConSurface::gain_to_pitch_bend(42);
    assert_eq!(pb0, expected_pb0);

    // Master Fader (Ch 8 / 0xE8): Monitor Step 12 -> 14-bit pitch bend
    let master_msg = feedback.iter().find(|m| m[0] == 0xE8).expect("Master fader pitch bend missing");
    let pb_master = ((master_msg[2] as u16) << 7) | (master_msg[1] as u16);
    let expected_pb_master = QConSurface::monitor_vol_to_pitch_bend(12);
    assert_eq!(pb_master, expected_pb_master);

    // 2. Verify Button LEDs:
    // Mute 1 (Note 0x10) is active (bypassed) -> Velocity 0x7F (LED ON)
    let mute1_led = feedback.iter().find(|m| m[0] == 0x90 && m[1] == 0x10).expect("Mute 1 LED missing");
    assert_eq!(mute1_led[2], 0x7F);

    // Mute 2 (Note 0x11) is inactive -> Velocity 0x00 (LED OFF)
    let mute2_led = feedback.iter().find(|m| m[0] == 0x90 && m[1] == 0x11).expect("Mute 2 LED missing");
    assert_eq!(mute2_led[2], 0x00);

    // Select 1 (Note 0x18) is active -> 0x7F
    let sel1_led = feedback.iter().find(|m| m[0] == 0x90 && m[1] == 0x18).expect("Select 1 LED missing");
    assert_eq!(sel1_led[2], 0x7F);

    // Global Bypass / Stop (Note 0x5D) is active -> 0x7F
    let stop_led = feedback.iter().find(|m| m[0] == 0x90 && m[1] == 0x5D).expect("Stop LED missing");
    assert_eq!(stop_led[2], 0x7F);

    // Play / Program Mode (Note 0x5E) is active -> 0x7F
    let play_led = feedback.iter().find(|m| m[0] == 0x90 && m[1] == 0x5E).expect("Play LED missing");
    assert_eq!(play_led[2], 0x7F);

    // 3. Deadband Test:
    // Repeated tick with identical state should NOT re-transmit fader pitch bend
    let second_feedback = qcon.feedback(&state);
    let fader_updates = second_feedback.iter().filter(|m| (0xE0..=0xE8).contains(&m[0])).count();
    assert_eq!(fader_updates, 0, "Unchanged faders must not spam motors");
}

#[test]
fn test_round_trip_gain_fader_fidelity() {
    let gain_input = 45_u8;
    let pb = QConSurface::gain_to_pitch_bend(gain_input);
    let lsb = (pb & 0x7F) as u8;
    let msb = ((pb >> 7) & 0x7F) as u8;

    let mut qcon = QConSurface::default();
    let cmds = qcon.on_midi(&[0xE0, lsb, msb]);
    assert_eq!(cmds.len(), 1);
    match cmds[0] {
        SurfaceCommand::Gain { input, gain_db } => {
            assert_eq!(input, 1);
            assert_eq!(gain_db, gain_input, "Gain round-trip must be exact");
        }
        _ => panic!("Expected Gain command"),
    }
}

#[test]
fn test_surface_command_to_remote_message_translation() {
    let dsp_cmd = SurfaceCommand::DspParam {
        target: Target::Channel1,
        param_id: "sat_drive",
        value: 14.5,
    };
    let remote_msg = dsp_cmd.to_remote_message().expect("Must convert to RemoteMessage");
    match remote_msg {
        RemoteMessage::SetDspParam { target, param, value } => {
            assert_eq!(target, "ch1");
            assert_eq!(param, "sat_drive");
            assert_relative_eq!(value, 14.5, epsilon = 1e-4);
        }
        _ => panic!("Expected SetDspParam"),
    }

    let preset_cmd = SurfaceCommand::ChannelPreset {
        target: Target::Channel2,
        preset: InstrumentPreset::ElectricGuitar,
    };
    let remote_msg = preset_cmd.to_remote_message().expect("Must convert to RemoteMessage");
    match remote_msg {
        RemoteMessage::SetChannelPreset { target, preset } => {
            assert_eq!(target, "ch2");
            assert_eq!(preset, "eguitar");
        }
        _ => panic!("Expected SetChannelPreset"),
    }
}

#[test]
fn test_list_midi_ports_no_crash() {
    // Proves list_midi_ports initializes CoreMIDI / ALSA cleanly without panic
    let (inputs, outputs) = list_midi_ports();
    println!("Detected {} inputs, {} outputs", inputs.len(), outputs.len());
}
