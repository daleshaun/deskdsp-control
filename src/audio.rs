//! Audio Engine using cpal for CoreAudio / ALSA / PipeWire / JACK streaming.
//! Connects Zen Go input channels to ChannelStrip and output to MasterChain.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{Device, SampleFormat, Stream};

use crate::dsp::channel_strip::ChannelStrip;
use crate::dsp::master_chain::MasterChain;

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
    pub channel_strip_1: Arc<Mutex<ChannelStrip>>,
    pub channel_strip_2: Arc<Mutex<ChannelStrip>>,
    pub master_chain: Arc<Mutex<MasterChain>>,
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
        let channel_strip_1 = Arc::new(Mutex::new(ChannelStrip::new(sample_rate as f32)));
        let channel_strip_2 = Arc::new(Mutex::new(ChannelStrip::new(sample_rate as f32)));
        let master_chain = Arc::new(Mutex::new(MasterChain::new(sample_rate as f32)));
        let meters = Arc::new(AudioMeters::default());

        // Lock-free ring buffer between input callback and output callback
        let ring_buffer_size = 4096;
        let ring_buffer_l = Arc::new(Mutex::new(vec![0.0_f32; ring_buffer_size]));
        let ring_buffer_r = Arc::new(Mutex::new(vec![0.0_f32; ring_buffer_size]));
        let write_idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let read_idx = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        // Setup input stream
        let in_channels = in_config.channels() as usize;
        let in_rb_l = Arc::clone(&ring_buffer_l);
        let in_rb_r = Arc::clone(&ring_buffer_r);
        let in_w_idx = Arc::clone(&write_idx);
        let in_meters = Arc::clone(&meters);
        let in_cs1 = Arc::clone(&channel_strip_1);
        let in_cs2 = Arc::clone(&channel_strip_2);

        let in_stream = match in_config.sample_format() {
            SampleFormat::F32 => in_dev.build_input_stream(
                &in_config.into(),
                move |data: &[f32], _| {
                    let mut max_l = 0.0_f32;
                    let mut max_r = 0.0_f32;
                    let num_frames = data.len() / in_channels;

                    if let (Ok(mut rb_l), Ok(mut rb_r), Ok(mut cs1), Ok(mut cs2)) = 
                        (in_rb_l.lock(), in_rb_r.lock(), in_cs1.lock(), in_cs2.lock()) 
                    {
                        let mut w = in_w_idx.load(Ordering::Relaxed);
                        for f in 0..num_frames {
                            let raw_l = data[f * in_channels];
                            let raw_r = if in_channels > 1 { data[f * in_channels + 1] } else { raw_l };

                            max_l = max_l.max(raw_l.abs());
                            max_r = max_r.max(raw_r.abs());

                            // Process inputs through Channel Strips 1 and 2
                            let proc_l = cs1.process(raw_l);
                            let proc_r = cs2.process(raw_r);

                            rb_l[w % ring_buffer_size] = proc_l;
                            rb_r[w % ring_buffer_size] = proc_r;
                            w = (w + 1) % (ring_buffer_size * 2);
                        }
                        in_w_idx.store(w, Ordering::Relaxed);

                        // Update Channel Strip meters from CS1
                        AudioMeters::store_f32(&in_meters.gate_reduction_db, cs1.gate.current_reduction_db());
                        AudioMeters::store_f32(&in_meters.deesser_reduction_db, cs1.deesser.gain_reduction_db());
                        AudioMeters::store_f32(&in_meters.comp_gr_db, cs1.compressor.gain_reduction_db());
                        AudioMeters::store_f32(&in_meters.tuner_detected_freq, cs1.tuner.detected_freq_hz.unwrap_or(0.0));
                        AudioMeters::store_f32(&in_meters.tuner_cents, cs1.tuner.cents_deviation);
                    }

                    AudioMeters::store_f32(&in_meters.in_l_peak, max_l);
                    AudioMeters::store_f32(&in_meters.in_r_peak, max_r);
                },
                |err| eprintln!("Input stream error: {err}"),
                None,
            )?,
            _ => return Err(anyhow!("Unsupported input sample format, expected F32")),
        };

        // Setup output stream
        let out_channels = out_config.channels() as usize;
        let out_rb_l = Arc::clone(&ring_buffer_l);
        let out_rb_r = Arc::clone(&ring_buffer_r);
        let out_r_idx = Arc::clone(&read_idx);
        let out_w_idx = Arc::clone(&write_idx);
        let out_master = Arc::clone(&master_chain);
        let out_meters = Arc::clone(&meters);

        let out_stream = match out_config.sample_format() {
            SampleFormat::F32 => out_dev.build_output_stream(
                &out_config.into(),
                move |data: &mut [f32], _| {
                    let num_frames = data.len() / out_channels;
                    let mut max_l = 0.0_f32;
                    let mut max_r = 0.0_f32;

                    if let (Ok(rb_l), Ok(rb_r), Ok(mut master)) = (out_rb_l.lock(), out_rb_r.lock(), out_master.lock()) {
                        let mut r = out_r_idx.load(Ordering::Relaxed);
                        let w = out_w_idx.load(Ordering::Relaxed);

                        for f in 0..num_frames {
                            let (in_l, in_r) = if r != w {
                                let l = rb_l[r % ring_buffer_size];
                                let right = rb_r[r % ring_buffer_size];
                                r = (r + 1) % (ring_buffer_size * 2);
                                (l, right)
                            } else {
                                (0.0, 0.0)
                            };

                            // Process stereo through Master Chain!
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
                        out_r_idx.store(r, Ordering::Relaxed);

                        // Telemetry for Master Chain
                        AudioMeters::store_f32(&out_meters.master_limiter_gr_db, master.limiter.gain_reduction_db());
                        AudioMeters::store_f32(&out_meters.master_true_peak_dbtp, master.meter.max_true_peak_dbtp);
                        AudioMeters::store_f32(&out_meters.momentary_lufs, master.meter.momentary_lufs);
                        AudioMeters::store_f32(&out_meters.short_term_lufs, master.meter.short_term_lufs);
                        AudioMeters::store_f32(&out_meters.integrated_lufs, master.meter.integrated_lufs);
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
            channel_strip_1,
            channel_strip_2,
            master_chain,
            meters,
            _input_stream: Some(in_stream),
            _output_stream: Some(out_stream),
            sample_rate,
        })
    }
}
