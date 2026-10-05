# DeskDSP Control

> **Note:** Formerly developed under the prototype name `zen-dsp-workstation`. Renamed and unified into **DeskDSP Control** — a modular, allocation-free real-time DSP suite running on the Antelope Audio Zen Go Synergy Core.

A native Rust host DSP suite and hardware controller for the **Antelope Audio Zen Go Synergy Core**. DeskDSP Control pairs low-latency, allocation-free software DSP chains running natively on your CPU with direct USB HID hardware control over the Zen Go's analog preamps, converters, and monitor outputs.

---

## Architecture & Signal Paths

DeskDSP Control provides two distinct, modular real-time signal paths:

### 1. Vocal Channel Strip (Tracking)
Ordered vocal tracking pipeline designed for low latency and zero heap allocations in the audio thread:

```
Input Trim / Phase Invert
         │
         ▼
80 Hz High-Pass Filter (12 dB/oct Butterworth)
         │
         ▼
Noise Gate / Expander (Lookahead-free, hold & smooth envelope)
         │
         ▼
Vocal De-Esser (3 kHz–11 kHz sidechain bandpass detection)
         │
         ▼
4-Band Parametric EQ (Low-shelf 100Hz, Low-mid 450Hz, High-mid 3.2kHz, High-shelf 10kHz)
         │
         ▼
Vocal Compressor (Soft-knee, Opto dual-stage release & FET response)
         │
         ▼
Vocal Tuner (YIN pitch detection + musical scale quantizer + PSOLA pitch shifter)
         │
         ▼
Analog Saturation (Tube even-harmonic & Tape odd-harmonic warmth + DC blocker)
         │
         ▼
Output Trim & Safety Peak Limiter
```

### 2. Master Chain (Mix Bus & Mastering)
Ordered stereo mix bus processing and True-Peak mastering:

```
5-Band Minimum-Phase Mastering EQ (Sub, Low-mid, Mid, High-mid, Air shelf)
         │
         ▼
3-Band Multiband Compressor (Linkwitz-Riley 4th order crossovers: 160Hz & 3.2kHz)
         │
         ▼
Stereo Width / Mid-Side Processor (Width 0..200% with elliptical mono-bass filter < 120Hz)
         │
         ▼
VCA Glue Compressor + Tape Warmth (Mix bus glue with 80Hz sidechain HPF)
         │
         ▼
True-Peak Brickwall Limiter (Fixed -1.0 dBTP ceiling with oversampled inter-sample peak detection)
         │
         ▼
EBU R128 Loudness Metering (Momentary 400ms, Short-Term 3s, Integrated LUFS, and True-Peak dBTP)
```

---

## Hardware Control Plane (USB HID)

DeskDSP Control directly interfaces with the Antelope Zen Go Synergy Core over USB HID (Interface 3, Vendor ID `0x23E5`, Product ID `0xA015`):
- **Preamp Gain:** Discrete 0–65 dB analog preamp gain for Inputs 1 & 2.
- **Input Modes:** Mic, Line, and Hi-Z impedance switching.
- **Phantom Power:** Independent +48V phantom power toggle per channel.
- **Phase Invert:** Polarity inversion.
- **Output Attenuation:** Independent volume control for Main Monitor, Headphone 1, and Headphone 2.
- **Bidirectional Telemetry:** Decodes 0x73 telemetry frame snapshots to keep the UI in sync with physical knob rotations.

---

## Vocal Tuner: Status & Quality Notice

> **Honest Assessment / Known Boundaries:**
> Artifact-free vocal pitch correction is technically complex. DeskDSP Control implements a solid, monophonic time-domain pitch corrector (YIN pitch detector + musical scale quantizer + dual-tap windowed pitch shifter).
> 
> * **Verified capabilities:** Detects fundamental vocal pitch accurately (70–900 Hz), snaps to musical scales (Chromatic, Major, Minor, Pentatonic), and corrects monophonic solo voice smoothly with configurable retune speed and correction strength.
> * **Current limitations:** It is intended for monophonic tracking; it is not yet an artifact-free commercial Auto-Tune or Melodyne clone. Rapid vibrato or polyphonic inputs may exhibit time-domain modulation. Phase 2 roadmap introduces phase-vocoder spectral processing and formant preservation.

---

## Verification & Offline Test Suite

DeskDSP Control includes an offline verification test harness. **No physical hardware is required to run tests.**

Run the verification test suite:
```bash
cargo test
```

### Verified Test Cases:
1. **RBJ Biquad Coefficients:** Validates high-pass, low-shelf, peaking, and high-shelf filter coefficients against Robert Bristow-Johnson's Audio EQ Cookbook formulas.
2. **Bit-Identical Bypass Guarantee:** Proves every DSP node passes audio bit-identically (`sample_in == sample_out`) when bypassed.
3. **Compressor Transfer Curves:** Measures soft-knee threshold and gain reduction curves against mathematical expectations.
4. **Limiter True-Peak Ceiling:** Subjects the brickwall limiter to aggressive $+12\text{ dBFS}$ test tones and verifies that the output never exceeds the $-1.0\text{ dBTP}$ ceiling.
5. **Pitch Detector Precision:** Validates YIN pitch detection on synthetic sine waves ($A_3 = 220\text{ Hz}$, $C_4 = 261.63\text{ Hz}$, $A_4 = 440\text{ Hz}$) to within $\pm 0.5\text{ Hz}$.
6. **Scale Quantizer Snapping:** Verifies pitch snapping and cent deviation over a known glissando sweep.
7. **THD Measurement Harness:** Quantifies total harmonic distortion and harmonic generation across tube and tape saturation algorithms.

---

## Cross-Platform & Linux Support

DeskDSP Control is engineered to run seamlessly on macOS and Linux (including the mini PC setup):

- **macOS:** CoreAudio via `cpal` + `hidapi`.
- **Linux:** ALSA, PipeWire, or JACK via `cpal` + `hidapi`.

### Linux Udev Rules (Non-Root USB HID Access)
To access the Zen Go hardware without running as root, copy the included udev rule:

```bash
sudo cp udev/99-antelope-zengo.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```

Ensure your user is a member of the `audio` group:
```bash
sudo usermod -aG audio $USER
```

---

## Interactive Workstation (TUI)

Launch the full interactive terminal dashboard:
```bash
cargo run --release
```

### Hotkey Reference:
| Key | Function |
|---|---|
| `1` / `2` | Select Channel Strip Input 1 or 2 |
| `g` / `G` | Increase / decrease hardware preamp gain |
| `p` | Toggle +48V phantom power |
| `m` | Cycle input mode (`Mic` $\to$ `Line` $\to$ `Hi-Z`) |
| `v` / `V` | Adjust monitor output volume step |
| `e` | Toggle 4-Band Parametric EQ bypass |
| `c` | Toggle Vocal Compressor bypass |
| `t` | Toggle Vocal Tuner bypass |
| `k` | Cycle Tuner Scale (`Chromatic` $\to$ `Major` $\to$ `Minor` $\to$ `Pentatonic`) |
| `s` | Toggle Analog Saturation bypass |
| `x` | Toggle Noise Gate bypass |
| `d` | Toggle De-Esser bypass |
| `b` | Toggle Master Bus Glue Compressor bypass |
| `l` | Toggle Master True-Peak Limiter bypass |
| `[` / `]` | Adjust vocal compressor threshold |
| `{` / `}` | Adjust saturation drive level |
| `q` / `Esc` | Graceful exit |

---

## Headless CLI Modes

Control hardware or stream audio non-interactively without opening the TUI:

* **Hardware status query:**
  ```bash
  deskdsp-control --status
  ```
* **Set preamp gain:**
  ```bash
  deskdsp-control --gain 1:42
  ```
* **Toggle +48V phantom power:**
  ```bash
  deskdsp-control --phantom 1
  ```
* **Headless audio streaming benchmark (e.g. 5 seconds):**
  ```bash
  deskdsp-control --test-audio 5
  ```

---

## Presets & Configuration

DeskDSP Control includes preset serialization (`src/presets.rs`) supporting JSON preset save/load with standard vocal chains (`Modern Pop Vocal`, `Warm Tube Vocal`).
