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

## Real-Time Engine Architecture: Lock-Free SPSC & Flipped Ownership

> **Architectural Evolution & Honesty Note:**
> Early prototypes of DeskDSP Control shared the DSP chains between threads using `Arc<Mutex<ChannelStrip>>`, `Arc<Mutex<MasterChain>>`, and `Arc<Mutex<Vec<f32>>>`. While the DSP maths and rack hot-swap logic were mathematically correct, acquiring standard `std::sync::Mutex` locks on the audio thread under UI contention (knob drags, preset changes, bypass toggling) risked thread priority inversion and audio dropouts. Furthermore, dropping a retired `Box<dyn DspNode>` directly on the audio thread invokes the system deallocator (`free()`), violating real-time safety.
>
> DeskDSP Control has been re-architected with **flipped ownership** and **SPSC message passing**:
> 1. **Audio Callback OWNS State:** `ChannelStrip` (Input 1 & 2) and `MasterChain` are moved by value directly into the real-time audio callback closures. The UI thread never takes a lock on audio DSP structures.
> 2. **Zero Mutexes on Real-Time Threads:** There is no `std::sync::Mutex` anywhere in the hot audio processing path.
> 3. **Lock-Free Control Plane (`rtrb`):** The UI control plane communicates with the audio engine via bounded, pre-allocated Single-Producer Single-Consumer (`rtrb`) ring buffers. All node Boxes are constructed off-thread on the UI thread.
> 4. **Never Drop on the Audio Thread (Garbage Return Queue):** When an effect node is removed or a rack is swapped on the audio thread, the retired `Box<dyn DspNode>` or `Box<MonoRack>` is transferred across a reverse SPSC garbage queue back to the UI/cleanup thread, where it is drained and safely dropped off the audio thread.
> 5. **Lock-Free Inter-Stream Ring Buffer:** The previous `Arc<Mutex<Vec<f32>>>` ring buffer is replaced with lock-free SPSC `rtrb::RingBuffer<f32>` (8192 capacity) between input and output audio callbacks.
> 6. **Zero-Allocation LV2 Host:** In `src/dsp/lv2_host.rs`, control port buffers (`ctrl_buffer: Vec<f32>`) are pre-allocated at instantiation time and updated in-place, eliminating dynamic heap allocations in `process_stereo()`.

---

## Dynamic Node Rack System

DeskDSP Control uses an ordered, pluggable dynamic rack architecture:
- **`MonoRack` (`Vec<Box<dyn DspNode>>`):** Used in `ChannelStrip` for tracking.
- **`StereoRack` (`Vec<Box<dyn StereoDspNode>>`):** Used in `MasterChain` for mastering.

### Real-Time Safety & Zero-Allocation Guarantees
- **Allocation-Free Audio Loop:** `process()` and `process_stereo()` iterate over pre-allocated trait object vectors with zero heap allocations.
- **Atomic Hot Swap:** Runtime chain modifications (adding/removing/reordering nodes or loading presets) can be constructed off-thread and swapped in with `swap_rack(new_rack)` via `std::mem::swap`. Retired racks are moved to the SPSC garbage queue — the audio thread never allocates or deallocates.
- **Bit-Identical Bypass:** Any bypassed node passes signal through bit-for-bit without DSP overhead or phase shifts.
- **Safe Typed Downcasting:** UI and hotkeys query concrete node parameters via `rack.find_node_mut::<T>()` and dispatch lock-free commands.

---

## Adding Custom Effects & Third-Party Plugin Hosting

### 1. Adding Custom Effects in Rust
Adding new DSP nodes is designed to be trivial and low friction. Follow the 4-step checklist in [`docs/ADDING_EFFECTS.md`](docs/ADDING_EFFECTS.md):
1. Create your module in `src/dsp/` implementing `DspNode` or `StereoDspNode`.
2. Register in `src/dsp/mod.rs`.
3. Push to `MonoRack` or `StereoRack`.
4. Add verification tests in `tests/dsp_verification_tests.rs`.

A complete worked template is provided in [`src/dsp/exciter.rs`](src/dsp/exciter.rs) (`HarmonicExciter`).

### 2. Third-Party Plugin Hosting (Linux LV2)
DeskDSP Control includes an LV2 host engine (`src/dsp/lv2_host.rs`) powered by the `livi` crate:
- **Ecosystem:** Targets the rich Linux open-source audio ecosystem (LSP Plugins, Calf Studio Gear, x42).
- **Cargo Feature:** Kept behind the `lv2-host` feature (`cargo build --features lv2-host`) so the standalone app and tests build without external dependencies on macOS and minimal Linux distros.
- **Port Bridging:** Bridges LV2 control ports directly to DeskDSP parameters with pre-allocated scratch buffers on the audio thread.

> **Honest Plugin Format Support Notice:**
> - **LV2:** Supported on Linux via `livi` (`lv2-host` feature).
> - **CLAP:** Targeted next as the primary cross-platform open plugin standard.
> - **VST3 & Audio Units (AU):** VST3 hosting in pure Rust remains immature and AU is macOS-only. **DeskDSP Control does NOT claim VST3 or AU plugin hosting support.** Only native Rust nodes, LV2 (Linux), and exported DAW targets (CLAP/VST3 via `nih-plug`) are implemented.

---

## Interactive Workstation (TUI)

Launch the full interactive terminal dashboard:
```bash
cargo run --release
```

### Hotkey Reference:
| Key | Function |
|---|---|
| `Tab` / `BackTab` | Cycle selected rack node in Channel Strip |
| `Space` | Toggle bypass on selected rack node |
| `<` / `>` | Move selected node up/down in signal chain (reorder) |
| `a` | Add new node to rack (pre-allocated capacity) |
| `Delete` / `Backspace` | Remove selected node from rack |
| `1` / `2` | Select Channel Strip Input 1 or 2 |
| `g` / `G` | Increase / decrease hardware preamp gain |
| `p` | Toggle +48V phantom power |
| `m` | Cycle input mode (`Mic` $\to$ `Line` $\to$ `Hi-Z`) |
| `v` / `V` | Adjust monitor output volume step |
| `e` | Quick toggle EQ bypass |
| `c` | Quick toggle Vocal Compressor bypass |
| `t` | Quick toggle Vocal Tuner bypass |
| `k` | Cycle Tuner Scale (`Chromatic` $\to$ `Major` $\to$ `Minor` $\to$ `Pentatonic`) |
| `s` | Quick toggle Analog Saturation bypass |
| `x` | Quick toggle Noise Gate bypass |
| `d` | Quick toggle De-Esser bypass |
| `b` | Toggle Master Bus Glue Compressor bypass |
| `l` | Toggle Master True-Peak Limiter bypass |
| `[` / `]` | Adjust vocal compressor threshold |
| `{` / `}` | Adjust saturation drive level |
| `q` / `Esc` | Graceful exit |

---

## Verification & Offline Test Suite

DeskDSP Control includes an offline verification test harness. **No physical hardware is required to run tests.**

Run the verification test suite:
```bash
cargo test
```

### 18 Verified Offline Test Cases:
1. **RBJ Biquad Coefficients:** Validates high-pass, low-shelf, peaking, and high-shelf filter coefficients against Robert Bristow-Johnson's Audio EQ Cookbook formulas.
2. **Bit-Identical Bypass Guarantee:** Proves every DSP node passes audio bit-identically (`sample_in == sample_out`) when bypassed.
3. **Compressor Transfer Curves:** Measures soft-knee threshold and gain reduction curves against mathematical expectations.
4. **Limiter True-Peak Ceiling:** Subjects the brickwall limiter to aggressive $+12\text{ dBFS}$ test tones and verifies that the output never exceeds the $-1.0\text{ dBTP}$ ceiling using a 4x oversampled polyphase FIR filter.
5. **EBU R128 Two-Stage Gating & Zero Allocation:** Validates BS.1770 K-weighted LKFS loudness measurement with -70 LKFS absolute and -10 LU relative gating using a 10,000-block circular buffer without heap allocations.
6. **Dynamic Rack Default Order & Bypass:** Asserts the tracking chain contains all 7 nodes in exact order and passes audio bit-identically when bypassed.
7. **Master Chain Order & Typed Downcast:** Asserts the 6-stage mastering chain and verifies typed downcasting (`find_node_mut::<TruePeakLimiter>()`).
8. **Off-Thread Rack Swap:** Tests zero-allocation hot-swapping of pre-built `MonoRack` instances in real time.
9. **Rack Reorder, Add & Remove:** Verifies bounds safety, insertion, and reordering without panics.
10. **Pitch Detector Precision:** Validates YIN pitch detection on synthetic sine waves ($A_3 = 220\text{ Hz}$, $C_4 = 261.63\text{ Hz}$, $A_4 = 440\text{ Hz}$) to within $\pm 0.5\text{ Hz}$.
11. **Scale Quantizer Snapping:** Verifies pitch snapping and cent deviation over a known glissando sweep.
12. **THD Measurement Harness:** Quantifies total harmonic distortion and harmonic generation across tube and tape saturation algorithms.
13. **Harmonic Exciter Bypass & Spectral Coloration:** Verifies high-pass sidechain filtering, quadratic overtone generation, and low-frequency immunity.
14. **Harmonic Exciter Rack Integration:** Tests dynamic rack integration and typed downcast.
15. **LV2 Host Scanning & Port Bridging:** Verifies plugin discovery, control port parameter clamping, and stereo rack execution.
16. **RT Zero Allocations Under Heavy Command Stream (`assert_no_alloc`):** Enforces 0 allocations and 0 frees in the audio processing thread while actively draining `InsertMonoNode`, `RemoveNode`, `SwapMonoRack`, `SetParam`, `SetBypass`, and `MoveNode` commands.
17. **Garbage Return Queue Drop Isolation:** Validates that retired nodes removed from racks are never dropped on the audio thread, transferring safely to the cleanup thread where deallocation is performed off-thread.
18. **Rack Insert-at-Capacity Boundary:** Proves that attempting to insert past pre-allocated capacity (`len >= capacity`) triggers zero vector reallocations on the audio thread, returning the uninserted node directly to the garbage return queue.

