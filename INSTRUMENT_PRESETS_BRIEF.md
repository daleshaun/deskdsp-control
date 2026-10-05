# Brief: Per-Channel Instrument Presets + Guitar/Keys/Piano DSP

**For:** antigravity (on-Mac implementation)
**Author:** Claude (peer/architecture)
**Target state at write:** `main` @ `7473f01`

## 1. Goal

Turn DeskDSP from a fixed dual-vocal-strip into a **per-channel instrument
workstation**. Each input channel can independently be assigned a processing
chain matched to its recording source (Vocal, Electric Guitar, Acoustic
Guitar, Bass, Keys/Synth, Piano, or Program-thru). Chains are hot-swappable
live with **zero audio-thread allocation**, selectable from the tablet.

Nothing here replaces existing DSP — it's additive. The vocal strip becomes
*one preset among several*.

## 2. What already exists (build on it, don't rebuild)

- `src/audio.rs:220-221` — two independent `ChannelStrip` instances (`cs1`,
  `cs2`), one per input. Per-channel chains already run separately.
- `src/dsp/channel_strip.rs:168` — `swap_rack(&mut self, new: MonoRack) ->
  MonoRack`: swaps the chain and **returns the old rack to the caller** so it
  drops off-thread. This is the RT-safe hot-swap primitive. It is currently
  **unused** — wire it up.
- `src/dsp/rack.rs` — `MonoRack` (`Vec<Box<dyn DspNode>>`, `with_capacity`,
  `push_boxed`, `try_insert` returning `Err(node)` on realloc) **and**
  `StereoRack`/`StereoDspNode` (already present — use for stereo keys/piano).
- `src/dsp/mod.rs:47` — `DspNode` trait: `name`, `is_bypassed`,
  `set_bypassed`, `reset`, `process_sample`, `process_block` (default),
  `telemetry`. Any new node implements this and drops straight into a rack.
- Existing garbage-return pattern on the audio thread (used by the rack
  insert-at-capacity fix) — reuse it to drop the old rack.
- `PreampMode::{Mic, Line, HiZ}` (`src/hardware.rs`) — the Hi-Z/instrument
  input mode for plugging guitar/bass straight in. Preset selection should
  *suggest* the right preamp mode but the user still controls it.

## 3. Architecture

### 3.1 Preset definition (new: `src/dsp/presets.rs`)

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InstrumentPreset {
    Vocal,          // existing strip
    ElectricGuitar, // gate -> amp -> cab -> eq -> comp
    AcousticGuitar, // hpf -> gate -> eq -> comp -> exciter -> (reverb)
    Bass,           // gate -> comp -> drive -> eq -> limiter
    Keys,           // eq -> comp -> exciter -> (chorus)
    Piano,          // eq -> comp(gentle) -> saturation -> (reverb)
    ProgramThru,    // empty rack: clean passthrough to master chain
}

/// Builds the node list OFF the audio thread. Allocates freely here.
pub fn build_rack(preset: InstrumentPreset, sample_rate: f32) -> MonoRack { ... }

/// Suggested preamp mode + phantom for the source (UI hint only).
pub fn suggested_input(preset: InstrumentPreset) -> (PreampMode, bool) { ... }
```

`build_rack` reuses existing nodes where they fit (EQ, comp, gate, saturation,
exciter) and the new nodes from §4. Match the current `ChannelStrip::new`
defaults: nodes start **bypassed** unless the preset's character needs them on
(e.g. the guitar amp/cab should be *on* when ElectricGuitar is selected —
otherwise the preset does nothing audible).

### 3.2 RT-safe swap path (critical — follow exactly)

**Never build a rack on the audio thread.** The flow:

1. Tablet sends `{type:"set_channel_preset", target:"ch1", preset:"eguitar"}`.
2. Remote/forwarder thread (the existing `tablet-dsp-forwarder` OS thread)
   calls `presets::build_rack(preset, sample_rate)` — allocation happens
   **here, off the audio thread**.
3. Forwarder enqueues `AudioCommand::SwapRack { target, rack }` on the
   existing rtrb producer (preserve the single-producer invariant — route
   through the same producer the forwarder already owns for that target).
4. Audio thread pops it, calls `cs.swap_rack(rack)`, gets the **old** rack
   back, and pushes it into the existing garbage-return producer so it drops
   off-thread. No alloc, no free on the audio thread.

`AudioCommand::SwapRack` carries a `MonoRack` (it's `Send`; it's heap
pointers, so the enum stays small). Add the variant alongside the existing
`SetNodeBypass` / `SetSourceMode` variants.

### 3.3 Generalize the source mode

The global `source_mode: AtomicBool` (Program/Vocal) becomes **per-channel
preset**. Keep `ProgramThru` as the preset that means "finished stereo mix,
skip the strip" (empty/all-bypassed rack → straight to master chain). You can
retire `source_mode` or keep it as a convenience that maps CH1+CH2 to
`ProgramThru`. Reflect the active preset per channel in the 20 Hz
`state_sync` broadcast so the tablet shows the right picker state.

### 3.4 Click-free swap

Instantaneous pointer swap is acceptable for v1 (users change instrument
between takes, not mid-note). If a click shows up in testing, add a ~10 ms
equal-power crossfade between old-rack and new-rack output on the audio
thread (keep both racks for the fade window, then garbage-return the old
one). Don't gold-plate this up front.

## 4. New DSP nodes (each is a `DspNode`, buffers allocated in `::new`)

### 4.1 `GuitarAmp` (`src/dsp/guitar_amp.rs`)
- Input drive gain → waveshaper → 3-band tone stack → output level.
- Waveshaper: asymmetric soft clip (e.g. `tanh` with a bias term, or a
  dual-slope curve) for tube-ish even+odd harmonics. Oversample 2–4× around
  the nonlinearity to avoid aliasing, then decimate (this is the one place
  aliasing will bite — do not skip oversampling).
- Tone stack: low-shelf / mid-peak / high-shelf biquads (reuse
  `BiquadFilter`).
- Params: `drive`, `bass`, `mid`, `treble`, `level`, `bypassed`.

### 4.2 `CabSim` (`src/dsp/cab_sim.rs`)
Two acceptable approaches — **recommend the parametric one for v1** (no IR
files to ship/manage, stays fully in-house / "my software, my logo"):
- **Parametric voiced cab (v1):** cascade of fixed biquads emulating a 1×12 /
  4×12 response — presence bump ~2–4 kHz, steep low-pass ~5–6 kHz, high-pass
  ~80 Hz, a low-mid scoop. Cheap, no IR, fully yours.
- **Convolution cab (v2, optional):** partitioned FFT convolution against a
  short (≤2048-tap) impulse response baked into the binary as a `const`
  array. Heavier; do only if the parametric voice isn't convincing.
- Expose a `cab_type` selector (e.g. `None / 1x12 / 2x12 / 4x12`).

### 4.3 `Drive` (`src/dsp/drive.rs`) — optional, for Bass/overdrive
- Simple pre-gain + soft-clip + post-gain + tilt EQ. Lighter than the full
  amp; used in the Bass preset and as a standalone grit stage.

### 4.4 `Chorus` (`src/dsp/chorus.rs`) — for Keys
- Modulated delay line (LFO 0.1–3 Hz, depth, mix). Delay buffer allocated in
  `::new` (off-thread). Linear/Hermite interpolated read. RT-safe.

### 4.5 `Reverb` (`src/dsp/reverb.rs`) — for Piano/Acoustic
- Freeverb-style (8 parallel combs + 4 series allpass per channel) or a small
  FDN. All buffers allocated in `::new`. Params: `size`, `damp`, `mix`,
  `pre_delay`. RT-safe — no per-sample alloc.

> Chorus/Reverb are time-based: all delay/comb buffers **must** be sized and
> allocated in `::new` (which runs off-thread via `build_rack`). Nothing grows
> on the audio thread.

## 5. Stereo keys/piano

v1: run the Keys/Piano preset **dual-mono** — assign it to both CH1 and CH2;
the master chain's existing `StereoWidthMidSide` gives the stereo image on the
bus. This needs no new stereo plumbing.

v2 (optional): a real **stereo-link** mode using the existing `StereoRack` so
CH1+CH2 are processed as a linked L/R pair (lets a stereo chorus/reverb see
both channels). Note the 2-input hardware limit: a stereo instrument uses
*both* inputs, so you can't also track a separate mono source at the same
time.

## 6. Tablet UI (`tablet_v3.html` — auto-embedded via `include_str!`, no bake step)

- Add a **per-channel instrument picker** on each channel (CH1, CH2): a
  segmented/dropdown control listing Vocal · E.Gtr · A.Gtr · Bass · Keys ·
  Piano · Program. Selecting sends
  `{type:"set_channel_preset", target:"ch1|ch2", preset:"vocal|eguitar|aguitar|bass|keys|piano|program"}`.
- The PROCESSING tab should show the **modules for the active preset** (amp +
  cab faces when a guitar preset is active; chorus/reverb for keys/piano),
  reusing the existing hardware-faceplate styling and per-unit IN/BYP buttons.
- Replace the global PROGRAM/VOCAL toggle with the per-channel picker
  (Program becomes the `program` option in the picker).
- Show a small "suggested input: Hi-Z / +48V" hint from `suggested_input`
  when a preset is chosen; do not auto-change the preamp — the user confirms.
- `onMsg` reflects each channel's active preset from `state_sync`.

## 7. Server plumbing (`src/remote/mod.rs`)

- Add `set_channel_preset` to the `RemoteMessage` enum (serde rename).
- In the forwarder: match preset string → `InstrumentPreset`, call
  `build_rack` off-thread, enqueue `AudioCommand::SwapRack{target, rack}` on
  the correct per-target producer.
- Emit each channel's active preset string in `state_sync`.

## 8. RT-safety checklist (must hold)

- [ ] No `build_rack` / `Box::new` / `Vec` growth on the audio thread.
- [ ] `SwapRack` old rack returned via garbage producer, dropped off-thread.
- [ ] All new nodes allocate every buffer in `::new`.
- [ ] Nonlinear stages (amp, drive) oversampled to control aliasing.
- [ ] `assert_no_alloc` still passes under preset switching.
- [ ] Single-producer invariant per rtrb producer preserved.

## 9. Tests

- Unit test each new node (amp drive curve monotonic + bounded; cab response
  shape; chorus/reverb produce output and don't NaN; tail decays).
- `build_rack` produces the expected node count/order per preset.
- Preset-swap smoke test: swap racks repeatedly while streaming a sine; assert
  no NaN/Inf and no sample-level discontinuity beyond threshold (or that the
  crossfade bounds it).
- RT-safety: existing `assert_no_alloc` harness around a swap.

## 10. Suggested build order

1. `presets.rs` + `InstrumentPreset` + `build_rack` using **only existing
   nodes** (Vocal, Keys=eq/comp/exciter, Piano=eq/comp/sat, Program=empty).
2. `AudioCommand::SwapRack` + forwarder wiring + tablet picker. **Ship this
   first** — it already gives Vocal/Keys/Piano/Program per channel with zero
   new DSP.
3. `GuitarAmp` + `CabSim` (parametric) → ElectricGuitar + Bass presets.
4. `Chorus` + `Reverb` → fill out Keys/Piano.
5. (optional) convolution cab; stereo-link mode.
