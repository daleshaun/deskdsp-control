//! Hardware Control Surface Integration for DeskDSP Control.
//!
//! Provides protocol drivers for:
//! - Native Instruments Maschine MK3 (MIDI Mode, input-only: pads, knobs, banks)
//! - iCON QCon Pro G2 (Mackie Control Universal / MCU, bidirectional motorized faders & LEDs)
//!
//! Control surfaces feed the same command channel as the wireless tablet remote
//! (`RemoteMessage` / `AudioCommand`) via dedicated OS threads. Real-time audio
//! callbacks are never touched.

#![allow(dead_code)]

pub mod mk3;
pub mod qcon;

pub use mk3::Mk3Surface;
pub use qcon::QConSurface;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::dsp::InstrumentPreset;
use crate::remote::RemoteMessage;

/// Target channel strip or master chain for control surface commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Channel1,
    Channel2,
    Master,
}

impl Target {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Channel1 => "ch1",
            Self::Channel2 => "ch2",
            Self::Master => "master",
        }
    }
}

/// Commands emitted by control surfaces.
/// Maps 1:1 onto the existing `RemoteMessage` set.
#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceCommand {
    DspParam {
        target: Target,
        param_id: &'static str,
        value: f32,
    },
    ChannelPreset {
        target: Target,
        preset: InstrumentPreset,
    },
    NodeBypass {
        target: Target,
        node: &'static str,
        bypassed: bool,
    },
    Gain {
        input: u8,
        gain_db: u8,
    },
    MonitorVolume(u8),
    Hp1Volume(u8),
    Hp2Volume(u8),
    GlobalBypass(bool),
    SourceMode(String),
    Trigger {
        pad: u8,
        velocity: u8,
    },
}

impl SurfaceCommand {
    /// Converts a `SurfaceCommand` into the equivalent `RemoteMessage` accepted
    /// by the DeskDSP forwarder and tablet state machine.
    pub fn to_remote_message(self) -> Option<RemoteMessage> {
        match self {
            Self::DspParam { target, param_id, value } => Some(RemoteMessage::SetDspParam {
                target: target.as_str().into(),
                param: param_id.into(),
                value,
            }),
            Self::ChannelPreset { target, preset } => Some(RemoteMessage::SetChannelPreset {
                target: target.as_str().into(),
                preset: preset.as_str().into(),
            }),
            Self::NodeBypass { target, node, bypassed } => Some(RemoteMessage::SetNodeBypass {
                target: target.as_str().into(),
                node: node.into(),
                bypassed,
            }),
            Self::Gain { input, gain_db } => Some(RemoteMessage::SetGain { input, gain_db }),
            Self::MonitorVolume(step) => Some(RemoteMessage::SetMonitorVolume { step }),
            Self::Hp1Volume(step) => Some(RemoteMessage::SetHp1Volume { step }),
            Self::Hp2Volume(step) => Some(RemoteMessage::SetHp2Volume { step }),
            Self::GlobalBypass(enabled) => Some(RemoteMessage::SetGlobalBypass { enabled }),
            Self::SourceMode(mode) => Some(RemoteMessage::SetSourceMode { mode }),
            Self::Trigger { .. } => None,
        }
    }
}

/// A snapshot of DeskDSP state used for bidirectional surface feedback (motors & LEDs).
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceState {
    pub ch1_gain_db: u8,
    pub ch2_gain_db: u8,
    pub monitor_step: u8,
    pub hp1_step: u8,
    pub hp2_step: u8,
    pub global_bypass: bool,
    pub source_mode: String,
    pub ch1_preset: InstrumentPreset,
    pub ch2_preset: InstrumentPreset,
    pub selected_channel: u8, // 0 = Ch1, 1 = Ch2
    pub ch1_bypassed: bool,
    pub ch2_bypassed: bool,
    pub in_l_dbfs: f32,
    pub in_r_dbfs: f32,
    pub out_l_dbfs: f32,
    pub out_r_dbfs: f32,
    pub comp_gr_db: f32,
    pub limiter_gr_db: f32,
}

impl Default for SurfaceState {
    fn default() -> Self {
        Self {
            ch1_gain_db: 30,
            ch2_gain_db: 30,
            monitor_step: 0,
            hp1_step: 0,
            hp2_step: 0,
            global_bypass: false,
            source_mode: "vocal".into(),
            ch1_preset: InstrumentPreset::Vocal,
            ch2_preset: InstrumentPreset::Vocal,
            selected_channel: 0,
            ch1_bypassed: false,
            ch2_bypassed: false,
            in_l_dbfs: -80.0,
            in_r_dbfs: -80.0,
            out_l_dbfs: -80.0,
            out_r_dbfs: -80.0,
            comp_gr_db: 0.0,
            limiter_gr_db: 0.0,
        }
    }
}

/// A hardware control surface translates device MIDI <-> DeskDSP commands + state.
/// Pure, hardware-free trait: unit-testable against synthetic MIDI byte slices.
pub trait ControlSurface: Send {
    /// Name of the control surface driver (e.g. "Maschine MK3", "iCON QCon Pro G2").
    fn name(&self) -> &'static str;

    /// Parse one inbound MIDI message into zero or more DeskDSP commands.
    /// Pure function with internal state tracking — 100% testable without hardware.
    fn on_midi(&mut self, msg: &[u8]) -> Vec<SurfaceCommand>;

    /// Produce outbound MIDI messages to mirror current DSP state (faders, LEDs, rings).
    /// Called periodically (e.g. 20 Hz tick). Returns empty vector for input-only surfaces.
    fn feedback(&mut self, state: &SurfaceState) -> Vec<Vec<u8>>;
}

/// Enumerates supported hardware control surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum SurfaceType {
    Mk3,
    Qcon,
    Both,
    None,
}

/// Lists all available MIDI input and output ports on the system.
pub fn list_midi_ports() -> (Vec<String>, Vec<String>) {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();

    if let Ok(midi_in) = MidiInput::new("deskdsp-probe-in") {
        for port in midi_in.ports() {
            if let Ok(name) = midi_in.port_name(&port) {
                inputs.push(name);
            }
        }
    }

    if let Ok(midi_out) = MidiOutput::new("deskdsp-probe-out") {
        for port in midi_out.ports() {
            if let Ok(name) = midi_out.port_name(&port) {
                outputs.push(name);
            }
        }
    }

    (inputs, outputs)
}

/// Spawns a dedicated MIDI control surface runner thread.
///
/// Dispatches inbound MIDI messages to the surface driver, forwards generated
/// `SurfaceCommand`s to the remote command channel, and periodically triggers
/// outbound feedback messages for motor/LED synchronization.
pub struct SurfaceRunner {
    _in_conn: Option<MidiInputConnection<()>>,
    _out_conn: Option<MidiOutputConnection>,
    stop_flag: Arc<AtomicBool>,
}

impl SurfaceRunner {
    /// Starts a control surface runner with given driver, port patterns, and command sink.
    pub fn start<S: ControlSurface + 'static>(
        surface: S,
        in_pattern: Option<String>,
        out_pattern: Option<String>,
        cmd_tx: std::sync::mpsc::Sender<RemoteMessage>,
        state_query: Arc<dyn Fn() -> SurfaceState + Send + Sync + 'static>,
    ) -> Result<Self, anyhow::Error> {
        let stop_flag = Arc::new(AtomicBool::new(false));

        // Connect MIDI Input
        let in_conn = if let Some(pat) = in_pattern {
            let midi_in = MidiInput::new("deskdsp-surface-in")?;
            let ports = midi_in.ports();
            let matched_port = ports.into_iter().find(|p| {
                if let Ok(name) = midi_in.port_name(p) {
                    name.to_lowercase().contains(&pat.to_lowercase())
                } else {
                    false
                }
            });

            if let Some(port) = matched_port {
                let port_name = midi_in.port_name(&port).unwrap_or_else(|_| "Unknown".into());
                println!("🎹 Control Surface Connected (MIDI In): \"{}\"", port_name);

                let cmd_tx_clone = cmd_tx.clone();
                let surface_in = Arc::new(std::sync::Mutex::new(surface));
                let surface_in_cb = Arc::clone(&surface_in);

                let conn = midi_in.connect(
                    &port,
                    "deskdsp-in",
                    move |_timestamp, message, _| {
                        if let Ok(mut surf) = surface_in_cb.lock() {
                            let cmds = surf.on_midi(message);
                            for c in cmds {
                                if let Some(rm) = c.to_remote_message() {
                                    let _ = cmd_tx_clone.send(rm);
                                }
                            }
                        }
                    },
                    (),
                ).map_err(|e| anyhow::anyhow!("Failed to connect MIDI input: {}", e))?;

                // Note: surface was moved into Arc Mutex
                // Connect MIDI Output if requested
                let out_conn = if let Some(out_pat) = out_pattern {
                    let midi_out = MidiOutput::new("deskdsp-surface-out")?;
                    let out_ports = midi_out.ports();
                    let matched_out = out_ports.into_iter().find(|p| {
                        if let Ok(name) = midi_out.port_name(p) {
                            name.to_lowercase().contains(&out_pat.to_lowercase())
                        } else {
                            false
                        }
                    });

                    if let Some(out_port) = matched_out {
                        let out_name = midi_out.port_name(&out_port).unwrap_or_else(|_| "Unknown".into());
                        println!("🎛️ Control Surface Connected (MIDI Out): \"{}\"", out_name);
                        let mut out_conn = midi_out.connect(&out_port, "deskdsp-out")
                            .map_err(|e| anyhow::anyhow!("Failed to connect MIDI output: {}", e))?;

                        // Spawn 20Hz Feedback Worker Thread
                        let stop_worker = Arc::clone(&stop_flag);
                        let surface_feedback = Arc::clone(&surface_in);
                        std::thread::Builder::new()
                            .name("midi-surface-feedback".into())
                            .spawn(move || {
                                while !stop_worker.load(Ordering::Relaxed) {
                                    std::thread::sleep(Duration::from_millis(50)); // 20 Hz
                                    let state = state_query();
                                    if let Ok(mut surf) = surface_feedback.lock() {
                                        let msgs = surf.feedback(&state);
                                        for m in msgs {
                                            let _ = out_conn.send(&m);
                                        }
                                    }
                                }
                            })
                            .expect("Failed to spawn midi-surface-feedback thread");

                        None
                    } else {
                        println!("⚠️ MIDI output matching \"{}\" not found (feedback disabled)", out_pat);
                        None
                    }
                } else {
                    None
                };

                return Ok(Self {
                    _in_conn: Some(conn),
                    _out_conn: out_conn,
                    stop_flag,
                });
            } else {
                println!("⚠️ MIDI input matching \"{}\" not found (control surface inactive)", pat);
                return Ok(Self {
                    _in_conn: None,
                    _out_conn: None,
                    stop_flag,
                });
            }
        } else {
            None
        };

        Ok(Self {
            _in_conn: in_conn,
            _out_conn: None,
            stop_flag,
        })
    }
}

impl Drop for SurfaceRunner {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
    }
}
