//! Standalone CLI Utility for Microphone Impulse Response Measurement & Deconvolution.
//!
//! Provides two essential functions for DeskDSP MicImage:
//! 1. `generate-sweep`: Synthesizes a high-precision exponential sine sweep test signal.
//! 2. `deconvolve`: Computes the transfer impulse response (Target ÷ Reference)
//!    using Tikhonov-regularized spectral deconvolution with bandpass tapering
//!    and direct-arrival peak alignment.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use realfft::{RealFftPlanner, RealToComplex, ComplexToReal};
use rustfft::num_complex::Complex32;

#[derive(Parser, Debug)]
#[command(name = "ir-tool")]
#[command(about = "Microphone Transfer IR Generation & Deconvolution Tool for DeskDSP", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Generate an exponential sine sweep WAV audio file for acoustic capture
    GenerateSweep {
        /// Output WAV file path
        #[arg(short, long, default_value = "mic_sweep_48k.wav")]
        output: PathBuf,

        /// Sweep duration in seconds (excluding silence tail)
        #[arg(short, long, default_value_t = 10.0)]
        duration: f32,

        /// Sample rate in Hz
        #[arg(long, default_value_t = 48000)]
        sample_rate: u32,

        /// Sweep start frequency in Hz
        #[arg(long, default_value_t = 20.0)]
        start_freq: f32,

        /// Sweep end frequency in Hz
        #[arg(long, default_value_t = 24000.0)]
        end_freq: f32,

        /// Silence tail duration in seconds
        #[arg(long, default_value_t = 1.0)]
        silence_tail: f32,
    },

    /// Deconvolve Target ÷ Reference to compute the microphone transfer IR
    Deconvolve {
        /// Reference mic recording (the mic you own, e.g. NT1, SM58)
        #[arg(short, long)]
        reference: Option<PathBuf>,

        /// Target mic recording (the sound you want, e.g. SM7B, U87)
        #[arg(short, long)]
        target: Option<PathBuf>,

        /// Combined stereo recording (Ch 1 = Reference, Ch 2 = Target)
        #[arg(short, long)]
        stereo: Option<PathBuf>,

        /// Output transfer IR WAV file
        #[arg(short, long, default_value = "transfer_ir.wav")]
        output: PathBuf,

        /// Optional output JSON file containing float sample array for WebSockets
        #[arg(long)]
        json: Option<PathBuf>,

        /// Truncated IR length in samples (512 or 1024 recommended for ConvEngine)
        #[arg(long, default_value_t = 512)]
        taps: usize,

        /// Tikhonov regularization epsilon (e.g. 0.0001 = -40 dB noise floor)
        #[arg(long, default_value_t = 0.0001)]
        regularization: f32,

        /// Low-cut frequency taper in Hz
        #[arg(long, default_value_t = 30.0)]
        low_cut: f32,

        /// High-cut frequency taper in Hz
        #[arg(long, default_value_t = 20000.0)]
        high_cut: f32,

        /// Peak normalization target in dBFS
        #[arg(long, default_value_t = -1.0)]
        normalize_db: f32,
    },
}

// ============================================================================
// Simple WAV File Reader & Writer (16/24-bit PCM & 32-bit Float)
// ============================================================================

struct WavData {
    sample_rate: u32,
    channels: u16,
    samples: Vec<f32>, // interleaved if multi-channel
}

fn read_wav(path: &PathBuf) -> Result<WavData> {
    let file = File::open(path).with_context(|| format!("Failed to open {:?}", path))?;
    let mut reader = BufReader::new(file);

    let mut header = [0u8; 12];
    reader.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WAVE" {
        return Err(anyhow!("Not a valid RIFF/WAVE file"));
    }

    let mut sample_rate = 48000;
    let mut channels = 1;
    let mut bits_per_sample = 16;
    let mut audio_format = 1; // 1 = PCM, 3 = IEEE Float
    let mut raw_data = Vec::new();

    // Parse RIFF chunks
    let mut chunk_header = [0u8; 8];
    while reader.read_exact(&mut chunk_header).is_ok() {
        let chunk_id = &chunk_header[0..4];
        let chunk_size = u32::from_le_bytes(chunk_header[4..8].try_into().unwrap()) as usize;

        if chunk_id == b"fmt " {
            let mut fmt_bytes = vec![0u8; chunk_size];
            reader.read_exact(&mut fmt_bytes)?;
            if fmt_bytes.len() < 16 {
                return Err(anyhow!("Invalid fmt chunk size"));
            }
            audio_format = u16::from_le_bytes(fmt_bytes[0..2].try_into().unwrap());
            channels = u16::from_le_bytes(fmt_bytes[2..4].try_into().unwrap());
            sample_rate = u32::from_le_bytes(fmt_bytes[4..8].try_into().unwrap());
            bits_per_sample = u16::from_le_bytes(fmt_bytes[14..16].try_into().unwrap());
        } else if chunk_id == b"data" {
            raw_data = vec![0u8; chunk_size];
            reader.read_exact(&mut raw_data)?;
            break;
        } else {
            // Skip unknown chunk
            let mut skip = vec![0u8; chunk_size];
            reader.read_exact(&mut skip)?;
        }
    }

    if raw_data.is_empty() {
        return Err(anyhow!("No audio data found in WAV file"));
    }

    let samples = match (audio_format, bits_per_sample) {
        (1, 16) => {
            let num_samples = raw_data.len() / 2;
            let mut s = Vec::with_capacity(num_samples);
            for i in 0..num_samples {
                let val = i16::from_le_bytes(raw_data[i * 2..i * 2 + 2].try_into().unwrap());
                s.push(val as f32 / 32768.0);
            }
            s
        }
        (1, 24) => {
            let num_samples = raw_data.len() / 3;
            let mut s = Vec::with_capacity(num_samples);
            for i in 0..num_samples {
                let b = &raw_data[i * 3..i * 3 + 3];
                let val = i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8;
                s.push(val as f32 / 8388608.0);
            }
            s
        }
        (3, 32) => {
            let num_samples = raw_data.len() / 4;
            let mut s = Vec::with_capacity(num_samples);
            for i in 0..num_samples {
                let val = f32::from_le_bytes(raw_data[i * 4..i * 4 + 4].try_into().unwrap());
                s.push(val);
            }
            s
        }
        _ => return Err(anyhow!("Unsupported WAV format: format={}, bits={}", audio_format, bits_per_sample)),
    };

    Ok(WavData {
        sample_rate,
        channels,
        samples,
    })
}

fn write_wav_mono_float(path: &PathBuf, samples: &[f32], sample_rate: u32) -> Result<()> {
    let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
    let mut writer = BufWriter::new(file);

    let num_samples = samples.len() as u32;
    let data_bytes = num_samples * 4;
    let total_file_size = 36 + data_bytes;

    // RIFF Header
    writer.write_all(b"RIFF")?;
    writer.write_all(&total_file_size.to_le_bytes())?;
    writer.write_all(b"WAVE")?;

    // fmt chunk (IEEE Float)
    writer.write_all(b"fmt ")?;
    writer.write_all(&16u32.to_le_bytes())?; // Chunk size 16
    writer.write_all(&3u16.to_le_bytes())?;  // Audio format 3 = IEEE float
    writer.write_all(&1u16.to_le_bytes())?;  // Channels: 1 (Mono)
    writer.write_all(&sample_rate.to_le_bytes())?;
    let byte_rate = sample_rate * 4;
    writer.write_all(&byte_rate.to_le_bytes())?;
    let block_align = 4u16;
    writer.write_all(&block_align.to_le_bytes())?;
    let bits_per_sample = 32u16;
    writer.write_all(&bits_per_sample.to_le_bytes())?;

    // data chunk
    writer.write_all(b"data")?;
    writer.write_all(&data_bytes.to_le_bytes())?;
    for &s in samples {
        writer.write_all(&s.to_le_bytes())?;
    }

    writer.flush()?;
    Ok(())
}

fn write_wav_mono_24bit(path: &PathBuf, samples: &[f32], sample_rate: u32) -> Result<()> {
    let file = File::create(path).with_context(|| format!("Failed to create {:?}", path))?;
    let mut writer = BufWriter::new(file);

    let num_samples = samples.len() as u32;
    let data_bytes = num_samples * 3;
    let total_file_size = 36 + data_bytes;

    writer.write_all(b"RIFF")?;
    writer.write_all(&total_file_size.to_le_bytes())?;
    writer.write_all(b"WAVE")?;

    writer.write_all(b"fmt ")?;
    writer.write_all(&16u32.to_le_bytes())?;
    writer.write_all(&1u16.to_le_bytes())?; // PCM
    writer.write_all(&1u16.to_le_bytes())?; // Mono
    writer.write_all(&sample_rate.to_le_bytes())?;
    let byte_rate = sample_rate * 3;
    writer.write_all(&byte_rate.to_le_bytes())?;
    let block_align = 3u16;
    writer.write_all(&block_align.to_le_bytes())?;
    let bits_per_sample = 24u16;
    writer.write_all(&bits_per_sample.to_le_bytes())?;

    writer.write_all(b"data")?;
    writer.write_all(&data_bytes.to_le_bytes())?;
    for &s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let int_val = (clamped * 8388607.0).round() as i32;
        let bytes = int_val.to_le_bytes();
        writer.write_all(&[bytes[0], bytes[1], bytes[2]])?;
    }

    writer.flush()?;
    Ok(())
}

// ============================================================================
// Core Math: Exponential Sine Sweep & Regularized Deconvolution
// ============================================================================

fn generate_exponential_sine_sweep(
    duration_s: f32,
    sample_rate: u32,
    start_freq: f32,
    end_freq: f32,
    silence_tail_s: f32,
) -> Vec<f32> {
    let sr = sample_rate as f32;
    let num_sweep_samples = (duration_s * sr).round() as usize;
    let num_tail_samples = (silence_tail_s * sr).round() as usize;
    let total_samples = num_sweep_samples + num_tail_samples;

    let mut out = vec![0.0_f32; total_samples];
    let w1 = 2.0 * std::f32::consts::PI * start_freq;
    let w2 = 2.0 * std::f32::consts::PI * end_freq;
    let l = duration_s;
    let log_ratio = (w2 / w1).ln();

    // Fade in/out window to prevent transducer click
    let fade_samples = (0.05 * sr) as usize; // 50ms fade

    for n in 0..num_sweep_samples {
        let t = n as f32 / sr;
        let phase = (w1 * l / log_ratio) * ((t / l * log_ratio).exp() - 1.0);
        let mut sample = phase.sin();

        // 50ms raised-cosine fade-in
        if n < fade_samples {
            let f = n as f32 / fade_samples as f32;
            let win = 0.5 * (1.0 - (f * std::f32::consts::PI).cos());
            sample *= win;
        }
        // 50ms raised-cosine fade-out
        if n >= num_sweep_samples - fade_samples {
            let f = (num_sweep_samples - 1 - n) as f32 / fade_samples as f32;
            let win = 0.5 * (1.0 - (f * std::f32::consts::PI).cos());
            sample *= win;
        }

        out[n] = sample * 0.90; // -1 dBFS peak headroom
    }

    out
}

fn deconvolve_transfer_ir(
    ref_samples: &[f32],
    target_samples: &[f32],
    sample_rate: u32,
    taps: usize,
    epsilon: f32,
    low_cut: f32,
    high_cut: f32,
    normalize_db: f32,
) -> Result<Vec<f32>> {
    let n_samples = ref_samples.len().max(target_samples.len());
    let fft_len = (n_samples * 2).next_power_of_two().max(4096);
    let spectrum_len = fft_len / 2 + 1;

    let mut planner = RealFftPlanner::<f32>::new();
    let fft_fwd: Arc<dyn RealToComplex<f32>> = planner.plan_fft_forward(fft_len);
    let fft_inv: Arc<dyn ComplexToReal<f32>> = planner.plan_fft_inverse(fft_len);

    let mut scratch_fwd = fft_fwd.make_scratch_vec();
    let mut scratch_inv = fft_inv.make_scratch_vec();

    // Prepare time buffers
    let mut ref_time = vec![0.0_f32; fft_len];
    let mut target_time = vec![0.0_f32; fft_len];
    ref_time[..ref_samples.len()].copy_from_slice(ref_samples);
    target_time[..target_samples.len()].copy_from_slice(target_samples);

    // Compute Forward FFT
    let mut ref_freq = vec![Complex32::new(0.0, 0.0); spectrum_len];
    let mut target_freq = vec![Complex32::new(0.0, 0.0); spectrum_len];

    fft_fwd.process_with_scratch(&mut ref_time, &mut ref_freq, &mut scratch_fwd)?;
    fft_fwd.process_with_scratch(&mut target_time, &mut target_freq, &mut scratch_fwd)?;

    // Find peak power of reference for relative Tikhonov regularization
    let max_ref_power = ref_freq.iter()
        .map(|c| c.norm_sqr())
        .fold(0.0_f32, f32::max)
        .max(1e-12);

    let reg_floor = epsilon * max_ref_power;

    // Transfer frequency response: H(f) = (Target * conj(Ref)) / (|Ref|^2 + epsilon * max_power)
    let mut transfer_freq = vec![Complex32::new(0.0, 0.0); spectrum_len];
    let sr = sample_rate as f32;

    for k in 0..spectrum_len {
        let freq_hz = (k as f32 / fft_len as f32) * sr;
        let r = ref_freq[k];
        let t = target_freq[k];

        // Tikhonov regularized division
        let denom = r.norm_sqr() + reg_floor;
        let conj_r = Complex32::new(r.re, -r.im);
        let mut h = (t * conj_r) / denom;

        // Bandpass tapering (smooth roll-off below low_cut and above high_cut)
        let taper = if freq_hz < low_cut {
            let f = (freq_hz / low_cut).clamp(0.0, 1.0);
            0.5 * (1.0 - (f * std::f32::consts::PI).cos())
        } else if freq_hz > high_cut {
            let f = ((sr * 0.5 - freq_hz) / (sr * 0.5 - high_cut)).clamp(0.0, 1.0);
            0.5 * (1.0 - (f * std::f32::consts::PI).cos())
        } else {
            1.0
        };

        h *= taper;
        transfer_freq[k] = h;
    }

    // Inverse FFT to get raw transfer impulse response
    let mut ir_time = vec![0.0_f32; fft_len];
    fft_inv.process_with_scratch(&mut transfer_freq, &mut ir_time, &mut scratch_inv)?;

    // Scale IFFT (1 / N)
    let scale = 1.0 / fft_len as f32;
    for s in ir_time.iter_mut() {
        *s *= scale;
    }

    // Direct Arrival Peak Finding:
    // Look in the first half of the buffer for the maximum absolute peak
    let search_window = (fft_len / 4).min(ir_time.len());
    let mut max_abs = 0.0_f32;
    let mut peak_idx = 0;
    for i in 0..search_window {
        let abs_val = ir_time[i].abs();
        if abs_val > max_abs {
            max_abs = abs_val;
            peak_idx = i;
        }
    }

    // Truncate to requested taps (e.g. 512 samples) with 16 samples of pre-arrival headroom
    let pre_delay = 16.min(peak_idx);
    let start_idx = peak_idx.saturating_sub(pre_delay);
    let mut final_ir = vec![0.0_f32; taps];

    for i in 0..taps {
        let src_idx = (start_idx + i) % fft_len;
        final_ir[i] = ir_time[src_idx];
    }

    // Apply smooth half-Hann fade-out on the tail (last 15% of taps) to avoid any edge click
    let fade_len = (taps as f32 * 0.15).round() as usize;
    let fade_start = taps - fade_len;
    for i in 0..fade_len {
        let f = (fade_len - 1 - i) as f32 / fade_len as f32;
        let win = 0.5 * (1.0 - (f * std::f32::consts::PI).cos());
        final_ir[fade_start + i] *= win;
    }

    // Peak Normalization
    let peak = final_ir.iter().map(|s| s.abs()).fold(0.0_f32, f32::max).max(1e-12);
    let target_peak = 10.0_f32.powf(normalize_db / 20.0);
    let norm_gain = target_peak / peak;
    for s in final_ir.iter_mut() {
        *s *= norm_gain;
    }

    Ok(final_ir)
}

// ============================================================================
// Main CLI Entrypoint
// ============================================================================

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::GenerateSweep {
            output,
            duration,
            sample_rate,
            start_freq,
            end_freq,
            silence_tail,
        } => {
            println!("🎵 Synthesizing exponential sine sweep ({} Hz -> {} Hz, {:.1}s @ {} Hz)...",
                start_freq, end_freq, duration, sample_rate);

            let sweep = generate_exponential_sine_sweep(
                duration,
                sample_rate,
                start_freq,
                end_freq,
                silence_tail,
            );

            write_wav_mono_24bit(&output, &sweep, sample_rate)?;
            println!("✓ Exported sweep signal to: {:?} ({} samples)", output, sweep.len());
            println!("👉 Play this sweep through studio monitors and record both your reference & target mics.");
        }

        Commands::Deconvolve {
            reference,
            target,
            stereo,
            output,
            json,
            taps,
            regularization,
            low_cut,
            high_cut,
            normalize_db,
        } => {
            let (ref_samples, target_samples, sample_rate) = if let Some(stereo_path) = stereo {
                println!("📂 Loading stereo recording: {:?}", stereo_path);
                let wav = read_wav(&stereo_path)?;
                if wav.channels < 2 {
                    return Err(anyhow!("Stereo recording must contain at least 2 channels (Ch1=Ref, Ch2=Target)"));
                }
                let frames = wav.samples.len() / wav.channels as usize;
                let mut r = Vec::with_capacity(frames);
                let mut t = Vec::with_capacity(frames);
                for i in 0..frames {
                    r.push(wav.samples[i * wav.channels as usize]);
                    t.push(wav.samples[i * wav.channels as usize + 1]);
                }
                (r, t, wav.sample_rate)
            } else if let (Some(r_path), Some(t_path)) = (reference, target) {
                println!("📂 Loading reference mic: {:?}", r_path);
                let ref_wav = read_wav(&r_path)?;
                println!("📂 Loading target mic: {:?}", t_path);
                let target_wav = read_wav(&t_path)?;

                if ref_wav.sample_rate != target_wav.sample_rate {
                    return Err(anyhow!(
                        "Sample rate mismatch: Reference is {} Hz, Target is {} Hz. Both must match.",
                        ref_wav.sample_rate, target_wav.sample_rate
                    ));
                }
                (ref_wav.samples, target_wav.samples, ref_wav.sample_rate)
            } else {
                return Err(anyhow!("Please provide either --stereo <file.wav> OR both --reference <ref.wav> and --target <target.wav>"));
            };

            println!("⚡ Computing transfer IR (Target ÷ Reference)...");
            println!("   • Sample Rate:    {} Hz", sample_rate);
            println!("   • Target Length:  {} taps ({:.1} ms)", taps, (taps as f32 / sample_rate as f32) * 1000.0);
            println!("   • Regularization: {} ({:.1} dB noise floor)", regularization, 10.0 * regularization.log10());
            println!("   • Bandpass Range: {:.0} Hz - {:.0} Hz", low_cut, high_cut);

            let ir = deconvolve_transfer_ir(
                &ref_samples,
                &target_samples,
                sample_rate,
                taps,
                regularization,
                low_cut,
                high_cut,
                normalize_db,
            )?;

            write_wav_mono_float(&output, &ir, sample_rate)?;
            println!("✓ Exported transfer IR WAV to: {:?}", output);

            if let Some(json_path) = json {
                let json_data = serde_json::json!({
                    "name": output.file_stem().and_then(|s| s.to_str()).unwrap_or("transfer_ir"),
                    "sample_rate": sample_rate,
                    "taps": taps,
                    "samples": ir,
                });
                let mut f = File::create(&json_path)?;
                serde_json::to_writer_pretty(&mut f, &json_data)?;
                println!("✓ Exported WebSocket JSON payload to: {:?}", json_path);
            }

            println!("\n🎉 Ready for DeskDSP! Open the touch tablet remote (http://localhost:8080), navigate to Vocal preset, and tap 'LOAD IR' to select {:?}.", output);
        }
    }

    Ok(())
}
