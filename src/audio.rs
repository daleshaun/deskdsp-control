//! Lock-Free Real-Time Audio Engine using cpal and rtrb SPSC queues.
//!
//! Real-Time Safety Guarantees:
//! 1. Zero mutex locks on the audio thread: audio callbacks OWN their DSP state.
//! 2. Zero memory allocations in `process()` / callbacks.
//! 3. Zero deallocations on the audio thread: removed nodes and swapped racks
//!    are transferred back to the UI/cleanup thread via an SPSC garbage queue.
//! 4. Lock-free SPSC audio ring buffer (`rtrb`) between input and output streams.
//! 5. Lock-free atomic meters read/write without contention.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream};

use crate::dsp::channel_strip::ChannelStrip;
use crate::dsp::master_chain::MasterChain;
use crate::dsp::rack::{MonoRack, StereoRack};
use crate::dsp::tuner::Scale;
use crate::dsp::{DspNode, StereoDspNode};

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
    ToggleBypass { target: CommandTarget, node_index: usize },
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
    pub input_cmd_producer: rtrb::Producer<AudioCommand>,
    pub output_cmd_producer: rtrb::Producer<AudioCommand>,
    pub input_garbage_consumer: rtrb::Consumer<AudioGarbage>,
    pub output_garbage_consumer: rtrb::Consumer<AudioGarbage>,
    pub meters: Arc<AudioMeters>,
    _input_stream: Option<Stream>,
    _output_stream: Option<Stream>,
    pub sample_rate: u32,
}

impl AudioEngine {
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        
        // Locate Zen Go devices
        let mut in_dev: Option<Device> = None;
        let mut out_dev: Option<Device> = None;

        if let Ok(devices) = host.input_devices() {
            for d in devices {
                if let Ok(name) = d.name() {
                    if name.contains("Zen Go") {
                        in_dev = Some(d);
                        break;
                    }
                }
            }
        }

        if let Ok(devices) = host.output_devices() {
            for d in devices {
                if let Ok(name) = d.name() {
                    if name.contains("Zen Go") {
                        out_dev = Some(d);
                        break;
                    }
                }
            }
        }

        // Fallback to system defaults if specific named device not found
        let in_dev = in_dev.or_else(|| host.default_input_device()).ok_or_else(|| anyhow!("No audio input device available"))?;
        let out_dev = out_dev.or_else(|| host.default_output_device()).ok_or_else(|| anyhow!("No audio output device available"))?;

        let in_config = in_dev.default_input_config()?;
        let out_config = out_dev.default_output_config()?;

        let sample_rate = in_config.sample_rate().0;
        let meters = Arc::new(AudioMeters::default());

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

                    let mut max_l = 0.0_f32;
                    let mut max_r = 0.0_f32;
                    let num_frames = data.len() / in_channels;

                    for f in 0..num_frames {
                        let raw_l = data[f * in_channels];
                        let raw_r = if in_channels > 1 { data[f * in_channels + 1] } else { raw_l };

                        max_l = max_l.max(raw_l.abs());
                        max_r = max_r.max(raw_r.abs());

                        // Process through owned channel strips
                        let proc_l = cs1.process(raw_l);
                        let proc_r = cs2.process(raw_r);

                        // Push to lock-free SPSC ring buffers
                        let _ = ring_l_prod.push(proc_l);
                        let _ = ring_r_prod.push(proc_r);
                    }

                    // Update Channel Strip meters from CS1
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
        let mut master = MasterChain::new(sample_rate as f32);

        let out_stream = match out_config.sample_format() {
            SampleFormat::F32 => out_dev.build_output_stream(
                &out_config.into(),
                move |data: &mut [f32], _| {
                    // Top of callback: drain command queue and apply to owned state
                    while let Ok(cmd) = out_cmd_consumer.pop() {
                        apply_output_command(cmd, &mut out_garbage_producer, &mut master);
                    }

                    let num_frames = data.len() / out_channels;
                    let mut max_l = 0.0_f32;
                    let mut max_r = 0.0_f32;

                    for f in 0..num_frames {
                        let in_l = ring_l_cons.pop().unwrap_or(0.0);
                        let in_r = ring_r_cons.pop().unwrap_or(0.0);

                        // Process stereo through owned Master Chain!
                        let (out_l, out_r) = master.process_stereo(in_l, in_r);

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
                    if let Some(limiter) = master.limiter() {
                        AudioMeters::store_f32(&out_meters.master_limiter_gr_db, limiter.gain_reduction_db());
                    }
                    if let Some(meter) = master.meter() {
                        AudioMeters::store_f32(&out_meters.master_true_peak_dbtp, meter.max_true_peak_dbtp);
                        AudioMeters::store_f32(&out_meters.momentary_lufs, meter.momentary_lufs);
                        AudioMeters::store_f32(&out_meters.short_term_lufs, meter.short_term_lufs);
                        AudioMeters::store_f32(&out_meters.integrated_lufs, meter.integrated_lufs);
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
            input_cmd_producer,
            output_cmd_producer,
            input_garbage_consumer,
            output_garbage_consumer,
            meters,
            _input_stream: Some(in_stream),
            _output_stream: Some(out_stream),
            sample_rate,
        })
    }

    /// Push a command to the audio thread via lock-free SPSC queue.
    pub fn send_command(&mut self, cmd: AudioCommand) -> Result<(), AudioCommand> {
        match cmd {
            AudioCommand::SetInputGain { target: CommandTarget::Master, .. }
            | AudioCommand::SetOutputGain { target: CommandTarget::Master, .. }
            | AudioCommand::SetBypass { target: CommandTarget::Master, .. }
            | AudioCommand::ToggleBypass { target: CommandTarget::Master, .. }
            | AudioCommand::MoveNode { target: CommandTarget::Master, .. }
            | AudioCommand::SwapNodes { target: CommandTarget::Master, .. }
            | AudioCommand::InsertStereoNode { target: CommandTarget::Master, .. }
            | AudioCommand::RemoveNode { target: CommandTarget::Master, .. }
            | AudioCommand::SwapStereoRack { target: CommandTarget::Master, .. }
            | AudioCommand::SetParam { target: CommandTarget::Master, .. }
            | AudioCommand::ResetAll { target: CommandTarget::Master, .. } => {
                self.output_cmd_producer.push(cmd).map_err(|e| match e { rtrb::PushError::Full(c) => c })
            }
            _ => {
                self.input_cmd_producer.push(cmd).map_err(|e| match e { rtrb::PushError::Full(c) => c })
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
                        gate.threshold_db = value;
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
                        w.width = value;
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
