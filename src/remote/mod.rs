//! Wireless Touch Tablet Remote Server for DeskDSP Control & Antelope Zen Go.
//!
//! Architectural Guarantees:
//! 1. Zero locks on the real-time audio thread: server only interacts with
//!    `HardwareController` and lock-free atomic `AudioMeters`.
//! 2. Dedicated OS thread for HID writes: all blocking USB HID writes and mutex locks
//!    are moved completely off the Tokio async executor, eliminating fader latency.
//! 3. Shortened device-lock holds: HID reader holds lock for at most 5ms and yields.
//! 4. Hardware -> Tablet state sync: real-time 0x73 telemetry snapshots from Zen Go
//!    are pushed over WebSocket to all connected tablets.
//! 5. Token authentication gate: optional `--remote-token` protecting WebSocket and REST APIs.
//! 6. Self-contained: embeds responsive tablet touch HTML5/CSS3 application.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json, Response},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::audio::{AudioCommand, AudioMeters, CommandTarget};
use crate::dsp::presets::{self, InstrumentPreset};
use crate::dsp::tuner::Scale;
use crate::hardware::HardwareController;
use antelope_protocol::PreampMode;

mod assets;
use assets::{ICON_192_PNG, ICON_512_PNG, MANIFEST_JSON, SW_JS, TABLET_TOUCH_HTML};

fn default_preset_ch1() -> String {
    "vocal".to_string()
}
fn default_preset_ch2() -> String {
    "vocal".to_string()
}
fn default_voicing() -> String {
    "flat".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RemoteMessage {
    #[serde(rename = "state_sync")]
    StateSync {
        input1: PreampChannelState,
        input2: PreampChannelState,
        outputs: OutputLevelsState,
        meters: LiveMetersState,
        #[serde(default)]
        global_bypass: bool,
        #[serde(default)]
        source_mode: String,
        #[serde(default = "default_preset_ch1")]
        ch1_preset: String,
        #[serde(default = "default_preset_ch2")]
        ch2_preset: String,
        #[serde(default)]
        ch1_suggested_mode: String,
        #[serde(default)]
        ch1_suggested_phantom: bool,
        #[serde(default)]
        ch2_suggested_mode: String,
        #[serde(default)]
        ch2_suggested_phantom: bool,
        #[serde(default = "default_voicing")]
        ch1_voicing: String,
        #[serde(default = "default_voicing")]
        ch2_voicing: String,
        #[serde(default)]
        ch1_mic_ir_loaded: bool,
        #[serde(default)]
        ch2_mic_ir_loaded: bool,
    },
    #[serde(rename = "set_gain")]
    SetGain { input: u8, gain_db: u8 },
    #[serde(rename = "set_phantom")]
    SetPhantom { input: u8, enabled: bool },
    #[serde(rename = "set_phase")]
    SetPhase { input: u8, enabled: bool },
    #[serde(rename = "set_mode")]
    SetMode { input: u8, mode: String },
    #[serde(rename = "set_monitor_volume")]
    SetMonitorVolume { step: u8 },
    #[serde(rename = "set_monitor_mute")]
    SetMonitorMute { enabled: bool },
    #[serde(rename = "set_hp1_volume")]
    SetHp1Volume { step: u8 },
    #[serde(rename = "set_hp2_volume")]
    SetHp2Volume { step: u8 },
    #[serde(rename = "set_global_bypass")]
    SetGlobalBypass { enabled: bool },
    #[serde(rename = "set_node_bypass")]
    SetNodeBypass {
        target: String,
        node: String,
        bypassed: bool,
    },
    #[serde(rename = "set_source_mode")]
    SetSourceMode {
        mode: String,
    },
    #[serde(rename = "set_dsp_param")]
    SetDspParam {
        target: String,
        param: String,
        value: f32,
    },
    #[serde(rename = "set_tuner_scale")]
    SetTunerScale {
        target: String,
        scale: String,
    },
    #[serde(rename = "set_channel_preset")]
    SetChannelPreset {
        target: String,
        preset: String,
    },
    #[serde(rename = "set_mic_voicing")]
    SetMicVoicing {
        target: String,
        voicing: String,
    },
    #[serde(rename = "set_mic_image_ir")]
    SetMicImageIr {
        target: String,
        ir_name: String,
        #[serde(default)]
        samples: Option<Vec<f32>>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreampChannelState {
    pub gain_db: u8,
    pub mode: String,
    pub phantom: bool,
    pub phase_invert: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputLevelsState {
    pub monitor_step: u8,
    pub monitor_mute: bool,
    pub hp1_step: u8,
    pub hp2_step: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveMetersState {
    pub in_l_dbfs: f32,
    pub in_r_dbfs: f32,
    pub out_l_dbfs: f32,
    pub out_r_dbfs: f32,
    pub comp_gr_db: f32,
    pub lim_gr_db: f32,
    #[serde(default)]
    pub integrated_lufs: f32,
    #[serde(default)]
    pub true_peak_dbtp: f32,
}

/// Commands sent to the dedicated coalescing HID worker OS thread
#[derive(Debug, Clone)]
enum CoalescedHidCommand {
    Gain { input: u8, val: u8 },
    MonitorVol { val: u8 },
    Hp1Vol { val: u8 },
    Hp2Vol { val: u8 },
    Phantom { input: u8, val: bool },
    Phase { input: u8, val: bool },
    Mode { input: u8, mode: PreampMode },
    MonitorMute { val: bool },
}

struct AppState {
    hw: Option<HardwareController>,
    meters: Arc<AudioMeters>,
    global_bypass: Arc<AtomicBool>,
    source_mode: Arc<AtomicBool>,
    ch1_preset: Arc<std::sync::RwLock<String>>,
    ch2_preset: Arc<std::sync::RwLock<String>>,
    ch1_voicing: Arc<std::sync::RwLock<String>>,
    ch2_voicing: Arc<std::sync::RwLock<String>>,
    ch1_mic_ir_loaded: Arc<AtomicBool>,
    ch2_mic_ir_loaded: Arc<AtomicBool>,
    sample_rate: f32,
    cmd_tx: std::sync::mpsc::Sender<CoalescedHidCommand>,
    dsp_cmd_tx: std::sync::mpsc::Sender<AudioCommand>,
    broadcast_tx: broadcast::Sender<String>,
    token: Option<String>,
    broadcast_started: AtomicBool,
}

impl AppState {
    pub fn dispatch_remote_message(&self, cmd: RemoteMessage) {
        match cmd {
            RemoteMessage::SetGain { input, gain_db } => {
                let _ = self.cmd_tx.send(CoalescedHidCommand::Gain { input, val: gain_db });
            }
            RemoteMessage::SetPhantom { input, enabled } => {
                let _ = self.cmd_tx.send(CoalescedHidCommand::Phantom { input, val: enabled });
            }
            RemoteMessage::SetPhase { input, enabled } => {
                let _ = self.cmd_tx.send(CoalescedHidCommand::Phase { input, val: enabled });
            }
            RemoteMessage::SetMode { input, mode } => {
                let m = match mode.as_str() {
                    "Mic" => PreampMode::Mic,
                    "Line" => PreampMode::Line,
                    "HiZ" => PreampMode::HiZ,
                    _ => PreampMode::Mic,
                };
                let _ = self.cmd_tx.send(CoalescedHidCommand::Mode { input, mode: m });
            }
            RemoteMessage::SetMonitorVolume { step } => {
                let _ = self.cmd_tx.send(CoalescedHidCommand::MonitorVol { val: step });
            }
            RemoteMessage::SetMonitorMute { enabled } => {
                let _ = self.cmd_tx.send(CoalescedHidCommand::MonitorMute { val: enabled });
            }
            RemoteMessage::SetHp1Volume { step } => {
                let _ = self.cmd_tx.send(CoalescedHidCommand::Hp1Vol { val: step });
            }
            RemoteMessage::SetHp2Volume { step } => {
                let _ = self.cmd_tx.send(CoalescedHidCommand::Hp2Vol { val: step });
            }
            RemoteMessage::SetGlobalBypass { enabled } => {
                self.global_bypass.store(enabled, Ordering::Relaxed);
            }
            RemoteMessage::SetNodeBypass { target, node, bypassed } => {
                let node_id: &'static str = match node.as_str() {
                    "hpf" => "hpf",
                    "gate" => "gate",
                    "deesser" => "deesser",
                    "eq" => "eq",
                    "comp" => "comp",
                    "tuner" => "tuner",
                    "sat" => "sat",
                    "amp" => "amp",
                    "cab" => "cab",
                    "drive" => "drive",
                    "chorus" => "chorus",
                    "reverb" => "reverb",
                    "mic_image" => "mic_image",
                    "glue" => "glue",
                    "width" => "width",
                    "limiter" => "limiter",
                    _ => return,
                };
                match target.as_str() {
                    "ch1" => {
                        let _ = self.dsp_cmd_tx.send(AudioCommand::SetNodeBypass {
                            target: CommandTarget::Channel1,
                            node_id,
                            bypassed,
                        });
                    }
                    "ch2" => {
                        let _ = self.dsp_cmd_tx.send(AudioCommand::SetNodeBypass {
                            target: CommandTarget::Channel2,
                            node_id,
                            bypassed,
                        });
                    }
                    "both" => {
                        let _ = self.dsp_cmd_tx.send(AudioCommand::SetNodeBypass {
                            target: CommandTarget::Channel1,
                            node_id,
                            bypassed,
                        });
                        let _ = self.dsp_cmd_tx.send(AudioCommand::SetNodeBypass {
                            target: CommandTarget::Channel2,
                            node_id,
                            bypassed,
                        });
                    }
                    "master" => {
                        let _ = self.dsp_cmd_tx.send(AudioCommand::SetNodeBypass {
                            target: CommandTarget::Master,
                            node_id,
                            bypassed,
                        });
                    }
                    _ => {}
                }
            }
            RemoteMessage::SetSourceMode { mode } => {
                let is_prog = mode.to_lowercase() == "program";
                self.source_mode.store(is_prog, Ordering::Relaxed);
                let preset = if is_prog { InstrumentPreset::ProgramThru } else { InstrumentPreset::Vocal };
                if let Ok(mut lock) = self.ch1_preset.write() {
                    *lock = preset.as_str().to_string();
                }
                if let Ok(mut lock) = self.ch2_preset.write() {
                    *lock = preset.as_str().to_string();
                }
                let sample_rate = self.sample_rate;
                let rack1 = presets::build_rack(preset, sample_rate);
                let rack2 = presets::build_rack(preset, sample_rate);
                let _ = self.dsp_cmd_tx.send(AudioCommand::SwapMonoRack {
                    target: CommandTarget::Channel1,
                    rack: Box::new(rack1),
                });
                let _ = self.dsp_cmd_tx.send(AudioCommand::SwapMonoRack {
                    target: CommandTarget::Channel2,
                    rack: Box::new(rack2),
                });
            }
            RemoteMessage::SetChannelPreset { target, preset } => {
                let preset_variant = InstrumentPreset::parse_str(&preset).unwrap_or(InstrumentPreset::Vocal);
                let sample_rate = self.sample_rate;
                let new_rack = presets::build_rack(preset_variant, sample_rate);
                let cmd_target = match target.to_lowercase().as_str() {
                    "ch1" | "1" | "input1" => {
                        if let Ok(mut lock) = self.ch1_preset.write() {
                            *lock = preset_variant.as_str().to_string();
                        }
                        CommandTarget::Channel1
                    }
                    "ch2" | "2" | "input2" => {
                        if let Ok(mut lock) = self.ch2_preset.write() {
                            *lock = preset_variant.as_str().to_string();
                        }
                        CommandTarget::Channel2
                    }
                    _ => return,
                };
                let _ = self.dsp_cmd_tx.send(AudioCommand::SwapMonoRack {
                    target: cmd_target,
                    rack: Box::new(new_rack),
                });
            }
            RemoteMessage::SetDspParam { target, param, value } => {
                let cmd_target = match target.as_str() {
                    "ch1" => CommandTarget::Channel1,
                    "ch2" => CommandTarget::Channel2,
                    "master" => CommandTarget::Master,
                    _ => return,
                };
                let param_id: &'static str = match param.as_str() {
                    "gate_threshold" => "gate_threshold",
                    "comp_threshold" => "comp_threshold",
                    "comp_ratio" => "comp_ratio",
                    "sat_drive" => "sat_drive",
                    "hpf_freq" => "hpf_freq",
                    "eq_low_gain" => "eq_low_gain",
                    "eq_lmid_gain" => "eq_lmid_gain",
                    "eq_hmid_gain" => "eq_hmid_gain",
                    "eq_hi_gain" => "eq_hi_gain",
                    "deess_amount" => "deess_amount",
                    "comp_attack" => "comp_attack",
                    "comp_release" => "comp_release",
                    "tuner_retune" => "tuner_retune",
                    "amp_drive" => "amp_drive",
                    "amp_level" => "amp_level",
                    "cab_type" => "cab_type",
                    "drive_gain" => "drive_gain",
                    "drive_blend" => "drive_blend",
                    "chorus_mix" => "chorus_mix",
                    "reverb_mix" => "reverb_mix",
                    "glue_threshold" => "glue_threshold",
                    "stereo_width" => "stereo_width",
                    "limiter_ceiling" => "limiter_ceiling",
                    "master_eq_low" => "master_eq_low",
                    "master_eq_mid" => "master_eq_mid",
                    "master_eq_high" => "master_eq_high",
                    "mic_dry_wet" => "mic_dry_wet",
                    _ => return,
                };
                let _ = self.dsp_cmd_tx.send(AudioCommand::SetParam {
                    target: cmd_target,
                    param_id,
                    value,
                });
            }
            RemoteMessage::SetTunerScale { target, scale } => {
                let cmd_target = match target.as_str() {
                    "ch1" => CommandTarget::Channel1,
                    "ch2" => CommandTarget::Channel2,
                    _ => return,
                };
                let scale_variant = match scale.as_str() {
                    "Chromatic" => Scale::Chromatic,
                    "Major" => Scale::Major,
                    "NaturalMinor" => Scale::NaturalMinor,
                    "HarmonicMinor" => Scale::HarmonicMinor,
                    "MajorPentatonic" => Scale::MajorPentatonic,
                    "MinorPentatonic" => Scale::MinorPentatonic,
                    _ => return,
                };
                let _ = self.dsp_cmd_tx.send(AudioCommand::SetTunerScale {
                    target: cmd_target,
                    scale: scale_variant,
                });
            }
            RemoteMessage::SetMicVoicing { target, voicing } => {
                let parsed = crate::dsp::MicVoicing::parse_str(&voicing).unwrap_or(crate::dsp::MicVoicing::Flat);
                let cmd_target = match target.to_lowercase().as_str() {
                    "ch1" | "1" => {
                        if let Ok(mut lock) = self.ch1_voicing.write() {
                            *lock = parsed.as_str().to_string();
                        }
                        CommandTarget::Channel1
                    }
                    "ch2" | "2" => {
                        if let Ok(mut lock) = self.ch2_voicing.write() {
                            *lock = parsed.as_str().to_string();
                        }
                        CommandTarget::Channel2
                    }
                    _ => return,
                };
                let _ = self.dsp_cmd_tx.send(AudioCommand::ApplyMicVoicing {
                    target: cmd_target,
                    voicing: parsed,
                });
            }
            RemoteMessage::SetMicImageIr { target, ir_name, samples } => {
                let (cmd_target, flag_ref) = match target.to_lowercase().as_str() {
                    "ch1" | "1" => (CommandTarget::Channel1, &self.ch1_mic_ir_loaded),
                    "ch2" | "2" => (CommandTarget::Channel2, &self.ch2_mic_ir_loaded),
                    _ => return,
                };
                let sample_rate = self.sample_rate;
                if let Some(s) = samples {
                    if !s.is_empty() {
                        let rack = crate::dsp::build_vocal_rack_with_mic_image(&s, &ir_name, sample_rate);
                        flag_ref.store(true, Ordering::Relaxed);
                        let _ = self.dsp_cmd_tx.send(AudioCommand::SwapMonoRack {
                            target: cmd_target,
                            rack: Box::new(rack),
                        });
                        return;
                    }
                }
                let rack = presets::build_rack(InstrumentPreset::Vocal, sample_rate);
                flag_ref.store(false, Ordering::Relaxed);
                let _ = self.dsp_cmd_tx.send(AudioCommand::SwapMonoRack {
                    target: cmd_target,
                    rack: Box::new(rack),
                });
            }
            _ => {}
        }
    }
}

#[derive(Clone)]
pub struct TabletRemoteServer {
    port: u16,
    state: Arc<AppState>,
    pub(crate) remote_msg_tx: std::sync::mpsc::Sender<RemoteMessage>,
}

impl TabletRemoteServer {
    #[allow(dead_code)]
    pub fn new(
        hw: Option<HardwareController>,
        meters: Arc<AudioMeters>,
        global_bypass: Arc<AtomicBool>,
        source_mode: Arc<AtomicBool>,
        engine_producers: Option<(rtrb::Producer<AudioCommand>, rtrb::Producer<AudioCommand>)>,
        port: u16,
        token: Option<String>,
    ) -> Self {
        Self::with_sample_rate(
            hw,
            meters,
            global_bypass,
            source_mode,
            engine_producers,
            port,
            token,
            48000.0,
        )
    }

    pub fn with_sample_rate(
        hw: Option<HardwareController>,
        meters: Arc<AudioMeters>,
        global_bypass: Arc<AtomicBool>,
        source_mode: Arc<AtomicBool>,
        engine_producers: Option<(rtrb::Producer<AudioCommand>, rtrb::Producer<AudioCommand>)>,
        port: u16,
        token: Option<String>,
        sample_rate: f32,
    ) -> Self {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<CoalescedHidCommand>();
        let (dsp_cmd_tx, dsp_cmd_rx) = std::sync::mpsc::channel::<AudioCommand>();
        let (broadcast_tx, _) = broadcast::channel::<String>(64);

        // 1. Dedicated Coalescing HID Worker (Dedicated OS Thread)
        // Moves all blocking USB HID writes and mutex locks completely off the Tokio
        // async executor. High-frequency touch scrubbing (gain/volume) is coalesced at 50Hz,
        // while discrete buttons (+48V, phase, mode, mute) execute immediately.
        let hw_clone = hw.clone();
        std::thread::Builder::new()
            .name("tablet-hid-writer".into())
            .spawn(move || {
                let mut pending_gain: [Option<u8>; 2] = [None, None];
                let mut pending_mon_vol: Option<u8> = None;
                let mut pending_hp1_vol: Option<u8> = None;
                let mut pending_hp2_vol: Option<u8> = None;

                let flush_interval = Duration::from_millis(20); // 50Hz write rate for rapid touch response
                let mut last_flush = std::time::Instant::now();

                loop {
                    let timeout = flush_interval.saturating_sub(last_flush.elapsed());
                    match cmd_rx.recv_timeout(timeout) {
                        Ok(cmd) => {
                            handle_hid_cmd(&hw_clone, cmd, &mut pending_gain, &mut pending_mon_vol, &mut pending_hp1_vol, &mut pending_hp2_vol);
                            while let Ok(next) = cmd_rx.try_recv() {
                                handle_hid_cmd(&hw_clone, next, &mut pending_gain, &mut pending_mon_vol, &mut pending_hp1_vol, &mut pending_hp2_vol);
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }

                    if last_flush.elapsed() >= flush_interval {
                        if let Some(h) = &hw_clone {
                            for ch in 0..2 {
                                if let Some(g) = pending_gain[ch].take() {
                                    let _ = h.set_preamp_gain(ch as u8, g);
                                }
                            }
                            if let Some(mv) = pending_mon_vol.take() {
                                let _ = h.set_monitor_volume(mv);
                            }
                            if let Some(hp1) = pending_hp1_vol.take() {
                                let _ = h.set_hp1_volume(hp1);
                            }
                            if let Some(hp2) = pending_hp2_vol.take() {
                                let _ = h.set_hp2_volume(hp2);
                            }
                        }
                        last_flush = std::time::Instant::now();
                    }
                }
            })
            .expect("Failed to spawn tablet-hid-writer thread");

        // 2. Dedicated Engine Command Forwarder (Dedicated OS Thread)
        // Preserves the SPSC single-producer invariant by owning the rtrb command producers.
        // Drains commands sent from WebSocket handlers and pushes them non-blockingly to the audio engine.
        if let Some((mut in_prod, mut out_prod)) = engine_producers {
            std::thread::Builder::new()
                .name("tablet-dsp-forwarder".into())
                .spawn(move || {
                    while let Ok(cmd) = dsp_cmd_rx.recv() {
                        match cmd {
                            AudioCommand::SetInputGain { target: CommandTarget::Master, .. }
                            | AudioCommand::SetOutputGain { target: CommandTarget::Master, .. }
                            | AudioCommand::SetBypass { target: CommandTarget::Master, .. }
                            | AudioCommand::SetNodeBypass { target: CommandTarget::Master, .. }
                            | AudioCommand::ToggleBypass { target: CommandTarget::Master, .. }
                            | AudioCommand::MoveNode { target: CommandTarget::Master, .. }
                            | AudioCommand::SwapNodes { target: CommandTarget::Master, .. }
                            | AudioCommand::InsertStereoNode { target: CommandTarget::Master, .. }
                            | AudioCommand::RemoveNode { target: CommandTarget::Master, .. }
                            | AudioCommand::SwapStereoRack { target: CommandTarget::Master, .. }
                            | AudioCommand::SetParam { target: CommandTarget::Master, .. }
                            | AudioCommand::ResetAll { target: CommandTarget::Master, .. } => {
                                let _ = out_prod.push(cmd);
                            }
                            _ => {
                                let _ = in_prod.push(cmd);
                            }
                        }
                    }
                })
                .expect("Failed to spawn tablet-dsp-forwarder thread");
        }

        let state = Arc::new(AppState {
            hw,
            meters,
            global_bypass,
            source_mode,
            ch1_preset: Arc::new(std::sync::RwLock::new("vocal".to_string())),
            ch2_preset: Arc::new(std::sync::RwLock::new("vocal".to_string())),
            ch1_voicing: Arc::new(std::sync::RwLock::new("flat".to_string())),
            ch2_voicing: Arc::new(std::sync::RwLock::new("flat".to_string())),
            ch1_mic_ir_loaded: Arc::new(AtomicBool::new(false)),
            ch2_mic_ir_loaded: Arc::new(AtomicBool::new(false)),
            sample_rate,
            cmd_tx,
            dsp_cmd_tx,
            broadcast_tx,
            token,
            broadcast_started: AtomicBool::new(false),
        });

        // 2. Dedicated RemoteMessage Dispatcher (Dedicated OS Thread)
        let (remote_msg_tx, remote_msg_rx) = std::sync::mpsc::channel::<RemoteMessage>();
        let state_dispatch = Arc::clone(&state);
        std::thread::Builder::new()
            .name("remote-msg-dispatcher".into())
            .spawn(move || {
                while let Ok(msg) = remote_msg_rx.recv() {
                    state_dispatch.dispatch_remote_message(msg);
                }
            })
            .expect("Failed to spawn remote-msg-dispatcher thread");

        // 3. Real-Time Hardware -> Tablet State Sync Broadcast Worker
        // If constructed inside a Tokio runtime, spawn the broadcast worker immediately.
        if tokio::runtime::Handle::try_current().is_ok() {
            Self::spawn_broadcast_worker(&state);
        }

        Self { port, state, remote_msg_tx }
    }

    /// Expose command sender so AudioEngine / Desktop TUI can route through the lock-free forwarder.
    pub fn dsp_command_sender(&self) -> std::sync::mpsc::Sender<AudioCommand> {
        self.state.dsp_cmd_tx.clone()
    }

    /// Expose remote message sender so Control Surfaces can dispatch directly.
    pub fn message_sender(&self) -> std::sync::mpsc::Sender<RemoteMessage> {
        self.remote_msg_tx.clone()
    }


    /// Generates a snapshot of SurfaceState for hardware feedback.
    pub fn surface_state_snapshot(&self) -> crate::control::SurfaceState {
        let snap = self.state.hw.as_ref().and_then(|h| h.get_snapshot());
        let (ch1_gain, ch2_gain, mon_vol, hp1, hp2) = if let Some(s) = snap {
            (s.preamp.input1.gain_raw, s.preamp.input2.gain_raw, s.outputs[0].volume, s.outputs[1].volume, s.outputs[2].volume)
        } else {
            (30, 30, 0, 0, 0)
        };
        let ch1_str = self.state.ch1_preset.read().map(|g| g.clone()).unwrap_or_else(|_| "vocal".into());
        let ch2_str = self.state.ch2_preset.read().map(|g| g.clone()).unwrap_or_else(|_| "vocal".into());
        let p1 = InstrumentPreset::parse_str(&ch1_str).unwrap_or(InstrumentPreset::Vocal);
        let p2 = InstrumentPreset::parse_str(&ch2_str).unwrap_or(InstrumentPreset::Vocal);

        let in_l = AudioMeters::load_f32(&self.state.meters.in_l_peak);
        let in_r = AudioMeters::load_f32(&self.state.meters.in_r_peak);
        let out_l = AudioMeters::load_f32(&self.state.meters.out_l_peak);
        let out_r = AudioMeters::load_f32(&self.state.meters.out_r_peak);
        let in_l_dbfs = if in_l > 1e-4 { 20.0 * in_l.log10() } else { -80.0 };
        let in_r_dbfs = if in_r > 1e-4 { 20.0 * in_r.log10() } else { -80.0 };
        let out_l_dbfs = if out_l > 1e-4 { 20.0 * out_l.log10() } else { -80.0 };
        let out_r_dbfs = if out_r > 1e-4 { 20.0 * out_r.log10() } else { -80.0 };
        let comp_gr_db = AudioMeters::load_f32(&self.state.meters.comp_gr_db);
        let limiter_gr_db = AudioMeters::load_f32(&self.state.meters.master_limiter_gr_db);

        crate::control::SurfaceState {
            ch1_gain_db: ch1_gain,
            ch2_gain_db: ch2_gain,
            monitor_step: mon_vol,
            hp1_step: hp1,
            hp2_step: hp2,
            global_bypass: self.state.global_bypass.load(Ordering::Relaxed),
            source_mode: if self.state.source_mode.load(Ordering::Relaxed) { "program".into() } else { "vocal".into() },
            ch1_preset: p1,
            ch2_preset: p2,
            selected_channel: 0,
            ch1_bypassed: false,
            ch2_bypassed: false,
            in_l_dbfs,
            in_r_dbfs,
            out_l_dbfs,
            out_r_dbfs,
            comp_gr_db,
            limiter_gr_db,
        }
    }

    fn spawn_broadcast_worker(state: &Arc<AppState>) {
        if state.broadcast_started.swap(true, Ordering::SeqCst) {
            return; // Already started
        }

        let hw_sync = state.hw.clone();
        let meters_sync = Arc::clone(&state.meters);
        let bcast_tx = state.broadcast_tx.clone();
        let bypass_sync = Arc::clone(&state.global_bypass);
        let source_mode_sync = Arc::clone(&state.source_mode);
        let ch1_preset_sync = Arc::clone(&state.ch1_preset);
        let ch2_preset_sync = Arc::clone(&state.ch2_preset);
        let ch1_voicing_sync = Arc::clone(&state.ch1_voicing);
        let ch2_voicing_sync = Arc::clone(&state.ch2_voicing);
        let ch1_ir_sync = Arc::clone(&state.ch1_mic_ir_loaded);
        let ch2_ir_sync = Arc::clone(&state.ch2_mic_ir_loaded);

        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(50)); // 20Hz sync
            loop {
                ticker.tick().await;

                let (input1, input2, outputs) = if let Some(h) = &hw_sync {
                    if let Some(snap) = h.get_snapshot() {
                        let pre = snap.preamp;
                        (
                            PreampChannelState {
                                gain_db: pre.input1.gain_raw,
                                mode: format!("{:?}", pre.input1.mode),
                                phantom: pre.input1.phantom_on,
                                phase_invert: (pre.input1.mode_raw & 0x40) != 0,
                            },
                            PreampChannelState {
                                gain_db: pre.input2.gain_raw,
                                mode: format!("{:?}", pre.input2.mode),
                                phantom: pre.input2.phantom_on,
                                phase_invert: (pre.input2.mode_raw & 0x40) != 0,
                            },
                            OutputLevelsState {
                                monitor_step: snap.outputs[0].volume,
                                monitor_mute: snap.outputs[0].mode == antelope_protocol::OutputMode::Mute,
                                hp1_step: snap.outputs[1].volume,
                                hp2_step: snap.outputs[2].volume,
                            },
                        )
                    } else {
                        default_fallback_state()
                    }
                } else {
                    default_fallback_state()
                };

                let in_l = AudioMeters::load_f32(&meters_sync.in_l_peak);
                let in_r = AudioMeters::load_f32(&meters_sync.in_r_peak);
                let out_l = AudioMeters::load_f32(&meters_sync.out_l_peak);
                let out_r = AudioMeters::load_f32(&meters_sync.out_r_peak);

                let in_l_dbfs = if in_l > 1e-4 { 20.0 * in_l.log10() } else { -80.0 };
                let in_r_dbfs = if in_r > 1e-4 { 20.0 * in_r.log10() } else { -80.0 };
                let out_l_dbfs = if out_l > 1e-4 { 20.0 * out_l.log10() } else { -80.0 };
                let out_r_dbfs = if out_r > 1e-4 { 20.0 * out_r.log10() } else { -80.0 };
                let comp_gr_db = AudioMeters::load_f32(&meters_sync.comp_gr_db);
                let lim_gr_db = AudioMeters::load_f32(&meters_sync.master_limiter_gr_db);
                let integrated_lufs = AudioMeters::load_f32(&meters_sync.integrated_lufs);
                let true_peak_dbtp = AudioMeters::load_f32(&meters_sync.master_true_peak_dbtp);

                let ch1_str = ch1_preset_sync.read().map(|g| g.clone()).unwrap_or_else(|_| "vocal".into());
                let ch2_str = ch2_preset_sync.read().map(|g| g.clone()).unwrap_or_else(|_| "vocal".into());

                let p1 = InstrumentPreset::parse_str(&ch1_str).unwrap_or(InstrumentPreset::Vocal);
                let p2 = InstrumentPreset::parse_str(&ch2_str).unwrap_or(InstrumentPreset::Vocal);

                let (sug1_mode, sug1_48v) = presets::suggested_input(p1);
                let (sug2_mode, sug2_48v) = presets::suggested_input(p2);

                let ch1_voicing = ch1_voicing_sync.read().map(|g| g.clone()).unwrap_or_else(|_| "flat".into());
                let ch2_voicing = ch2_voicing_sync.read().map(|g| g.clone()).unwrap_or_else(|_| "flat".into());
                let ch1_mic_ir_loaded = ch1_ir_sync.load(Ordering::Relaxed);
                let ch2_mic_ir_loaded = ch2_ir_sync.load(Ordering::Relaxed);

                let msg = RemoteMessage::StateSync {
                    input1,
                    input2,
                    outputs,
                    meters: LiveMetersState {
                        in_l_dbfs,
                        in_r_dbfs,
                        out_l_dbfs,
                        out_r_dbfs,
                        comp_gr_db,
                        lim_gr_db,
                        integrated_lufs,
                        true_peak_dbtp,
                    },
                    global_bypass: bypass_sync.load(Ordering::Relaxed),
                    source_mode: if source_mode_sync.load(Ordering::Relaxed) { "program".into() } else { "vocal".into() },
                    ch1_preset: ch1_str,
                    ch2_preset: ch2_str,
                    ch1_suggested_mode: format!("{:?}", sug1_mode),
                    ch1_suggested_phantom: sug1_48v,
                    ch2_suggested_mode: format!("{:?}", sug2_mode),
                    ch2_suggested_phantom: sug2_48v,
                    ch1_voicing,
                    ch2_voicing,
                    ch1_mic_ir_loaded,
                    ch2_mic_ir_loaded,
                };

                if let Ok(json_str) = serde_json::to_string(&msg) {
                    let _ = bcast_tx.send(json_str);
                }
            }
        });
    }

    pub fn router(&self) -> Router {
        // Ensure broadcast worker is running if router is queried inside a runtime
        if tokio::runtime::Handle::try_current().is_ok() {
            Self::spawn_broadcast_worker(&self.state);
        }

        Router::new()
            .route("/", get(index_handler))
            .route("/manifest.webmanifest", get(manifest_handler))
            .route("/sw.js", get(sw_handler))
            .route("/icon-192.png", get(icon_192_handler))
            .route("/icon-512.png", get(icon_512_handler))
            .route("/ws", get(ws_handler))
            .route("/api/auth", get(auth_handler))
            .route("/api/status", get(status_handler))
            .route("/api/meters", get(meters_handler))
            .layer(tower_http::cors::CorsLayer::permissive())
            .with_state(Arc::clone(&self.state))
    }

    pub async fn run(self) -> Result<()> {
        Self::spawn_broadcast_worker(&self.state);
        let app = self.router();
        let addr = SocketAddr::from(([0, 0, 0, 0], self.port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        println!("📡 Wireless Touch Tablet Remote online: http://0.0.0.0:{}", self.port);
        if self.state.token.is_some() {
            println!("🔒 Remote access token protection: ACTIVE");
        }
        axum::serve(listener, app).await?;
        Ok(())
    }
}

fn handle_hid_cmd(
    hw: &Option<HardwareController>,
    cmd: CoalescedHidCommand,
    pending_gain: &mut [Option<u8>; 2],
    pending_mon_vol: &mut Option<u8>,
    pending_hp1_vol: &mut Option<u8>,
    pending_hp2_vol: &mut Option<u8>,
) {
    match cmd {
        CoalescedHidCommand::Gain { input, val } => {
            if (input as usize) < 2 {
                pending_gain[input as usize] = Some(val);
            }
        }
        CoalescedHidCommand::MonitorVol { val } => {
            *pending_mon_vol = Some(val);
        }
        CoalescedHidCommand::Hp1Vol { val } => {
            *pending_hp1_vol = Some(val);
        }
        CoalescedHidCommand::Hp2Vol { val } => {
            *pending_hp2_vol = Some(val);
        }
        // Discrete toggles execute immediately on the dedicated OS thread without waiting
        CoalescedHidCommand::Phantom { input, val } => {
            if let Some(h) = hw {
                let _ = h.set_phantom(input, val);
            }
        }
        CoalescedHidCommand::Phase { input, val } => {
            if let Some(h) = hw {
                let _ = h.set_phase(input, val);
            }
        }
        CoalescedHidCommand::Mode { input, mode } => {
            if let Some(h) = hw {
                let _ = h.set_preamp_mode(input, mode);
            }
        }
        CoalescedHidCommand::MonitorMute { val } => {
            if let Some(h) = hw {
                let _ = h.set_monitor_mute(val);
            }
        }
    }
}

fn default_fallback_state() -> (PreampChannelState, PreampChannelState, OutputLevelsState) {
    (
        PreampChannelState {
            gain_db: 30,
            mode: "Mic".into(),
            phantom: false,
            phase_invert: false,
        },
        PreampChannelState {
            gain_db: 30,
            mode: "Mic".into(),
            phantom: false,
            phase_invert: false,
        },
        OutputLevelsState {
            monitor_step: 32,
            monitor_mute: false,
            hp1_step: 32,
            hp2_step: 32,
        },
    )
}

fn is_authorized(
    token_opt: &Option<String>,
    query: &HashMap<String, String>,
    headers: &HeaderMap,
) -> bool {
    let Some(expected) = token_opt else {
        return true; // No token configured -> open access
    };

    if let Some(t) = query.get("token") {
        if t == expected {
            return true;
        }
    }

    if let Some(auth) = headers.get("authorization").and_then(|h| h.to_str().ok()) {
        if let Some(bearer) = auth.strip_prefix("Bearer ") {
            if bearer.trim() == expected {
                return true;
            }
        }
    }

    if let Some(custom) = headers.get("x-remote-token").and_then(|h| h.to_str().ok()) {
        if custom.trim() == expected {
            return true;
        }
    }

    false
}

async fn index_handler() -> Html<&'static str> {
    Html(TABLET_TOUCH_HTML)
}

async fn manifest_handler() -> Response {
    ([(header::CONTENT_TYPE, "application/manifest+json")], MANIFEST_JSON).into_response()
}

async fn sw_handler() -> Response {
    ([(header::CONTENT_TYPE, "text/javascript")], SW_JS).into_response()
}

async fn icon_192_handler() -> Response {
    ([(header::CONTENT_TYPE, "image/png")], ICON_192_PNG).into_response()
}

async fn icon_512_handler() -> Response {
    ([(header::CONTENT_TYPE, "image/png")], ICON_512_PNG).into_response()
}

async fn auth_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !is_authorized(&state.token, &query, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "authenticated": false,
                "token_required": true,
                "error": "Unauthorized: invalid or missing remote token"
            })),
        )
            .into_response();
    }

    Json(serde_json::json!({
        "authenticated": true,
        "token_required": state.token.is_some()
    }))
    .into_response()
}

async fn status_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !is_authorized(&state.token, &query, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Unauthorized: invalid or missing remote token"})),
        )
            .into_response();
    }

    let hw_online = state.hw.is_some();
    let ch1_str = state.ch1_preset.read().map(|g| g.clone()).unwrap_or_else(|_| "vocal".into());
    let ch2_str = state.ch2_preset.read().map(|g| g.clone()).unwrap_or_else(|_| "vocal".into());
    Json(serde_json::json!({
        "status": "online",
        "hardware_detected": hw_online,
        "sample_rate": state.sample_rate as u32,
        "service": "DeskDSP Control Wireless Tablet Remote",
        "token_required": state.token.is_some(),
        "global_bypass": state.global_bypass.load(Ordering::Relaxed),
        "source_mode": if state.source_mode.load(Ordering::Relaxed) { "program" } else { "vocal" },
        "ch1_preset": ch1_str,
        "ch2_preset": ch2_str,
    }))
    .into_response()
}

async fn meters_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !is_authorized(&state.token, &query, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "Unauthorized: invalid or missing remote token"})),
        )
            .into_response();
    }

    let in_l = AudioMeters::load_f32(&state.meters.in_l_peak);
    let in_r = AudioMeters::load_f32(&state.meters.in_r_peak);
    let out_l = AudioMeters::load_f32(&state.meters.out_l_peak);
    let out_r = AudioMeters::load_f32(&state.meters.out_r_peak);
    let in_db = if in_l > 1e-4 { 20.0 * in_l.log10() } else { -80.0 };
    let in_r_db = if in_r > 1e-4 { 20.0 * in_r.log10() } else { -80.0 };
    let out_db = if out_l > 1e-4 { 20.0 * out_l.log10() } else { -80.0 };
    let out_r_db = if out_r > 1e-4 { 20.0 * out_r.log10() } else { -80.0 };

    Json(serde_json::json!({
        "in_l_dbfs": in_db,
        "in_r_dbfs": in_r_db,
        "out_l_dbfs": out_db,
        "out_r_dbfs": out_r_db,
        "comp_gr_db": AudioMeters::load_f32(&state.meters.comp_gr_db),
        "lim_gr_db": AudioMeters::load_f32(&state.meters.master_limiter_gr_db),
        "integrated_lufs": AudioMeters::load_f32(&state.meters.integrated_lufs),
        "true_peak_dbtp": AudioMeters::load_f32(&state.meters.master_true_peak_dbtp),
        "tuner_freq_hz": AudioMeters::load_f32(&state.meters.tuner_detected_freq)
    }))
    .into_response()
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !is_authorized(&state.token, &query, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            "Unauthorized: invalid or missing remote token",
        )
            .into_response();
    }

    ws.on_upgrade(move |socket| handle_tablet_socket(socket, state))
        .into_response()
}

async fn handle_tablet_socket(mut socket: WebSocket, state: Arc<AppState>) {
    let mut rx = state.broadcast_tx.subscribe();

    loop {
        tokio::select! {
            // Outgoing broadcast message to tablet
            Ok(msg_str) = rx.recv() => {
                if socket.send(Message::Text(msg_str)).await.is_err() {
                    break;
                }
            }
            // Incoming message from tablet
            Some(Ok(msg)) = socket.recv() => {
                if let Message::Text(text) = msg {
                    if let Ok(cmd) = serde_json::from_str::<RemoteMessage>(&text) {
                        state.dispatch_remote_message(cmd);
                    }
                }
            }
            else => break,
        }
    }
}
