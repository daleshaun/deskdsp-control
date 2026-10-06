#!/usr/bin/env python3
"""
Microphone Transfer IR Deconvolution Tool (Python / NumPy / SciPy)

Computes the transfer impulse response H_transfer = Target ÷ Reference using
Tikhonov-regularized spectral division with bandpass windowing and peak alignment.

Usage:
    # 1. Generate test sweep:
    python3 tools/deconvolve_mic_ir.py sweep --output sweep.wav --duration 10.0

    # 2. Deconvolve:
    python3 tools/deconvolve_mic_ir.py deconvolve \
        --reference my_nt1_rec.wav \
        --target borrowed_sm7b_rec.wav \
        --output sm7b_from_nt1.wav \
        --taps 512
"""

import argparse
import math
import sys
import wave
import struct

def parse_args():
    parser = argparse.ArgumentParser(description="Microphone Transfer IR Tool")
    subparsers = parser.add_subparsers(dest="command", required=True)

    # Sweep command
    sweep_parser = subparsers.add_parser("sweep", help="Generate exponential sine sweep")
    sweep_parser.add_argument("-o", "--output", default="mic_sweep_48k.wav", help="Output WAV path")
    sweep_parser.add_argument("-d", "--duration", type=float, default=10.0, help="Duration in seconds")
    sweep_parser.add_argument("-r", "--sample-rate", type=int, default=48000, help="Sample rate in Hz")
    sweep_parser.add_argument("--start-freq", type=float, default=20.0, help="Start frequency in Hz")
    sweep_parser.add_argument("--end-freq", type=float, default=24000.0, help="End frequency in Hz")

    # Deconvolve command
    deconv_parser = subparsers.add_parser("deconvolve", help="Deconvolve Target ÷ Reference")
    deconv_parser.add_argument("-r", "--reference", help="Reference mic WAV file")
    deconv_parser.add_argument("-t", "--target", help="Target mic WAV file")
    deconv_parser.add_argument("-s", "--stereo", help="Stereo WAV file (Ch1=Ref, Ch2=Target)")
    deconv_parser.add_argument("-o", "--output", default="transfer_ir.wav", help="Output transfer IR WAV")
    deconv_parser.add_argument("--taps", type=int, default=512, help="IR length (512 or 1024)")
    deconv_parser.add_argument("--epsilon", type=float, default=1e-4, help="Tikhonov regularization")
    deconv_parser.add_argument("--low-cut", type=float, default=30.0, help="Low-cut filter in Hz")
    deconv_parser.add_argument("--high-cut", type=float, default=20000.0, help="High-cut filter in Hz")
    deconv_parser.add_argument("--norm-db", type=float, default=-1.0, help="Peak normalization in dBFS")

    return parser.parse_args()

def generate_sweep(filename, duration, sample_rate, start_freq, end_freq):
    n_samples = int(duration * sample_rate)
    tail_samples = int(1.0 * sample_rate)
    total_samples = n_samples + tail_samples

    w1 = 2.0 * math.pi * start_freq
    w2 = 2.0 * math.pi * end_freq
    l = duration
    log_ratio = math.log(w2 / w1)
    fade_len = int(0.05 * sample_rate)

    samples = []
    for n in range(n_samples):
        t = n / sample_rate
        phase = (w1 * l / log_ratio) * (math.exp(t / l * log_ratio) - 1.0)
        s = math.sin(phase)
        if n < fade_len:
            s *= 0.5 * (1.0 - math.cos(math.pi * n / fade_len))
        elif n >= n_samples - fade_len:
            s *= 0.5 * (1.0 - math.cos(math.pi * (n_samples - 1 - n) / fade_len))
        samples.append(s * 0.9)

    samples.extend([0.0] * tail_samples)

    with wave.open(filename, 'wb') as wf:
        wf.setnchannels(1)
        wf.setsampwidth(2)
        wf.setframerate(sample_rate)
        packed = struct.pack(f'<{len(samples)}h', *[int(max(-1.0, min(1.0, s)) * 32767.0) for s in samples])
        wf.writeframes(packed)

    print(f"✓ Sweep generated: {filename} ({len(samples)} samples @ {sample_rate} Hz)")

def run_deconvolve_numpy(ref_file, target_file, stereo_file, out_file, taps, epsilon, low_cut, high_cut, norm_db):
    try:
        import numpy as np
    except ImportError:
        print("❌ Error: NumPy is required to run deconvolution via Python.")
        print("👉 You can use the native DeskDSP binary instead (zero dependencies):")
        print(f"   target/release/ir_tool deconvolve -r {ref_file} -t {target_file} -o {out_file} --taps {taps}")
        sys.exit(1)

    # Read WAV helper
    def read_audio(path):
        with wave.open(path, 'rb') as wf:
            channels = wf.getnchannels()
            width = wf.getsampwidth()
            rate = wf.getframerate()
            frames = wf.readframes(wf.getnframes())
            if width == 2:
                data = np.frombuffer(frames, dtype=np.int16).astype(np.float32) / 32768.0
            elif width == 4:
                data = np.frombuffer(frames, dtype=np.float32)
            else:
                raise ValueError(f"Unsupported sample width: {width}")
            if channels > 1:
                data = data.reshape(-1, channels)
            return data, rate, channels

    if stereo_file:
        data, rate, channels = read_audio(stereo_file)
        if channels < 2:
            raise ValueError("Stereo file must have at least 2 channels")
        ref = data[:, 0]
        target = data[:, 1]
    else:
        ref, r_rate, _ = read_audio(ref_file)
        target, t_rate, _ = read_audio(target_file)
        assert r_rate == t_rate, "Sample rates must match"
        rate = r_rate

    n = max(len(ref), len(target))
    fft_len = 1 << (2 * n - 1).bit_length()

    R = np.fft.rfft(ref, n=fft_len)
    T = np.fft.rfft(target, n=fft_len)

    max_power = np.max(np.abs(R)**2)
    denom = np.abs(R)**2 + (epsilon * max_power)
    H = (T * np.conj(R)) / denom

    # Bandpass window
    freqs = np.fft.rfftfreq(fft_len, 1.0 / rate)
    taper = np.ones_like(freqs)
    low_mask = freqs < low_cut
    taper[low_mask] = 0.5 * (1.0 - np.cos(np.pi * freqs[low_mask] / low_cut))
    high_mask = freqs > high_cut
    taper[high_mask] = 0.5 * (1.0 - np.cos(np.pi * (rate * 0.5 - freqs[high_mask]) / (rate * 0.5 - high_cut)))
    H *= taper

    ir = np.fft.irfft(H, n=fft_len)
    peak_idx = np.argmax(np.abs(ir[:fft_len // 4]))
    start_idx = max(0, peak_idx - 16)
    truncated = ir[start_idx : start_idx + taps]

    if len(truncated) < taps:
        truncated = np.pad(truncated, (0, taps - len(truncated)))

    # Fade out tail
    fade_len = int(taps * 0.15)
    truncated[-fade_len:] *= 0.5 * (1.0 - np.cos(np.pi * np.arange(fade_len)[::-1] / fade_len))

    # Normalize
    peak = np.max(np.abs(truncated))
    target_peak = 10.0 ** (norm_db / 20.0)
    truncated = (truncated / peak) * target_peak

    with wave.open(out_file, 'wb') as wf:
        wf.setnchannels(1)
        wf.setsampwidth(2)
        wf.setframerate(rate)
        packed = struct.pack(f'<{len(truncated)}h', *[int(max(-1.0, min(1.0, s)) * 32767.0) for s in truncated])
        wf.writeframes(packed)

    print(f"✓ Exported transfer IR: {out_file} ({taps} taps @ {rate} Hz)")

def main():
    args = parse_args()
    if args.command == "sweep":
        generate_sweep(args.output, args.duration, args.sample_rate, args.start_freq, args.end_freq)
    elif args.command == "deconvolve":
        run_deconvolve_numpy(args.reference, args.target, args.stereo, args.output, args.taps,
                             args.epsilon, args.low_cut, args.high_cut, args.norm_db)

if __name__ == "__main__":
    main()
