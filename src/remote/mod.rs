//! Wireless Touch Tablet Remote Server for DeskDSP Control & Antelope Zen Go.
//!
//! Architectural Guarantees:
//! 1. Zero locks on the real-time audio thread: server only interacts with
//!    `HardwareController` and lock-free atomic `AudioMeters`.
//! 2. Coalesced HID writes: high-frequency touch scrub events (gain/volume)
//!    are debounced and coalesced to prevent USB HID buffer congestion.
//! 3. Hardware -> Tablet state sync: real-time 0x73 telemetry snapshots from Zen Go
//!    are pushed over WebSocket to all connected tablets.
//! 4. Self-contained: embeds responsive tablet touch HTML5/CSS3 application.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::audio::{AudioMeters};
use crate::hardware::HardwareController;
use antelope_protocol::PreampMode;

mod assets;
use assets::TABLET_TOUCH_HTML;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RemoteMessage {
    #[serde(rename = "state_sync")]
    StateSync {
        input1: PreampChannelState,
        input2: PreampChannelState,
        outputs: OutputLevelsState,
        meters: LiveMetersState,
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
}

/// Commands sent to the coalescing HID worker thread
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
    cmd_tx: mpsc::Sender<CoalescedHidCommand>,
    broadcast_tx: broadcast::Sender<String>,
}

pub struct TabletRemoteServer {
    port: u16,
    state: Arc<AppState>,
}

impl TabletRemoteServer {
    pub fn new(hw: Option<HardwareController>, meters: Arc<AudioMeters>, port: u16) -> Self {
        let (cmd_tx, mut cmd_rx) = mpsc::channel::<CoalescedHidCommand>(128);
        let (broadcast_tx, _) = broadcast::channel::<String>(64);

        // 1. Dedicated Coalescing HID Worker
        // Eliminates USB HID congestion under rapid touch fader scrubbing
        let hw_clone = hw.clone();
        tokio::spawn(async move {
            let mut pending_gain: [Option<u8>; 2] = [None, None];
            let mut pending_mon_vol: Option<u8> = None;
            let mut pending_hp1_vol: Option<u8> = None;
            let mut pending_hp2_vol: Option<u8> = None;

            let mut interval = tokio::time::interval(Duration::from_millis(25)); // 40Hz write rate

            loop {
                tokio::select! {
                    Some(cmd) = cmd_rx.recv() => {
                        match cmd {
                            CoalescedHidCommand::Gain { input, val } => {
                                if (input as usize) < 2 {
                                    pending_gain[input as usize] = Some(val);
                                }
                            }
                            CoalescedHidCommand::MonitorVol { val } => {
                                pending_mon_vol = Some(val);
                            }
                            CoalescedHidCommand::Hp1Vol { val } => {
                                pending_hp1_vol = Some(val);
                            }
                            CoalescedHidCommand::Hp2Vol { val } => {
                                pending_hp2_vol = Some(val);
                            }
                            // Discrete commands (buttons/toggles) execute immediately without waiting
                            CoalescedHidCommand::Phantom { input, val } => {
                                if let Some(h) = &hw_clone {
                                    let _ = h.set_phantom(input, val);
                                }
                            }
                            CoalescedHidCommand::Phase { input, val } => {
                                if let Some(h) = &hw_clone {
                                    let _ = h.set_phase(input, val);
                                }
                            }
                            CoalescedHidCommand::Mode { input, mode } => {
                                if let Some(h) = &hw_clone {
                                    let _ = h.set_preamp_mode(input, mode);
                                }
                            }
                            CoalescedHidCommand::MonitorMute { val } => {
                                if let Some(h) = &hw_clone {
                                    let _ = h.set_monitor_mute(val);
                                }
                            }
                        }
                    }
                    _ = interval.tick() => {
                        // Flush coalesced continuous values to hardware
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
                    }
                }
            }
        });

        // 2. Real-Time Hardware -> Tablet State Sync Broadcast Worker
        let hw_sync = hw.clone();
        let meters_sync = Arc::clone(&meters);
        let bcast_tx = broadcast_tx.clone();

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
                    },
                };

                if let Ok(json_str) = serde_json::to_string(&msg) {
                    let _ = bcast_tx.send(json_str);
                }
            }
        });

        let state = Arc::new(AppState {
            hw,
            meters,
            cmd_tx,
            broadcast_tx,
        });

        Self { port, state }
    }

    pub fn router(&self) -> Router {
        Router::new()
            .route("/", get(index_handler))
            .route("/ws", get(ws_handler))
            .route("/api/status", get(status_handler))
            .route("/api/meters", get(meters_handler))
            .layer(tower_http::cors::CorsLayer::permissive())
            .with_state(Arc::clone(&self.state))
    }

    pub async fn run(self) -> Result<()> {
        let app = self.router();
        let addr = SocketAddr::from(([0, 0, 0, 0], self.port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        println!("📡 Wireless Touch Tablet Remote online: http://0.0.0.0:{}", self.port);
        axum::serve(listener, app).await?;
        Ok(())
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

async fn index_handler() -> Html<&'static str> {
    Html(TABLET_TOUCH_HTML)
}

async fn status_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
    let hw_online = state.hw.is_some();
    Json(serde_json::json!({
        "status": "online",
        "hardware_detected": hw_online,
        "sample_rate": 48000,
        "service": "DeskDSP Control Wireless Tablet Remote"
    }))
}

async fn meters_handler(State(state): State<Arc<AppState>>) -> Json<serde_json::Value> {
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
        "tuner_freq_hz": AudioMeters::load_f32(&state.meters.tuner_detected_freq)
    }))
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_tablet_socket(socket, state))
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
                        match cmd {
                            RemoteMessage::SetGain { input, gain_db } => {
                                let _ = state.cmd_tx.send(CoalescedHidCommand::Gain { input, val: gain_db }).await;
                            }
                            RemoteMessage::SetPhantom { input, enabled } => {
                                let _ = state.cmd_tx.send(CoalescedHidCommand::Phantom { input, val: enabled }).await;
                            }
                            RemoteMessage::SetPhase { input, enabled } => {
                                let _ = state.cmd_tx.send(CoalescedHidCommand::Phase { input, val: enabled }).await;
                            }
                            RemoteMessage::SetMode { input, mode } => {
                                let m = match mode.as_str() {
                                    "Mic" => PreampMode::Mic,
                                    "Line" => PreampMode::Line,
                                    "HiZ" => PreampMode::HiZ,
                                    _ => PreampMode::Mic,
                                };
                                let _ = state.cmd_tx.send(CoalescedHidCommand::Mode { input, mode: m }).await;
                            }
                            RemoteMessage::SetMonitorVolume { step } => {
                                let _ = state.cmd_tx.send(CoalescedHidCommand::MonitorVol { val: step }).await;
                            }
                            RemoteMessage::SetMonitorMute { enabled } => {
                                let _ = state.cmd_tx.send(CoalescedHidCommand::MonitorMute { val: enabled }).await;
                            }
                            RemoteMessage::SetHp1Volume { step } => {
                                let _ = state.cmd_tx.send(CoalescedHidCommand::Hp1Vol { val: step }).await;
                            }
                            RemoteMessage::SetHp2Volume { step } => {
                                let _ = state.cmd_tx.send(CoalescedHidCommand::Hp2Vol { val: step }).await;
                            }
                            _ => {}
                        }
                    }
                }
            }
            else => break,
        }
    }
}
