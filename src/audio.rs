//! Lock-Free Real-Time Audio Engine using cpal and rtrb SPSC queues.
//!
//! Real-Time Safety Guarantees:
//! 1. Zero mutex locks on the audio thread: audio callbacks OWN their DSP state.
//! 2. Zero memory allocations in `process()` / callbacks.
//! 3. Zero deallocations on the audio thread: removed nodes and swapped racks
//!    are transferred back to the UI/cleanup thread via an SPSC garbage queue.
//! 4. Lock-free SPSC audio ring buffer (`rtrb`) between input and output streams.
//! 5. Lock-free atomic meters read/write without contention.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream};

use crate::dsp::channel_strip::ChannelStrip;
use crate::dsp::master_chain::MasterChain;
use crate::dsp::rack::{MonoRack, StereoRack};
use crate::dsp::tuner::Scale;
use crate::dsp::{DspNode, StereoDspNode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    Program,
    Vocal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandTarget {
    Channel1,
    Channel2,
    Master,
}

#[allow(dead_code)]
pub enum AudioCommand {
    SetInputGain { target: CommandTarget, gain_db: f32 },
    SetOutputGain { target: CommandTarget, gain_db: f32 },
    SetPhaseInvert { target: CommandTarget, invert: bool },
    SetBypass { target: CommandTarget, node_index: usize, bypassed: bool },
    SetNodeBypass { target: CommandTarget, node_id: &'static str, bypassed: bool },
    ToggleBypass { target: CommandTarget, node_index: usize },
    SetSourceMode { mode: SourceMode },
    MoveNode { target: CommandTarget, from: usize, to: usize },
    SwapNodes { target: CommandTarget, i: usize, j: usize },
    InsertMonoNode { target: CommandTarget, index: usize, node: Box<dyn DspNode> },
    InsertStereoNode { target: CommandTarget, index: usize, node: Box<dyn StereoDspNode> },
    RemoveNode { target: CommandTarget, index: usize },
    SwapMonoRack { target: CommandTarget, rack: Box<MonoRack> },
    SwapStereoRack { target: CommandTarget, rack: Box<StereoRack> },
    SetParam { target: CommandTarget, param_id: &'static str, value: f32 },
    SetTunerScale { target: CommandTarget, scale: Scale },
    ResetAll { target: CommandTarget },
}

#[allow(dead_code)]
pub enum AudioGarbage {
    MonoNode(Box<dyn DspNode>),
    StereoNode(Box<dyn StereoDspNode>),
    MonoRack(Box<MonoRack>),
    StereoRack(Box<StereoRack>),
}

#[derive(Default)]
pub struct AudioMeters {
    // Stored as f32 bits in AtomicU32 for lock-free read from UI thread
    pub in_l_peak: AtomicU32,
    pub in_r_peak: AtomicU32,
    pub out_l_peak: AtomicU32,
    pub out_r_peak: AtomicU32,

    // Channel Strip Dynamics
    pub gate_reduction_db: AtomicU32,
    pub deesser_reduction_db: AtomicU32,
    pub comp_gr_db: AtomicU32,
    
    // Tuner Telemetry
    pub tuner_detected_freq: AtomicU32,
    pub tuner_cents: AtomicU32,

    // Master Dynamics & Loudness
    pub master_limiter_gr_db: AtomicU32,
    pub master_true_peak_dbtp: AtomicU32,
    pub momentary_lufs: AtomicU32,
    pub short_term_lufs: AtomicU32,
    pub integrated_lufs: AtomicU32,
}

impl AudioMeters {
    pub fn store_f32(atomic: &AtomicU32, val: f32) {
        atomic.store(val.to_bits(), Ordering::Relaxed);
    }

    pub fn load_f32(atomic: &AtomicU32) -> f32 {
        f32::from_bits(atomic.load(Ordering::Relaxed))
    }
}

pub struct AudioEngine {
    pub input_cmd_producer: Option<rtrb::Producer<AudioCommand>>,
    pub output_cmd_producer: Option<rtrb::Producer<AudioCommand>>,
    pub forwarder_cmd_tx: Option<std::sync::mpsc::Sender<AudioCommand>>,
    pub input_garbage_consumer: rtrb::Consumer<AudioGarbage>,
    pub output_garbage_consumer: rtrb::Consumer<AudioGarbage>,
    pub meters: Arc<AudioMeters>,
    pub global_bypass: Arc<AtomicBool>,
    pub source_mode: Arc<AtomicBool>, // true = Program mode (default, bypasses vocal strips), false = Vocal mode
    _input_stream: Option<Stream>,
    _output_stream: Option<Stream>,
    pub sample_rate: u32,
}

impl AudioEngine {
    pub fn new(input_device_name: Option<String>, output_device_name: Option<String>) -> Result<Self> {
        let host = cpal::default_host();
        
        let mut in_dev: Option<Device> = None;
        let mut out_dev: Option<Device> = None;

        // 1. Locate Input Device: prioritize CLI match, then Zen Go, then default
        if let Ok(devices) = host.input_devices() {
            let devs: Vec<_> = devices.collect();
            if let Some(target) = &input_device_name {
                let target_lower = target.to_lowercase();
                for d in &devs {
                    if let Ok(name) = d.name() {
                        if name.to_lowercase().contains(&target_lower) {
                            println!("✓ Matched requested audio input device: \"{name}\" (pattern: \"{target}\")");
                            in_dev = Some(d.clone());
                            break;
                        }
                    }
                }
                if in_dev.is_none() {
                    eprintln!("⚠️ WARNING: Requested input device \"{target}\" NOT found in system devices! Falling back to Zen Go / default...");
                }
            }
            if in_dev.is_none() {
                for d in &devs {
                    if let Ok(name) = d.name() {
                        if name.contains("Zen Go") {
                            println!("✓ Detected Zen Go input device: \"{name}\"");
                            in_dev = Some(d.clone());
                            break;
                        }
                    }
                }
            }
        }

        // 2. Locate Output Device: prioritize CLI match, then Zen Go, then default
        if let Ok(devices) = host.output_devices() {
            let devs: Vec<_> = devices.collect();
            if let Some(target) = &output_device_name {
                let target_lower = target.to_lowercase();
                for d in &devs {
                    if let Ok(name) = d.name() {
                        if name.to_lowercase().contains(&target_lower) {
                            println!("✓ Matched requested audio output device: \"{name}\" (pattern: \"{target}\")");
                            out_dev = Some(d.clone());
                            break;
                        }
                    }
                }
                if out_dev.is_none() {
                    eprintln!("⚠️ WARNING: Requested output device \"{target}\" NOT found in system devices! Falling back to Zen Go / default...");
                }
            }
            if out_dev.is_none() {
                for d in &devs {
                    if let Ok(name) = d.name() {
                        if name.contains("Zen Go") {
                            println!("✓ Detected Zen Go output device: \"{name}\"");
                            out_dev = Some(d.clone());
                            break;
                        }
                    }
                }
            }
        }

        // Fallback to system defaults if specific named device not found
        let in_dev = in_dev.or_else(|| host.default_input_device()).ok_or_else(|| anyhow!("No audio input device available"))?;
        let out_dev = out_dev.or_else(|| host.default_output_device()).ok_or_else(|| anyhow!("No audio output device available"))?;

        if let Ok(name) = in_dev.name() {
            println!("🎤 Active Audio Input:  \"{name}\"");
        }
        if let Ok(name) = out_dev.name() {
            println!("🔊 Active Audio Output: \"{name}\"");
        }

        let in_config = in_dev.default_input_config()?;
        let out_config = out_dev.default_output_config()?;

        let sample_rate = in_config.sample_rate().0;
        let meters = Arc::new(AudioMeters::default());
        let global_bypass = Arc::new(AtomicBool::new(false));
        let in_bypass = Arc::clone(&global_bypass);
        let source_mode = Arc::new(AtomicBool::new(true)); // DEFAULT TO PROGRAM MODE (skips vocal strip)
        let in_source_mode = Arc::clone(&source_mode);

        // 1. Lock-free SPSC ring buffers between input and output callbacks (8192 frames)
        let (mut ring_l_prod, mut ring_l_cons) = rtrb::RingBuffer::<f32>::new(8192);
        let (mut ring_r_prod, mut ring_r_cons) = rtrb::RingBuffer::<f32>::new(8192);

        // 2. Lock-free SPSC command queues: UI -> Audio Callbacks
        let (input_cmd_producer, mut in_cmd_consumer) = rtrb::RingBuffer::<AudioCommand>::new(256);
        let (output_cmd_producer, mut out_cmd_consumer) = rtrb::RingBuffer::<AudioCommand>::new(256);

        // 3. Lock-free SPSC garbage return queues: Audio Callbacks -> UI thread
        let (mut in_garbage_producer, input_garbage_consumer) = rtrb::RingBuffer::<AudioGarbage>::new(256);
        let (mut out_garbage_producer, output_garbage_consumer) = rtrb::RingBuffer::<AudioGarbage>::new(256);

        // Setup input stream: Audio thread OWNS ChannelStrip 1 & 2
        let in_channels = in_config.channels() as usize;
        let in_meters = Arc::clone(&meters);
        let mut cs1 = ChannelStrip::new(sample_rate as f32);
        let mut cs2 = ChannelStrip::new(sample_rate as f32);

        let in_stream = match in_config.sample_format() {
            SampleFormat::F32 => in_dev.build_input_stream(
                &in_config.into(),
                move |data: &[f32], _| {
                    // Top of callback: drain command queue and apply to owned state
                    while let Ok(cmd) = in_cmd_consumer.pop() {
                        apply_input_command(cmd, &mut in_garbage_producer, &mut cs1, &mut cs2);
                    }

                    let bypassed = in_bypass.load(Ordering::Relaxed);
                    let is_program = in_source_mode.load(Ordering::Relaxed);
                    let mut max_l = 0.0_f32;
                    let mut max_r = 0.0_f32;
                    let num_frames = data.len() / in_channels;

                    for f in 0..num_frames {
                        let raw_l = data[f * in_channels];
                        let raw_r = if in_channels > 1 { data[f * in_channels + 1] } else { raw_l };

                        max_l = max_l.max(raw_l.abs());
                        max_r = max_r.max(raw_r.abs());

                        // When globally bypassed or in PROGRAM mode, skip vocal channel strip processing!
                        let (proc_l, proc_r) = if bypassed || is_program {
                            (raw_l, raw_r)
                        } else {
                            (cs1.process(raw_l), cs2.process(raw_r))
                        };

                        // Push to lock-free SPSC ring buffers
                        let _ = ring_l_prod.push(proc_l);
                        let _ = ring_r_prod.push(proc_r);
                    }

                    // Update Channel Strip meters
                    if bypassed || is_program {
                        AudioMeters::store_f32(&in_meters.gate_reduction_db, 0.0);
                        AudioMeters::store_f32(&in_meters.deesser_reduction_db, 0.0);
                        AudioMeters::store_f32(&in_meters.comp_gr_db, 0.0);
                        AudioMeters::store_f32(&in_meters.tuner_detected_freq, 0.0);
                        AudioMeters::store_f32(&in_meters.tuner_cents, 0.0);
                    } else {
                        if let Some(gate) = cs1.gate() {
                            AudioMeters::store_f32(&in_meters.gate_reduction_db, gate.current_reduction_db());
                        }
                        if let Some(deesser) = cs1.deesser() {
                            AudioMeters::store_f32(&in_meters.deesser_reduction_db, deesser.gain_reduction_db());
                        }
                        if let Some(comp) = cs1.compressor() {
                            AudioMeters::store_f32(&in_meters.comp_gr_db, comp.gain_reduction_db());
                        }
                        if let Some(tuner) = cs1.tuner() {
                            AudioMeters::store_f32(&in_meters.tuner_detected_freq, tuner.detected_freq_hz.unwrap_or(0.0));
                            AudioMeters::store_f32(&in_meters.tuner_cents, tuner.cents_deviation);
                        }
                    }

                    AudioMeters::store_f32(&in_meters.in_l_peak, max_l);
                    AudioMeters::store_f32(&in_meters.in_r_peak, max_r);
                },
                |err| eprintln!("Input stream error: {err}"),
                None,
            )?,
            _ => return Err(anyhow!("Unsupported input sample format, expected F32")),
        };

        // Setup output stream: Audio thread OWNS MasterChain
        let out_channels = out_config.channels() as usize;
        let out_meters = Arc::clone(&meters);
        let out_bypass = Arc::clone(&global_bypass);
        let mut master = MasterChain::new(sample_rate as f32);

        let out_stream = match out_config.sample_format() {
            SampleFormat::F32 => out_dev.build_output_stream(
                &out_config.into(),
                move |data: &mut [f32], _| {
                    // Top of callback: drain command queue and apply to owned state
                    while let Ok(cmd) = out_cmd_consumer.pop() {
                        apply_output_command(cmd, &mut out_garbage_producer, &mut master);
                    }

                    let bypassed = out_bypass.load(Ordering::Relaxed);
                    let num_frames = data.len() / out_channels;
                    let mut max_l = 0.0_f32;
                    let mut max_r = 0.0_f32;

                    for f in 0..num_frames {
                        let in_l = ring_l_cons.pop().unwrap_or(0.0);
                        let in_r = ring_r_cons.pop().unwrap_or(0.0);

                        // When globally bypassed, pass clean bit-identical (in_l, in_r) straight through!
                        let (out_l, out_r) = if bypassed {
                            (in_l, in_r)
                        } else {
                            master.process_stereo(in_l, in_r)
                        };

                        max_l = max_l.max(out_l.abs());
                        max_r = max_r.max(out_r.abs());

                        data[f * out_channels] = out_l;
                        if out_channels > 1 {
                            data[f * out_channels + 1] = out_r;
                        }
                        for ch in 2..out_channels {
                            data[f * out_channels + ch] = 0.0;
                        }
                    }

                    // Telemetry for Master Chain
                    if bypassed {
                        AudioMeters::store_f32(&out_meters.master_limiter_gr_db, 0.0);
                    } else {
                        if let Some(limiter) = master.limiter() {
                            AudioMeters::store_f32(&out_meters.master_limiter_gr_db, limiter.gain_reduction_db());
                        }
                        if let Some(meter) = master.meter() {
                            AudioMeters::store_f32(&out_meters.master_true_peak_dbtp, meter.max_true_peak_dbtp);
                            AudioMeters::store_f32(&out_meters.momentary_lufs, meter.momentary_lufs);
                            AudioMeters::store_f32(&out_meters.short_term_lufs, meter.short_term_lufs);
                            AudioMeters::store_f32(&out_meters.integrated_lufs, meter.integrated_lufs);
                        }
                    }

                    AudioMeters::store_f32(&out_meters.out_l_peak, max_l);
                    AudioMeters::store_f32(&out_meters.out_r_peak, max_r);
                },
                |err| eprintln!("Output stream error: {err}"),
                None,
            )?,
            _ => return Err(anyhow!("Unsupported output sample format, expected F32")),
        };

        in_stream.play()?;
        out_stream.play()?;

        Ok(Self {
            input_cmd_producer: Some(input_cmd_producer),
            output_cmd_producer: Some(output_cmd_producer),
            forwarder_cmd_tx: None,
            input_garbage_consumer,
            output_garbage_consumer,
            meters,
            global_bypass,
            source_mode,
            _input_stream: Some(in_stream),
            _output_stream: Some(out_stream),
            sample_rate,
        })
    }

    pub fn set_command_sender(&mut self, tx: std::sync::mpsc::Sender<AudioCommand>) {
        self.forwarder_cmd_tx = Some(tx);
    }

    #[allow(dead_code)]
    pub fn new_default() -> Result<Self> {
        Self::new(None, None)
    }

    /// Move the SPSC command producers out of the engine once to transfer ownership
    /// to a dedicated remote command forwarder, preserving the single-producer invariant.
    pub fn take_command_producers(
        &mut self,
    ) -> Option<(rtrb::Producer<AudioCommand>, rtrb::Producer<AudioCommand>)> {
        match (self.input_cmd_producer.take(), self.output_cmd_producer.take()) {
            (Some(in_p), Some(out_p)) => Some((in_p, out_p)),
            _ => None,
        }
    }

    pub fn list_devices() -> Result<()> {
        let host = cpal::default_host();
        println!("=== Available Audio Input Devices ===");
        if let Ok(devices) = host.input_devices() {
            let mut count = 0;
            for (i, d) in devices.enumerate() {
                if let Ok(name) = d.name() {
                    count += 1;
                    println!("  [{i}] \"{name}\"");
                }
            }
            if count == 0 {
                println!("  (No input devices found)");
            }
        }
        println!("\n=== Available Audio Output Devices ===");
        if let Ok(devices) = host.output_devices() {
            let mut count = 0;
            for (i, d) in devices.enumerate() {
                if let Ok(name) = d.name() {
                    count += 1;
                    println!("  [{i}] \"{name}\"");
                }
            }
            if count == 0 {
                println!("  (No output devices found)");
            }
        }
        println!();
        Ok(())
    }

    /// Push a command to the audio thread via lock-free SPSC queue or forwarder channel.
    pub fn send_command(&mut self, cmd: AudioCommand) -> Result<(), AudioCommand> {
        if let AudioCommand::SetSourceMode { mode } = &cmd {
            self.source_mode.store(*mode == SourceMode::Program, Ordering::Relaxed);
        }
        if let Some(tx) = &self.forwarder_cmd_tx {
            return tx.send(cmd).map_err(|e| e.0);
        }
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
                if let Some(producer) = &mut self.output_cmd_producer {
                    producer.push(cmd).map_err(|e| match e { rtrb::PushError::Full(c) => c })
                } else {
                    Err(cmd)
                }
            }
            _ => {
                if let Some(producer) = &mut self.input_cmd_producer {
                    producer.push(cmd).map_err(|e| match e { rtrb::PushError::Full(c) => c })
                } else {
                    Err(cmd)
                }
            }
        }
    }

    /// Drain the garbage queue on the UI/cleanup thread and drop retired boxes.
    pub fn drain_garbage(&mut self) -> usize {
        let mut count = 0;
        while let Ok(g) = self.input_garbage_consumer.pop() {
            drop(g);
            count += 1;
        }
        while let Ok(g) = self.output_garbage_consumer.pop() {
            drop(g);
            count += 1;
        }
        count
    }
}

/// Applies input command on the audio thread with ZERO allocations and sends retired boxes
/// to the garbage return queue (NEVER dropped on the audio thread).
pub fn apply_input_command(
    cmd: AudioCommand,
    garbage_producer: &mut rtrb::Producer<AudioGarbage>,
    cs1: &mut ChannelStrip,
    cs2: &mut ChannelStrip,
) {
    match cmd {
        AudioCommand::SetInputGain { target, gain_db } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            cs.input_gain_db = gain_db;
        }
        AudioCommand::SetOutputGain { target, gain_db } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            cs.output_gain_db = gain_db;
        }
        AudioCommand::SetPhaseInvert { target, invert } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            cs.phase_invert = invert;
        }
        AudioCommand::SetBypass { target, node_index, bypassed } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            cs.rack.set_bypassed(node_index, bypassed);
        }
        AudioCommand::SetNodeBypass { target, node_id, bypassed } => {
            let apply = |cs: &mut ChannelStrip| {
                match node_id {
                    "hpf" => { if let Some(n) = cs.hpf_mut() { n.set_bypassed(bypassed); } }
                    "gate" => { if let Some(n) = cs.gate_mut() { n.set_bypassed(bypassed); } }
                    "deesser" => { if let Some(n) = cs.deesser_mut() { n.set_bypassed(bypassed); } }
                    "eq" => { if let Some(n) = cs.eq_mut() { n.set_bypassed(bypassed); } }
                    "comp" => { if let Some(n) = cs.compressor_mut() { n.set_bypassed(bypassed); } }
                    "tuner" => { if let Some(n) = cs.tuner_mut() { n.set_bypassed(bypassed); } }
                    "sat" => { if let Some(n) = cs.saturation_mut() { n.set_bypassed(bypassed); } }
                    _ => {}
                }
            };
            if target == CommandTarget::Channel1 {
                apply(cs1);
            } else if target == CommandTarget::Channel2 {
                apply(cs2);
            }
        }
        AudioCommand::ToggleBypass { target, node_index } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            cs.rack.toggle_bypass(node_index);
        }
        AudioCommand::MoveNode { target, from, to } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            cs.rack.move_node(from, to);
        }
        AudioCommand::SwapNodes { target, i, j } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            cs.rack.swap(i, j);
        }
        AudioCommand::InsertMonoNode { target, index, node } => {
            if target == CommandTarget::Channel1 || target == CommandTarget::Channel2 {
                let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
                if let Err(uninserted) = cs.rack.try_insert(index, node) {
                    // Rack at preallocated capacity! Forward uninserted Box to garbage queue;
                    // NEVER reallocate vector and NEVER drop on RT thread!
                    if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::MonoNode(uninserted)) {
                        std::mem::forget(g);
                    }
                }
            } else {
                if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::MonoNode(node)) {
                    std::mem::forget(g);
                }
            }
        }
        AudioCommand::InsertStereoNode { node, .. } => {
            if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::StereoNode(node)) {
                std::mem::forget(g);
            }
        }
        AudioCommand::RemoveNode { target, index } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            if let Some(removed) = cs.rack.remove(index) {
                // Route to garbage queue; NEVER drop on RT thread!
                if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::MonoNode(removed)) {
                    std::mem::forget(g); // Safety guarantee against RT drop if queue full
                }
            }
        }
        AudioCommand::SwapMonoRack { target, mut rack } => {
            if target == CommandTarget::Channel1 || target == CommandTarget::Channel2 {
                let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
                std::mem::swap(&mut cs.rack, &mut rack);
                if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::MonoRack(rack)) {
                    std::mem::forget(g);
                }
            } else {
                if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::MonoRack(rack)) {
                    std::mem::forget(g);
                }
            }
        }
        AudioCommand::SwapStereoRack { rack, .. } => {
            if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::StereoRack(rack)) {
                std::mem::forget(g);
            }
        }
        AudioCommand::SetParam { target, param_id, value } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            match param_id {
                "comp_threshold" => {
                    if let Some(comp) = cs.compressor_mut() {
                        comp.threshold_db = value;
                    }
                }
                "comp_ratio" => {
                    if let Some(comp) = cs.compressor_mut() {
                        comp.ratio = value;
                    }
                }
                "sat_drive" => {
                    if let Some(sat) = cs.saturation_mut() {
                        sat.set_drive(value);
                    }
                }
                "gate_threshold" => {
                    if let Some(gate) = cs.gate_mut() {
                        gate.threshold_db = value.clamp(-80.0, 0.0);
                    }
                }
                "hpf_freq" => {
                    if let Some(hpf) = cs.hpf_mut() {
                        hpf.set_cutoff(value.clamp(20.0, 300.0));
                    }
                }
                "eq_low_gain" => {
                    if let Some(eq) = cs.eq_mut() {
                        eq.low_shelf.set_gain_db(value.clamp(-12.0, 12.0));
                    }
                }
                "eq_lmid_gain" => {
                    if let Some(eq) = cs.eq_mut() {
                        eq.low_mid.set_gain_db(value.clamp(-12.0, 12.0));
                    }
                }
                "eq_hmid_gain" => {
                    if let Some(eq) = cs.eq_mut() {
                        eq.high_mid.set_gain_db(value.clamp(-12.0, 12.0));
                    }
                }
                "eq_hi_gain" => {
                    if let Some(eq) = cs.eq_mut() {
                        eq.high_shelf.set_gain_db(value.clamp(-12.0, 12.0));
                    }
                }
                "deess_amount" => {
                    if let Some(deess) = cs.deesser_mut() {
                        deess.set_amount(value.clamp(0.0, 12.0));
                    }
                }
                "comp_attack" => {
                    if let Some(comp) = cs.compressor_mut() {
                        comp.set_attack_ms(value.clamp(0.1, 100.0));
                    }
                }
                "comp_release" => {
                    if let Some(comp) = cs.compressor_mut() {
                        comp.set_release_ms(value.clamp(10.0, 1000.0));
                    }
                }
                "tuner_retune" => {
                    if let Some(tuner) = cs.tuner_mut() {
                        tuner.retune_speed_ms = value.clamp(0.1, 200.0);
                    }
                }
                _ => {}
            }
        }
        AudioCommand::SetTunerScale { target, scale } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            if let Some(tuner) = cs.tuner_mut() {
                tuner.quantizer.scale = scale;
            }
        }
        AudioCommand::ResetAll { target } => {
            let cs = if target == CommandTarget::Channel1 { cs1 } else { cs2 };
            cs.reset_all();
        }
        _ => {}
    }
}

/// Applies output command on the audio thread with ZERO allocations and sends retired boxes
/// to the garbage return queue (NEVER dropped on the audio thread).
pub fn apply_output_command(
    cmd: AudioCommand,
    garbage_producer: &mut rtrb::Producer<AudioGarbage>,
    master: &mut MasterChain,
) {
    match cmd {
        AudioCommand::SetBypass { target: CommandTarget::Master, node_index, bypassed } => {
            master.rack.set_bypassed(node_index, bypassed);
        }
        AudioCommand::SetNodeBypass { target: CommandTarget::Master, node_id, bypassed } => {
            match node_id {
                "master_eq" | "eq" => { if let Some(n) = master.eq_mut() { n.set_bypassed(bypassed); } }
                "multiband" => { if let Some(n) = master.multiband_mut() { n.set_bypassed(bypassed); } }
                "stereo_width" | "width" => { if let Some(n) = master.stereo_width_mut() { n.set_bypassed(bypassed); } }
                "glue" => { if let Some(n) = master.glue_mut() { n.set_bypassed(bypassed); } }
                "limiter" => { if let Some(n) = master.limiter_mut() { n.set_bypassed(bypassed); } }
                _ => {}
            }
        }
        AudioCommand::ToggleBypass { target: CommandTarget::Master, node_index } => {
            master.rack.toggle_bypass(node_index);
        }
        AudioCommand::MoveNode { target: CommandTarget::Master, from, to } => {
            master.rack.move_node(from, to);
        }
        AudioCommand::SwapNodes { target: CommandTarget::Master, i, j } => {
            master.rack.swap(i, j);
        }
        AudioCommand::InsertStereoNode { target, index, node } => {
            if target == CommandTarget::Master {
                if let Err(uninserted) = master.rack.try_insert(index, node) {
                    // Rack at preallocated capacity! Forward uninserted Box to garbage queue;
                    // NEVER reallocate vector and NEVER drop on RT thread!
                    if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::StereoNode(uninserted)) {
                        std::mem::forget(g);
                    }
                }
            } else {
                if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::StereoNode(node)) {
                    std::mem::forget(g);
                }
            }
        }
        AudioCommand::InsertMonoNode { node, .. } => {
            if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::MonoNode(node)) {
                std::mem::forget(g);
            }
        }
        AudioCommand::RemoveNode { target: CommandTarget::Master, index } => {
            if let Some(removed) = master.rack.remove(index) {
                if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::StereoNode(removed)) {
                    std::mem::forget(g); // Never drop on RT thread!
                }
            }
        }
        AudioCommand::SwapStereoRack { target, mut rack } => {
            if target == CommandTarget::Master {
                std::mem::swap(&mut master.rack, &mut rack);
                if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::StereoRack(rack)) {
                    std::mem::forget(g); // Never drop on RT thread!
                }
            } else {
                if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::StereoRack(rack)) {
                    std::mem::forget(g);
                }
            }
        }
        AudioCommand::SwapMonoRack { rack, .. } => {
            if let Err(rtrb::PushError::Full(g)) = garbage_producer.push(AudioGarbage::MonoRack(rack)) {
                std::mem::forget(g);
            }
        }
        AudioCommand::SetParam { target: CommandTarget::Master, param_id, value } => {
            match param_id {
                "limiter_ceiling" => {
                    if let Some(limiter) = master.limiter_mut() {
                        limiter.set_ceiling_dbtp(value);
                    }
                }
                "glue_threshold" => {
                    if let Some(glue) = master.glue_mut() {
                        glue.threshold_db = value;
                    }
                }
                "stereo_width" => {
                    if let Some(w) = master.stereo_width_mut() {
                        w.width = value.clamp(0.0, 2.0);
                    }
                }
                "master_eq_low" => {
                    if let Some(eq) = master.eq_mut() {
                        let g = value.clamp(-12.0, 12.0);
                        eq.low_shelf.0.set_gain_db(g);
                        eq.low_shelf.1.set_gain_db(g);
                    }
                }
                "master_eq_mid" => {
                    if let Some(eq) = master.eq_mut() {
                        let g = value.clamp(-12.0, 12.0);
                        eq.low_mid.0.set_gain_db(g);
                        eq.low_mid.1.set_gain_db(g);
                        eq.high_mid.0.set_gain_db(g);
                        eq.high_mid.1.set_gain_db(g);
                    }
                }
                "master_eq_high" => {
                    if let Some(eq) = master.eq_mut() {
                        let g = value.clamp(-12.0, 12.0);
                        eq.high_shelf.0.set_gain_db(g);
                        eq.high_shelf.1.set_gain_db(g);
                    }
                }
                _ => {}
            }
        }
        AudioCommand::ResetAll { target: CommandTarget::Master } => {
            master.reset_all();
        }
        _ => {}
    }
}
