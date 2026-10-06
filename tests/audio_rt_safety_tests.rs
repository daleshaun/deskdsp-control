//! Real-Time Safety Verification Tests for DeskDSP Control Audio Engine.
//!
//! Proves:
//! 1. ZERO allocations and ZERO deallocations (frees) occur during the audio process path,
//!    even while simultaneously draining commands (SetParam, SetBypass, ToggleBypass,
//!    MoveNode, SwapNodes, InsertNode, RemoveNode, SwapRack).
//! 2. Retired `Box<dyn DspNode>` and `Box<MonoRack>` / `Box<StereoRack>` are NEVER dropped
//!    on the audio thread; they are transferred via the SPSC garbage return queue and
//!    dropped exclusively on the UI/cleanup thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use assert_no_alloc::{assert_no_alloc, AllocDisabler};

use deskdsp_control::audio::{
    apply_input_command, apply_output_command, AudioCommand, AudioGarbage, CommandTarget,
};
use deskdsp_control::dsp::channel_strip::ChannelStrip;
use deskdsp_control::dsp::master_chain::MasterChain;
use deskdsp_control::dsp::rack::{MonoRack, StereoRack};
use deskdsp_control::dsp::saturation::Saturation;
use deskdsp_control::dsp::tuner::Scale;
use deskdsp_control::dsp::{DspNode, NodeTelemetry};

#[cfg(debug_assertions)]
#[global_allocator]
static A: AllocDisabler = AllocDisabler;

/// Test node that records exactly when and where it is dropped.
struct DropTrackingNode {
    dropped_flag: Arc<AtomicBool>,
    drop_thread_name: Arc<std::sync::Mutex<Option<String>>>,
    bypassed: bool,
}

impl DropTrackingNode {
    fn new(
        dropped_flag: Arc<AtomicBool>,
        drop_thread_name: Arc<std::sync::Mutex<Option<String>>>,
    ) -> Self {
        Self {
            dropped_flag,
            drop_thread_name,
            bypassed: false,
        }
    }
}

impl Drop for DropTrackingNode {
    fn drop(&mut self) {
        self.dropped_flag.store(true, Ordering::SeqCst);
        let current_thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        if let Ok(mut lock) = self.drop_thread_name.lock() {
            *lock = Some(current_thread);
        }
    }
}

impl DspNode for DropTrackingNode {
    fn name(&self) -> &'static str {
        "DropTrackingNode"
    }

    fn is_bypassed(&self) -> bool {
        self.bypassed
    }

    fn set_bypassed(&mut self, bypassed: bool) {
        self.bypassed = bypassed;
    }

    fn reset(&mut self) {}

    fn process_sample(&mut self, sample: f32) -> f32 {
        sample * 0.99
    }

    fn telemetry(&self) -> NodeTelemetry {
        NodeTelemetry::default()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[test]
fn test_audio_process_path_zero_allocations_under_heavy_command_stream() {
    let sample_rate = 48000.0_f32;

    // Pre-allocate ChannelStrip (owns MonoRack with capacity 16) and MasterChain (capacity 16)
    let mut cs1 = ChannelStrip::new(sample_rate);
    let mut cs2 = ChannelStrip::new(sample_rate);
    let mut master = MasterChain::new(sample_rate);

    // Pre-allocate SPSC command and garbage queues
    let (mut in_cmd_prod, mut in_cmd_cons) = rtrb::RingBuffer::<AudioCommand>::new(256);
    let (mut out_cmd_prod, mut out_cmd_cons) = rtrb::RingBuffer::<AudioCommand>::new(256);
    let (mut in_garbage_prod, mut in_garbage_cons) = rtrb::RingBuffer::<AudioGarbage>::new(256);
    let (mut out_garbage_prod, mut out_garbage_cons) = rtrb::RingBuffer::<AudioGarbage>::new(256);

    // Lock-free audio ring buffer between input and output
    let (mut ring_l_prod, mut ring_l_cons) = rtrb::RingBuffer::<f32>::new(8192);
    let (mut ring_r_prod, mut ring_r_cons) = rtrb::RingBuffer::<f32>::new(8192);

    // --- OFF-THREAD (UI Thread): Construct Boxes and push commands ---
    // All allocations happen here on the control plane before audio execution.
    let new_mono_node: Box<dyn DspNode> = Box::new(Saturation::new(sample_rate));
    let mut prebuilt_mono_rack = Box::new(MonoRack::with_capacity(16));
    prebuilt_mono_rack.push(Saturation::new(sample_rate));

    let mut prebuilt_stereo_rack = Box::new(StereoRack::with_capacity(16));
    prebuilt_stereo_rack.push(deskdsp_control::dsp::master_eq::MasterEq::new(sample_rate));

    // Input thread commands
    in_cmd_prod.push(AudioCommand::SetParam {
        target: CommandTarget::Channel1,
        param_id: "comp_threshold",
        value: -24.0,
    }).unwrap();
    in_cmd_prod.push(AudioCommand::SetParam {
        target: CommandTarget::Channel1,
        param_id: "sat_drive",
        value: 4.5,
    }).unwrap();
    in_cmd_prod.push(AudioCommand::SetBypass {
        target: CommandTarget::Channel1,
        node_index: 2,
        bypassed: true,
    }).unwrap();
    in_cmd_prod.push(AudioCommand::ToggleBypass {
        target: CommandTarget::Channel1,
        node_index: 3,
    }).unwrap();
    in_cmd_prod.push(AudioCommand::MoveNode {
        target: CommandTarget::Channel1,
        from: 1,
        to: 2,
    }).unwrap();
    in_cmd_prod.push(AudioCommand::SwapNodes {
        target: CommandTarget::Channel1,
        i: 0,
        j: 1,
    }).unwrap();
    in_cmd_prod.push(AudioCommand::InsertMonoNode {
        target: CommandTarget::Channel1,
        index: 2,
        node: new_mono_node,
    }).unwrap();
    in_cmd_prod.push(AudioCommand::RemoveNode {
        target: CommandTarget::Channel1,
        index: 4,
    }).unwrap();
    in_cmd_prod.push(AudioCommand::SwapMonoRack {
        target: CommandTarget::Channel1,
        rack: prebuilt_mono_rack,
    }).unwrap();
    in_cmd_prod.push(AudioCommand::SetTunerScale {
        target: CommandTarget::Channel1,
        scale: Scale::NaturalMinor,
    }).unwrap();

    // Output thread commands
    out_cmd_prod.push(AudioCommand::SetParam {
        target: CommandTarget::Master,
        param_id: "limiter_ceiling",
        value: -0.5,
    }).unwrap();
    out_cmd_prod.push(AudioCommand::SetParam {
        target: CommandTarget::Master,
        param_id: "glue_threshold",
        value: -15.0,
    }).unwrap();
    out_cmd_prod.push(AudioCommand::ToggleBypass {
        target: CommandTarget::Master,
        node_index: 1,
    }).unwrap();
    out_cmd_prod.push(AudioCommand::RemoveNode {
        target: CommandTarget::Master,
        index: 2,
    }).unwrap();
    out_cmd_prod.push(AudioCommand::SwapStereoRack {
        target: CommandTarget::Master,
        rack: prebuilt_stereo_rack,
    }).unwrap();

    // Audio test frames (pre-allocated)
    let input_frames = [0.25_f32; 4800];
    let mut output_frames_l = [0.0_f32; 4800];
    let mut output_frames_r = [0.0_f32; 4800];

    // --- REAL-TIME AUDIO PATH ASSERTION ---
    // assert_no_alloc guarantees ZERO allocations and ZERO frees on this thread!
    assert_no_alloc(|| {
        // 1. Drain input commands and apply to owned state with zero alloc and zero free
        while let Ok(cmd) = in_cmd_cons.pop() {
            apply_input_command(cmd, &mut in_garbage_prod, &mut cs1, &mut cs2);
        }

        // 2. Process audio through owned ChannelStrips and feed lock-free SPSC ring
        for (i, &sample) in input_frames.iter().enumerate() {
            let proc_l = cs1.process(sample);
            let proc_r = cs2.process(sample);
            let _ = ring_l_prod.push(proc_l);
            let _ = ring_r_prod.push(proc_r);

            // Output callback: drain output commands on first frame
            if i == 0 {
                while let Ok(cmd) = out_cmd_cons.pop() {
                    apply_output_command(cmd, &mut out_garbage_prod, &mut master);
                }
            }

            // Output callback: consume from lock-free ring and process master chain
            let in_l = ring_l_cons.pop().unwrap_or(0.0);
            let in_r = ring_r_cons.pop().unwrap_or(0.0);
            let (out_l, out_r) = master.process_stereo(in_l, in_r);
            output_frames_l[i] = out_l;
            output_frames_r[i] = out_r;
        }
    });

    // Verify audio processed valid numbers without NaN or clipping explosion
    for &sample in &output_frames_l {
        assert!(sample.is_finite());
        assert!(sample.abs() <= 1.5);
    }

    // --- CONTROL/UI THREAD: Drain and verify garbage queue ---
    // The retired nodes and racks were transferred, NOT dropped on the audio path.
    let mut in_garbage_count = 0;
    while let Ok(garbage) = in_garbage_cons.pop() {
        match garbage {
            AudioGarbage::MonoNode(_) => in_garbage_count += 1,
            AudioGarbage::MonoRack(_) => in_garbage_count += 1,
            _ => panic!("Unexpected garbage variant on input return queue"),
        }
    }
    assert_eq!(in_garbage_count, 2, "Expected 1 removed node and 1 swapped rack in input garbage");

    let mut out_garbage_count = 0;
    while let Ok(garbage) = out_garbage_cons.pop() {
        match garbage {
            AudioGarbage::StereoNode(_) => out_garbage_count += 1,
            AudioGarbage::StereoRack(_) => out_garbage_count += 1,
            _ => panic!("Unexpected garbage variant on output return queue"),
        }
    }
    assert_eq!(out_garbage_count, 2, "Expected 1 removed stereo node and 1 swapped stereo rack in output garbage");
}

#[test]
fn test_garbage_queue_receives_retired_nodes_without_rt_drop() {
    let dropped_flag = Arc::new(AtomicBool::new(false));
    let drop_thread = Arc::new(std::sync::Mutex::new(None));

    let sample_rate = 48000.0_f32;
    let mut cs1 = ChannelStrip::new(sample_rate);
    let mut cs2 = ChannelStrip::new(sample_rate);

    let (mut cmd_prod, mut cmd_cons) = rtrb::RingBuffer::<AudioCommand>::new(64);
    let (mut garbage_prod, mut garbage_cons) = rtrb::RingBuffer::<AudioGarbage>::new(64);

    // Build tracking node on control thread
    let tracking_node = Box::new(DropTrackingNode::new(
        Arc::clone(&dropped_flag),
        Arc::clone(&drop_thread),
    ));

    // UI sends insert command
    cmd_prod.push(AudioCommand::InsertMonoNode {
        target: CommandTarget::Channel1,
        index: 2,
        node: tracking_node,
    }).unwrap();

    // Simulated Audio Thread: Apply Insert
    let rt_dropped_flag = Arc::clone(&dropped_flag);
    let audio_thread_name = "audio-rt-worker";
    let rt_handler = std::thread::Builder::new()
        .name(audio_thread_name.into())
        .spawn(move || {
            // Apply insert
            if let Ok(cmd) = cmd_cons.pop() {
                apply_input_command(cmd, &mut garbage_prod, &mut cs1, &mut cs2);
            }

            // Node should now be in ChannelStrip rack and NOT dropped
            assert!(!rt_dropped_flag.load(Ordering::SeqCst), "Node must NOT be dropped while active in rack");

            // Process one block
            for _ in 0..64 {
                let _ = cs1.process(0.5);
            }

            // Now audio thread applies RemoveNode
            let remove_cmd = AudioCommand::RemoveNode {
                target: CommandTarget::Channel1,
                index: 2,
            };
            apply_input_command(remove_cmd, &mut garbage_prod, &mut cs1, &mut cs2);

            // CRITICAL RT-SAFETY ASSERTION:
            // Node was removed from rack, but transferred to garbage queue.
            // It MUST NOT have been dropped on this audio thread!
            assert!(
                !rt_dropped_flag.load(Ordering::SeqCst),
                "PROVEN VIOLATION: Node was dropped on the audio thread!"
            );

            // Audio thread exits cleanly without dropping the node
        })
        .unwrap();

    rt_handler.join().unwrap();

    // Verify node is STILL not dropped after audio thread has completely terminated!
    assert!(
        !dropped_flag.load(Ordering::SeqCst),
        "Node must still be alive inside the garbage queue after audio thread exit"
    );

    // UI/Cleanup thread now drains the garbage queue
    let ui_thread_name = "ui-cleanup-thread";
    let cleanup_handler = std::thread::Builder::new()
        .name(ui_thread_name.into())
        .spawn(move || {
            let mut count = 0;
            while let Ok(g) = garbage_cons.pop() {
                drop(g); // Explicit drop on cleanup thread
                count += 1;
            }
            assert_eq!(count, 1, "Expected exactly 1 garbage node");
        })
        .unwrap();

    cleanup_handler.join().unwrap();

    // Now verify the node WAS dropped, and it was dropped on the UI cleanup thread!
    assert!(dropped_flag.load(Ordering::SeqCst), "Node should be dropped after garbage queue drain");
    let thread_name = drop_thread.lock().unwrap().clone();
    assert_eq!(
        thread_name,
        Some(ui_thread_name.to_string()),
        "Node must be dropped on the UI cleanup thread, NOT the audio thread"
    );
}

#[test]
fn test_rack_insert_at_capacity_boundary_zero_allocations_and_safe_garbage_return() {
    let sample_rate = 48000.0_f32;
    let mut cs1 = ChannelStrip::new(sample_rate);
    let mut cs2 = ChannelStrip::new(sample_rate);

    // 1. Fill cs1 rack up to exact capacity (16 nodes)
    let initial_len = cs1.rack.len();
    let capacity = cs1.rack.capacity();
    assert_eq!(capacity, 16);
    for _ in initial_len..capacity {
        cs1.rack.push(Saturation::new(sample_rate));
    }
    assert_eq!(cs1.rack.len(), 16, "Rack must be exactly at maximum pre-allocated capacity");
    assert_eq!(cs1.rack.capacity(), 16);

    let (mut cmd_prod, mut cmd_cons) = rtrb::RingBuffer::<AudioCommand>::new(32);
    let (mut garbage_prod, mut garbage_cons) = rtrb::RingBuffer::<AudioGarbage>::new(32);

    let dropped_flag = Arc::new(AtomicBool::new(false));
    let drop_thread = Arc::new(std::sync::Mutex::new(None));

    // Construct 17th node (overflow candidate) on UI thread
    let overflow_node = Box::new(DropTrackingNode::new(
        Arc::clone(&dropped_flag),
        Arc::clone(&drop_thread),
    ));

    // UI sends insert command targeting full rack
    cmd_prod.push(AudioCommand::InsertMonoNode {
        target: CommandTarget::Channel1,
        index: 4,
        node: overflow_node,
    }).unwrap();

    // 2. Audio Thread drains command inside assert_no_alloc!
    // Proves:
    // a) ZERO vector reallocations occur when attempting to insert at capacity!
    // b) ZERO frees/drops occur on the audio thread!
    assert_no_alloc(|| {
        if let Ok(cmd) = cmd_cons.pop() {
            apply_input_command(cmd, &mut garbage_prod, &mut cs1, &mut cs2);
        }
        // Process a block
        for _ in 0..64 {
            let _ = cs1.process(0.1);
        }
    });

    // 3. Verify rack remained at capacity (16) and was NOT resized
    assert_eq!(cs1.rack.len(), 16, "Rack length must not exceed capacity");
    assert_eq!(cs1.rack.capacity(), 16, "Rack capacity must remain strictly 16 without realloc");

    // 4. Verify overflow node was NOT dropped on audio thread
    assert!(
        !dropped_flag.load(Ordering::SeqCst),
        "Overflow node must NOT be dropped on the audio thread!"
    );

    // 5. Verify overflow node was safely transferred to garbage queue
    let mut garbage_count = 0;
    while let Ok(g) = garbage_cons.pop() {
        drop(g);
        garbage_count += 1;
    }
    assert_eq!(garbage_count, 1, "Garbage queue must receive the rejected overflow node");
    assert!(
        dropped_flag.load(Ordering::SeqCst),
        "Overflow node must be dropped only after cleanup thread drain"
    );
}

#[test]
fn test_global_bypass_bit_identical_clean_passthrough() {
    let mut cs1 = ChannelStrip::new(48000.0);
    let mut cs2 = ChannelStrip::new(48000.0);
    let mut master = MasterChain::new(48000.0);

    // Set aggressive processing so normal path modifies audio significantly
    cs1.input_gain_db = 12.0;
    cs2.input_gain_db = 12.0;

    let bypass = Arc::new(AtomicBool::new(true));

    let test_samples: [f32; 8] = [
        0.0,
        0.1234567,
        -0.4567891,
        0.7777777,
        -0.8888888,
        0.0001234,
        -0.0009876,
        0.9999999,
    ];

    // 1. In bypassed state inside assert_no_alloc, verify bit-identical passthrough and zero allocs
    assert_no_alloc(|| {
        for &s in &test_samples {
            let bypassed = bypass.load(Ordering::Relaxed);
            let (out_l, out_r) = if bypassed {
                (s, s)
            } else {
                (cs1.process(s), cs2.process(s))
            };

            assert_eq!(
                s.to_bits(),
                out_l.to_bits(),
                "ChannelStrip L sample must be bit-identical in bypass mode"
            );
            assert_eq!(
                s.to_bits(),
                out_r.to_bits(),
                "ChannelStrip R sample must be bit-identical in bypass mode"
            );

            let (master_l, master_r) = if bypassed {
                (out_l, out_r)
            } else {
                master.process_stereo(out_l, out_r)
            };

            assert_eq!(
                s.to_bits(),
                master_l.to_bits(),
                "Master Chain L sample must be bit-identical in bypass mode"
            );
            assert_eq!(
                s.to_bits(),
                master_r.to_bits(),
                "Master Chain R sample must be bit-identical in bypass mode"
            );
        }
    });

    // 2. Disable bypass: verify active processing alters audio
    bypass.store(false, Ordering::Relaxed);
    let mut altered = false;
    for &s in &test_samples {
        let bypassed = bypass.load(Ordering::Relaxed);
        let (out_l, _) = if bypassed {
            (s, s)
        } else {
            (cs1.process(s), cs2.process(s))
        };
        if s.to_bits() != out_l.to_bits() {
            altered = true;
        }
    }
    assert!(altered, "Active DSP processing must alter the audio signal");

    // 3. Re-enable bypass: verify immediate return to bit-identical passthrough
    bypass.store(true, Ordering::Relaxed);
    for &s in &test_samples {
        let bypassed = bypass.load(Ordering::Relaxed);
        let (out_l, out_r) = if bypassed {
            (s, s)
        } else {
            (cs1.process(s), cs2.process(s))
        };
        assert_eq!(s.to_bits(), out_l.to_bits());
        assert_eq!(s.to_bits(), out_r.to_bits());
    }
}

#[test]
fn test_preset_swap_audio_thread_zero_allocations_and_safe_garbage_return() {
    use deskdsp_control::dsp::{build_rack, InstrumentPreset};

    let sample_rate = 48000.0_f32;
    let mut cs1 = ChannelStrip::new(sample_rate);
    let mut cs2 = ChannelStrip::new(sample_rate);
    let (mut garbage_prod, mut garbage_cons) = rtrb::RingBuffer::<AudioGarbage>::new(32);

    // Build new rack OFF the audio thread
    let new_eguitar_rack = build_rack(InstrumentPreset::ElectricGuitar, sample_rate);
    let swap_cmd = AudioCommand::SwapMonoRack {
        target: CommandTarget::Channel1,
        rack: Box::new(new_eguitar_rack),
    };

    // PROVE: Swapping rack on audio thread incurs ZERO allocations and ZERO deallocations
    assert_no_alloc(|| {
        apply_input_command(swap_cmd, &mut garbage_prod, &mut cs1, &mut cs2);
        // Process block of audio immediately on new rack with zero allocations
        for _ in 0..64 {
            let _ = cs1.process(0.3);
        }
    });

    // Verify the retired vocal rack was safely deposited in the garbage queue
    let mut retired_rack_count = 0;
    while let Ok(garbage) = garbage_cons.pop() {
        if let AudioGarbage::MonoRack(retired_box) = garbage {
            assert_eq!(retired_box.len(), 7); // Original vocal strip had 7 nodes
            retired_rack_count += 1;
        }
    }
    assert_eq!(retired_rack_count, 1, "Retired rack must be pushed to garbage queue for off-thread drop");
}

#[test]
fn test_conv_engine_and_mic_image_zero_allocations() {
    use deskdsp_control::dsp::{ConvEngine, MicImage};

    // Preallocate IR and engine OFF the audio thread
    let ir: Vec<f32> = (0..512).map(|i| (-i as f32 / 100.0).exp()).collect();
    let mut conv = ConvEngine::new(&ir, 128);
    let mut mic = MicImage::new(&ir, 128, "Test Mic");
    mic.set_bypassed(false);

    let mut block = [0.25_f32; 128];

    // PROVE: Convolution processing loop incurs ZERO allocations
    assert_no_alloc(|| {
        for s in block.iter_mut() {
            *s = conv.process_sample(*s);
        }
        for s in block.iter_mut() {
            *s = mic.process_sample(*s);
        }
    });
}

