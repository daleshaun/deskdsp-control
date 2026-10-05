use std::io::{self, Stdout};
use std::time::Duration;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
    Terminal,
};

use crate::audio::{AudioEngine, AudioMeters};
use crate::dsp::tuner::Scale;
use crate::dsp::StereoDspNode;
use crate::hardware::HardwareController;
use antelope_protocol::PreampMode;

pub struct WorkstationApp {
    hardware: Option<HardwareController>,
    audio: AudioEngine,
    selected_channel: u8, // 0 for In 1, 1 for In 2
    preamp_gain: [u8; 2],
    preamp_mode: [PreampMode; 2],
    phantom: [bool; 2],
    phase: [bool; 2],
    monitor_vol: u8,
    status_msg: String,
}

impl WorkstationApp {
    pub fn new(hardware: Option<HardwareController>, audio: AudioEngine) -> Self {
        Self {
            hardware,
            audio,
            selected_channel: 0,
            preamp_gain: [30, 30],
            preamp_mode: [PreampMode::Mic, PreampMode::Mic],
            phantom: [false, false],
            phase: [false, false],
            monitor_vol: 0x20,
            status_msg: "DeskDSP Control active. Real-time audio streaming.".into(),
        }
    }

    pub fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let res = self.event_loop(&mut terminal);

        disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
        terminal.show_cursor()?;

        res
    }

    fn event_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        loop {
            // Update local state from hardware snapshot if available
            if let Some(hw) = &self.hardware {
                if let Some(snap) = hw.get_snapshot() {
                    let pre = &snap.preamp;
                    self.preamp_gain[0] = pre.input1.gain_raw;
                    self.preamp_gain[1] = pre.input2.gain_raw;
                    self.preamp_mode[0] = pre.input1.mode;
                    self.preamp_mode[1] = pre.input2.mode;
                    self.phantom[0] = pre.input1.phantom_on;
                    self.phantom[1] = pre.input2.phantom_on;
                    self.phase[0] = (pre.input1.mode_raw & 0x40) != 0;
                    self.phase[1] = (pre.input2.mode_raw & 0x40) != 0;
                    self.monitor_vol = snap.outputs[0].volume;
                }
            }

            terminal.draw(|f| self.render_ui(f))?;

            if event::poll(Duration::from_millis(50))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('1') => {
                            self.selected_channel = 0;
                            self.status_msg = "Selected Input 1".into();
                        }
                        KeyCode::Char('2') => {
                            self.selected_channel = 1;
                            self.status_msg = "Selected Input 2".into();
                        }
                        // Hardware Preamp Gain
                        KeyCode::Char('g') => {
                            let ch = self.selected_channel as usize;
                            self.preamp_gain[ch] = (self.preamp_gain[ch] + 1).min(65);
                            if let Some(hw) = &self.hardware {
                                let _ = hw.set_preamp_gain(self.selected_channel, self.preamp_gain[ch]);
                            }
                            self.status_msg = format!("Input {} Gain: {} dB", ch + 1, self.preamp_gain[ch]);
                        }
                        KeyCode::Char('G') => {
                            let ch = self.selected_channel as usize;
                            self.preamp_gain[ch] = self.preamp_gain[ch].saturating_sub(1);
                            if let Some(hw) = &self.hardware {
                                let _ = hw.set_preamp_gain(self.selected_channel, self.preamp_gain[ch]);
                            }
                            self.status_msg = format!("Input {} Gain: {} dB", ch + 1, self.preamp_gain[ch]);
                        }
                        // Hardware 48V Phantom
                        KeyCode::Char('p') => {
                            let ch = self.selected_channel as usize;
                            self.phantom[ch] = !self.phantom[ch];
                            if let Some(hw) = &self.hardware {
                                let _ = hw.set_phantom(self.selected_channel, self.phantom[ch]);
                            }
                            self.status_msg = format!("Input {} 48V: {}", ch + 1, if self.phantom[ch] { "ON" } else { "OFF" });
                        }
                        // Hardware Mode (Mic / Line / Hi-Z)
                        KeyCode::Char('m') => {
                            let ch = self.selected_channel as usize;
                            self.preamp_mode[ch] = match self.preamp_mode[ch] {
                                PreampMode::Mic => PreampMode::Line,
                                PreampMode::Line => PreampMode::HiZ,
                                PreampMode::HiZ => PreampMode::Mic,
                                _ => PreampMode::Mic,
                            };
                            if let Some(hw) = &self.hardware {
                                let _ = hw.set_preamp_mode(self.selected_channel, self.preamp_mode[ch]);
                            }
                            self.status_msg = format!("Input {} Mode: {:?}", ch + 1, self.preamp_mode[ch]);
                        }
                        // Hardware Monitor Volume
                        KeyCode::Char('v') => {
                            self.monitor_vol = self.monitor_vol.saturating_sub(2);
                            if let Some(hw) = &self.hardware {
                                let _ = hw.set_monitor_volume(self.monitor_vol);
                            }
                            self.status_msg = format!("Monitor Attenuation: {} (0=Unity)", self.monitor_vol);
                        }
                        KeyCode::Char('V') => {
                            self.monitor_vol = (self.monitor_vol + 2).min(127);
                            if let Some(hw) = &self.hardware {
                                let _ = hw.set_monitor_volume(self.monitor_vol);
                            }
                            self.status_msg = format!("Monitor Attenuation: {} (127=Mute)", self.monitor_vol);
                        }
                        // Channel Strip DSP Toggles
                        KeyCode::Char('e') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                cs.eq.bypassed = !cs.eq.bypassed;
                                self.status_msg = format!("Channel {} EQ: {}", self.selected_channel + 1, if cs.eq.bypassed { "BYPASS" } else { "ACTIVE" });
                            }
                        }
                        KeyCode::Char('c') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                cs.compressor.bypassed = !cs.compressor.bypassed;
                                self.status_msg = format!("Channel {} Compressor: {}", self.selected_channel + 1, if cs.compressor.bypassed { "BYPASS" } else { "ACTIVE" });
                            }
                        }
                        KeyCode::Char('t') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                cs.tuner.bypassed = !cs.tuner.bypassed;
                                self.status_msg = format!("Channel {} Vocal Tuner: {}", self.selected_channel + 1, if cs.tuner.bypassed { "BYPASS" } else { "ACTIVE" });
                            }
                        }
                        KeyCode::Char('k') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                cs.tuner.quantizer.scale = match cs.tuner.quantizer.scale {
                                    Scale::Chromatic => Scale::Major,
                                    Scale::Major => Scale::NaturalMinor,
                                    Scale::NaturalMinor => Scale::MajorPentatonic,
                                    _ => Scale::Chromatic,
                                };
                                self.status_msg = format!("Tuner Scale: {:?}", cs.tuner.quantizer.scale);
                            }
                        }
                        KeyCode::Char('s') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                cs.saturation.bypassed = !cs.saturation.bypassed;
                                self.status_msg = format!("Channel {} Saturation: {}", self.selected_channel + 1, if cs.saturation.bypassed { "BYPASS" } else { "ACTIVE" });
                            }
                        }
                        KeyCode::Char('x') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                cs.gate.bypassed = !cs.gate.bypassed;
                                self.status_msg = format!("Channel {} Noise Gate: {}", self.selected_channel + 1, if cs.gate.bypassed { "BYPASS" } else { "ACTIVE" });
                            }
                        }
                        KeyCode::Char('d') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                cs.deesser.bypassed = !cs.deesser.bypassed;
                                self.status_msg = format!("Channel {} De-Esser: {}", self.selected_channel + 1, if cs.deesser.bypassed { "BYPASS" } else { "ACTIVE" });
                            }
                        }
                        // Master Chain Toggles
                        KeyCode::Char('b') => {
                            if let Ok(mut master) = self.audio.master_chain.lock() {
                                master.glue.bypassed = !master.glue.bypassed;
                                self.status_msg = format!("Master Glue Compressor: {}", if master.glue.bypassed { "BYPASS" } else { "ACTIVE" });
                            }
                        }
                        KeyCode::Char('l') => {
                            if let Ok(mut master) = self.audio.master_chain.lock() {
                                master.limiter.bypassed = !master.limiter.bypassed;
                                self.status_msg = format!("Master Brickwall Limiter: {}", if master.limiter.bypassed { "BYPASS" } else { "ACTIVE" });
                            }
                        }
                        // Adjust Compressor Threshold
                        KeyCode::Char('[') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                cs.compressor.threshold_db = (cs.compressor.threshold_db - 2.0).max(-40.0);
                                self.status_msg = format!("Comp Thresh: {:.1} dB", cs.compressor.threshold_db);
                            }
                        }
                        KeyCode::Char(']') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                cs.compressor.threshold_db = (cs.compressor.threshold_db + 2.0).min(0.0);
                                self.status_msg = format!("Comp Thresh: {:.1} dB", cs.compressor.threshold_db);
                            }
                        }
                        // Adjust Saturation Drive
                        KeyCode::Char('{') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                let new_drive = (cs.saturation.drive + 0.5).min(10.0);
                                cs.saturation.set_drive(new_drive);
                                self.status_msg = format!("Saturation Drive: {:.1}x", cs.saturation.drive);
                            }
                        }
                        KeyCode::Char('}') => {
                            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
                            if let Ok(mut cs) = cs_arc.lock() {
                                let new_drive = (cs.saturation.drive - 0.5).max(1.0);
                                cs.saturation.set_drive(new_drive);
                                self.status_msg = format!("Saturation Drive: {:.1}x", cs.saturation.drive);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }

    fn render_ui(&self, f: &mut ratatui::Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3),  // Title banner
                Constraint::Length(5),  // Hardware front-end
                Constraint::Length(7),  // Vocal Channel Strip
                Constraint::Length(5),  // Master Chain
                Constraint::Length(6),  // Visual Audio Meters
                Constraint::Length(3),  // Status / key help
            ])
            .split(f.area());

        // 1. Header
        let title = Paragraph::new("⚡ DESKDSP CONTROL — ZEN GO SYNERGY CORE HOST WORKSTATION ⚡")
            .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL).title("Workstation Engine"));
        f.render_widget(title, chunks[0]);

        // 2. Hardware Controls
        let in1_str = format!("Input 1: {:<5} | Gain: {:>2} dB | +48V: {:<3} | Phase: {}", 
            format!("{:?}", self.preamp_mode[0]), self.preamp_gain[0], if self.phantom[0] { "ON" } else { "OFF" }, if self.phase[0] { "INV" } else { "NORM" });
        let in2_str = format!("Input 2: {:<5} | Gain: {:>2} dB | +48V: {:<3} | Phase: {}", 
            format!("{:?}", self.preamp_mode[1]), self.preamp_gain[1], if self.phantom[1] { "ON" } else { "OFF" }, if self.phase[1] { "INV" } else { "NORM" });
        let out_str = format!("Monitor Out Attenuation: {:>3} (0=Unity, 127=Mute) | Selected Ch: {}", self.monitor_vol, self.selected_channel + 1);

        let hw_paragraph = Paragraph::new(vec![
            Line::from(vec![Span::raw(in1_str)]),
            Line::from(vec![Span::raw(in2_str)]),
            Line::from(vec![Span::styled(out_str, Style::default().fg(Color::Yellow))]),
        ])
        .block(Block::default().borders(Borders::ALL).title("Hardware Front-End (USB HID Control Plane)"));
        f.render_widget(hw_paragraph, chunks[1]);

        // 3. Vocal Channel Strip Status
        let (gate_st, deess_st, eq_st, comp_st, tuner_st, sat_st) = {
            let cs_arc = if self.selected_channel == 0 { &self.audio.channel_strip_1 } else { &self.audio.channel_strip_2 };
            if let Ok(cs) = cs_arc.lock() {
                (
                    if cs.gate.bypassed { "BYPASS" } else { "ON (-42dB)" },
                    if cs.deesser.bypassed { "BYPASS" } else { "ON (6.5kHz)" },
                    if cs.eq.bypassed { "BYPASS" } else { "ON (HPF 80Hz | LS 100 | Mid 450/3.2k | HS 10k)" },
                    if cs.compressor.bypassed { "BYPASS" } else { "ON (Opto/FET Soft-Knee)" },
                    if cs.tuner.bypassed { "BYPASS" } else { "ACTIVE (YIN + Scale Quantizer + PSOLA)" },
                    if cs.saturation.bypassed { "BYPASS" } else { "ON (Tube/Tape Saturation + DC Blocker)" },
                )
            } else {
                ("ON", "ON", "ON", "ON", "ON", "ON")
            }
        };

        let cs_paragraph = Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Gate: ", Style::default().fg(Color::Green)), Span::raw(format!("{:<10} ", gate_st)),
                Span::styled("De-Esser: ", Style::default().fg(Color::Green)), Span::raw(format!("{:<12} ", deess_st)),
                Span::styled("4-Band EQ: ", Style::default().fg(Color::Green)), Span::raw(eq_st),
            ]),
            Line::from(vec![
                Span::styled("Compressor: ", Style::default().fg(Color::Green)), Span::raw(format!("{:<22} ", comp_st)),
                Span::styled("Tuner: ", Style::default().fg(Color::Yellow)), Span::raw(tuner_st),
            ]),
            Line::from(vec![
                Span::styled("Saturation: ", Style::default().fg(Color::Green)), Span::raw(sat_st),
            ]),
        ])
        .block(Block::default().borders(Borders::ALL).title(format!("Channel Strip {} (Tracking DSP Pipeline)", self.selected_channel + 1)));
        f.render_widget(cs_paragraph, chunks[2]);

        // 4. Master Chain Status
        let (m_eq, m_mb, m_w, m_glue, m_lim) = if let Ok(m) = self.audio.master_chain.lock() {
            (
                if m.eq.is_bypassed() { "BYPASS" } else { "ON (5-Band Min-Phase)" },
                if m.multiband.is_bypassed() { "BYPASS" } else { "ON (3-Band LR4)" },
                if m.stereo_width.is_bypassed() { "BYPASS" } else { "ON (M/S Width + Mono-Bass)" },
                if m.glue.is_bypassed() { "BYPASS" } else { "ON (VCA Bus + Tape)" },
                if m.limiter.is_bypassed() { "BYPASS" } else { "ON (-1.0 dBTP Ceiling)" },
            )
        } else {
            ("ON", "ON", "ON", "ON", "ON")
        };

        let master_paragraph = Paragraph::new(vec![
            Line::from(vec![
                Span::styled("EQ: ", Style::default().fg(Color::Cyan)), Span::raw(format!("{:<22} ", m_eq)),
                Span::styled("Multiband: ", Style::default().fg(Color::Cyan)), Span::raw(format!("{:<16} ", m_mb)),
                Span::styled("Width: ", Style::default().fg(Color::Cyan)), Span::raw(m_w),
            ]),
            Line::from(vec![
                Span::styled("Glue Comp: ", Style::default().fg(Color::Cyan)), Span::raw(format!("{:<18} ", m_glue)),
                Span::styled("Limiter: ", Style::default().fg(Color::Red)), Span::raw(m_lim),
            ]),
        ])
        .block(Block::default().borders(Borders::ALL).title("Master Chain (Mix Bus / True-Peak Mastering Pipeline)"));
        f.render_widget(master_paragraph, chunks[3]);

        // 5. Visual Audio Meters & Telemetry
        let in_peak_linear = AudioMeters::load_f32(&self.audio.meters.in_l_peak).max(AudioMeters::load_f32(&self.audio.meters.in_r_peak));
        let in_db = if in_peak_linear > 1e-4 { 20.0 * in_peak_linear.log10() } else { -80.0 };
        let in_ratio = ((in_db + 60.0) / 60.0).clamp(0.0, 1.0);

        let comp_gr_db = AudioMeters::load_f32(&self.audio.meters.comp_gr_db);
        let comp_gr_ratio = (comp_gr_db / 20.0).clamp(0.0, 1.0);

        let lim_gr_db = AudioMeters::load_f32(&self.audio.meters.master_limiter_gr_db);
        let lim_gr_ratio = (lim_gr_db / 12.0).clamp(0.0, 1.0);

        let out_peak_linear = AudioMeters::load_f32(&self.audio.meters.out_l_peak).max(AudioMeters::load_f32(&self.audio.meters.out_r_peak));
        let out_db = if out_peak_linear > 1e-4 { 20.0 * out_peak_linear.log10() } else { -80.0 };
        let out_ratio = ((out_db + 60.0) / 60.0).clamp(0.0, 1.0);

        let int_lufs = AudioMeters::load_f32(&self.audio.meters.integrated_lufs);
        let tp_dbtp = AudioMeters::load_f32(&self.audio.meters.master_true_peak_dbtp);
        let detected_hz = AudioMeters::load_f32(&self.audio.meters.tuner_detected_freq);
        let cents = AudioMeters::load_f32(&self.audio.meters.tuner_cents);

        let meter_layout = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(chunks[4]);

        let in_gauge = Gauge::default()
            .label(format!("Input 1/2 Peak: {:>5.1} dBFS", in_db))
            .gauge_style(Style::default().fg(if in_db > -3.0 { Color::Red } else { Color::Green }))
            .ratio(in_ratio as f64);
        f.render_widget(in_gauge, meter_layout[0]);

        let gr_gauge = Gauge::default()
            .label(format!("Vocal GR: -{:>4.1} dB | Limiter GR: -{:>4.1} dB", comp_gr_db, lim_gr_db))
            .gauge_style(Style::default().fg(Color::Yellow))
            .ratio((comp_gr_ratio.max(lim_gr_ratio)) as f64);
        f.render_widget(gr_gauge, meter_layout[1]);

        let out_gauge = Gauge::default()
            .label(format!("Master Peak: {:>5.1} dBFS | True-Peak: {:>5.1} dBTP | LUFS: {:>5.1}", out_db, tp_dbtp, int_lufs))
            .gauge_style(Style::default().fg(if out_db > -1.0 { Color::Red } else { Color::Cyan }))
            .ratio(out_ratio as f64);
        f.render_widget(out_gauge, meter_layout[2]);

        let tuner_str = if detected_hz > 20.0 {
            format!("Tuner Tracking: {:>6.1} Hz | Deviation: {:>+5.1} cents", detected_hz, cents)
        } else {
            "Tuner Tracking: (Listening for vocal pitch...)".into()
        };
        let tuner_line = Paragraph::new(tuner_str).style(Style::default().fg(Color::Magenta));
        f.render_widget(tuner_line, meter_layout[3]);

        // 6. Help / Status bar
        let help_text = format!("[1/2] Ch | [g/G] Gain | [p] 48V | [m] Mode | [v/V] Vol | [e/c/t/s/x/d] DSP | [k] Scale | [b/l] Master | [q] Quit  >> {}", self.status_msg);
        let help_para = Paragraph::new(help_text)
            .style(Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD))
            .block(Block::default().borders(Borders::ALL).title("DeskDSP Control Plane"));
        f.render_widget(help_para, chunks[5]);
    }
}
