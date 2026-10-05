# Adding Effects & DSP Nodes to DeskDSP Control

DeskDSP Control is engineered around a modular, allocation-free dynamic rack system (`MonoRack` for vocal tracking chains and `StereoRack` for mix/master chains). Adding your own custom DSP algorithm is straightforward, low-friction, and backed by automated offline verification tests.

This guide outlines the trait contract, real-time safety invariants, the 4-step integration process, and a copy-paste template based on the worked `HarmonicExciter` node.

---

## 1. The Trait Contracts

DeskDSP defines two core traits in [`src/dsp/mod.rs`](file:///Users/shaun25/.gemini/antigravity-ide/scratch/deskdsp-control/src/dsp/mod.rs):

### Monophonic DSP Node: `DspNode`
Used for vocal channel strips, tracking inputs, and single-channel processors.
```rust
pub trait DspNode: Send + 'static {
    /// Human-readable name displayed in the UI and telemetry readouts.
    fn name(&self) -> &'static str;

    /// Process a single mono sample in place. MUST be allocation-free.
    fn process_sample(&mut self, input: f32) -> f32;

    /// Block processing helper (default implementation loops over samples).
    fn process_block(&mut self, buffer: &mut [f32]) {
        for sample in buffer.iter_mut() {
            *sample = self.process_sample(*sample);
        }
    }

    /// Bypass status query.
    fn is_bypassed(&self) -> bool;

    /// Toggle bypass on or off.
    fn set_bypassed(&mut self, bypassed: bool);

    /// Clear internal state, delay lines, and filter history buffers.
    fn reset(&mut self);

    /// Any downcasting for dynamic GUI and hotkey parameter access.
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    /// Optional telemetry output (gain reduction, meter peaks, pitch).
    fn telemetry(&self) -> NodeTelemetry {
        NodeTelemetry::default()
    }
}
```

### Stereo DSP Node: `StereoDspNode`
Used for mix-bus processing, stereo widening, and mastering.
```rust
pub trait StereoDspNode: Send + 'static {
    fn name(&self) -> &'static str;
    fn process_stereo(&mut self, left: f32, right: f32) -> (f32, f32);
    fn is_bypassed(&self) -> bool;
    fn set_bypassed(&mut self, bypassed: bool);
    fn reset(&mut self);
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    fn telemetry(&self) -> NodeTelemetry {
        NodeTelemetry::default()
    }
}
```

---

## 2. Hard Real-Time (RT) Rules

To maintain click-free, sub-millisecond audio performance on both macOS (CoreAudio) and Linux (ALSA/JACK/PipeWire), all nodes must adhere to four strict rules:

| Rule | Requirement | Why It Matters |
| :--- | :--- | :--- |
| **1. Zero Audio-Thread Allocations** | Never call `Vec::push`, `Box::new`, `String::from`, or format strings inside `process_sample()` or `process_stereo()`. | OS memory allocators can acquire kernel locks, causing audio buffer dropouts (underruns/xruns). |
| **2. Bit-Identical Bypass** | When `is_bypassed()` is `true`, `process_sample(s)` must return `s` exactly (`assert_eq!(proc, s)`). | Prevents DC shifts, filter phase smearing, or level jumps when A/B testing effects. |
| **3. 64-Bit Filter Precision (`f64`)** | All IIR biquad filters and accumulators must calculate state and coefficients in `f64` (e.g. via [`BiquadFilter`](file:///Users/shaun25/.gemini/antigravity-ide/scratch/deskdsp-control/src/dsp/biquad.rs)). | 32-bit float accumulators suffer severe quantization noise and limit cycles at low frequencies (< 100 Hz). |
| **4. Pre-Allocated Buffers** | Pre-size delay lines and history buffers in `new(sample_rate)`. | Guarantees deterministic, bounded O(1) execution time per audio sample. |

---

## 3. The 4 Steps to Add an Effect

### Step 1: Create Your Effect Module
Create `src/dsp/my_effect.rs`. Implement your struct with `new(sample_rate)` and implement `DspNode` (or `StereoDspNode`).

```rust
use crate::dsp::biquad::{BiquadFilter, FilterType};
use crate::dsp::{DspNode, NodeTelemetry};

pub struct MyTremolo {
    rate_hz: f32,
    depth: f32,
    phase: f32,
    sample_rate: f32,
    bypassed: bool,
}

impl MyTremolo {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            rate_hz: 5.0,
            depth: 0.5,
            phase: 0.0,
            sample_rate,
            bypassed: false,
        }
    }
}

impl DspNode for MyTremolo {
    fn name(&self) -> &'static str { "Tremolo" }
    fn is_bypassed(&self) -> bool { self.bypassed }
    fn set_bypassed(&mut self, b: bool) { self.bypassed = b; }
    fn reset(&mut self) { self.phase = 0.0; }
    
    #[inline(always)]
    fn process_sample(&mut self, input: f32) -> f32 {
        if self.bypassed { return input; }
        let lfo = (2.0 * std::f32::consts::PI * self.phase).sin() * 0.5 + 0.5;
        let mod_gain = 1.0 - self.depth * (1.0 - lfo);
        self.phase = (self.phase + self.rate_hz / self.sample_rate).fract();
        input * mod_gain
    }

    fn as_any(&self) -> &dyn std::any::Any { self }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
}
```

### Step 2: Register in `src/dsp/mod.rs`
Add your module and export the node:
```rust
pub mod my_effect;
pub use my_effect::MyTremolo;
```

### Step 3: Add to a Rack or Chain
You can add your node to default chains in [`src/dsp/channel_strip.rs`](file:///Users/shaun25/.gemini/antigravity-ide/scratch/deskdsp-control/src/dsp/channel_strip.rs) or [`src/dsp/master_chain.rs`](file:///Users/shaun25/.gemini/antigravity-ide/scratch/deskdsp-control/src/dsp/master_chain.rs):
```rust
// Inside ChannelStrip::new(sample_rate)
rack.push(MyTremolo::new(sample_rate));
```
Or push it dynamically at runtime into any `MonoRack` or `StereoRack`:
```rust
channel_strip.rack.push(MyTremolo::new(48000.0));
```

### Step 4: Add Unit Tests & Expose in UI
Add an offline verification test in [`tests/dsp_verification_tests.rs`](file:///Users/shaun25/.gemini/antigravity-ide/scratch/deskdsp-control/tests/dsp_verification_tests.rs):
1. Test mathematical correctness (LFO shape, harmonic generation, or frequency response).
2. Test bit-identical bypass (`assert_eq!(node.process_sample(x), x)`).
3. Verify zero heap reallocations.

---

## 4. Worked Example: `HarmonicExciter`

The reference implementation is located in [`src/dsp/exciter.rs`](file:///Users/shaun25/.gemini/antigravity-ide/scratch/deskdsp-control/src/dsp/exciter.rs).

It demonstrates:
- High-pass sidechain filtering with $f64$ accumulator state (`BiquadFilter`).
- Asymmetric polynomial non-linear saturation generating 2nd (even) and 3rd (odd) harmonics.
- Wet/dry blending without volume drop.
- Zero audio-thread allocation.
- Bit-identical bypass.

Run verification tests anytime:
```bash
cargo test --test dsp_verification_tests
```
