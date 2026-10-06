# Brief: MK3 + QCon Control-Surface Integration

## 0. Execution mode — RUN TO COMPLETION, DO NOT PAUSE FOR APPROVAL

**Work this entire brief through to the end in one continuous run. Do NOT stop to
ask "should I proceed?", "ready to submit?", or wait for a yes/confirm between
steps.** Go straight through: implement → build → test → fix failures → repeat
until green → commit → `git push origin main`. Then report what you did.

- Do not present a plan and wait for sign-off. Start building.
- Do not pause between build phases for confirmation. Continue to the next phase.
- If something is ambiguous, pick the most reasonable default, **note the
  assumption in your final report**, and keep going — do not halt to ask.
- Only stop early for a genuine hard blocker: a destructive/irreversible action,
  missing credentials/hardware you cannot work around, or a contradiction that
  makes the task impossible. Otherwise, finish.
- Always end by committing and pushing to `origin/main`, then summarizing.

**For:** antigravity (on-Mac implementation)
**Author:** Claude (peer/architecture)
**Hard constraint:** must be **built and tested WITHOUT the hardware connected.** The
user does not have the MK3/QCon plugged in during development. All parsing and
mapping is unit-tested against **synthetic MIDI byte sequences**; the physical
units just work when plugged in later.

## 1. Goal

Let a **Maschine MK3** (MIDI mode) and an **iCON QCon Pro G2** (Mackie Control)
drive DeskDSP as hardware control surfaces, alongside the tablet. They produce
the **same internal commands the tablet already produces** — no new DSP, minimal
new command types.

- **MK3** = pads + knobs for triggering/params (MIDI in only).
- **QCon** = motorized faders + V-pots + transport, **bidirectional** (must send
  fader/LED state *back* so motors mirror DSP state).

## 2. Architecture (reuse the existing command path)

There is already a clean seam: the tablet sends `RemoteMessage`s which a forwarder
turns into audio commands (`src/remote/mod.rs`). Control surfaces plug into the
**same command channel** — they are just another producer of the existing
messages.

```
MK3  ─(MIDI in)─┐
                ├─► control-surface thread ─► maps to existing commands ─► same
QCon ─(MIDI io)─┘   (midir callback)          (SetDspParam, SetChannelPreset,     dsp/hw
                                               SetNodeBypass, SetGain, …)          channel
QCon ◄─(MIDI out)─── feedback from 20 Hz state_sync (fader pos, LEDs)
```

- **No audio-thread impact.** MIDI runs on its own thread (the `midir` callback),
  feeding the existing std-mpsc command channel — exactly the same discipline as
  the tablet forwarder. Never touch the audio callback.
- **Add the `midir` crate** (cross-platform; uses CoreMIDI on macOS). No other new
  deps.

## 3. New module: `src/control/mod.rs`

```rust
/// A control surface translates device MIDI <-> DeskDSP commands + state.
pub trait ControlSurface: Send {
    /// Parse one inbound MIDI message into zero or more commands.
    /// PURE + hardware-free → unit-testable with synthetic bytes.
    fn on_midi(&mut self, msg: &[u8]) -> Vec<SurfaceCommand>;

    /// Produce outbound MIDI to reflect current DSP state (motors/LEDs).
    /// Called from the 20 Hz state tick. Empty for input-only devices (MK3).
    fn feedback(&mut self, state: &SurfaceState) -> Vec<Vec<u8>>;
}

/// Maps 1:1 onto the existing RemoteMessage/command set — don't invent a parallel
/// command system. SurfaceCommand is a thin enum that the runner converts into the
/// same messages the tablet produces.
pub enum SurfaceCommand {
    DspParam { target: Target, param_id: &'static str, value: f32 },
    ChannelPreset { target: Target, preset: InstrumentPreset },
    NodeBypass { target: Target, node: &'static str, bypassed: bool },
    Gain { input: u8, gain_db: u8 },
    MonitorVolume(u8), Hp1Volume(u8), Hp2Volume(u8),
    GlobalBypass(bool),
    Trigger { pad: u8, velocity: u8 },   // MK3 pads
}
```

`SurfaceState` is a small snapshot built from the **same data as the 20 Hz
`state_sync`** (channel levels, preset per channel, bypass flags, meter values).
Reuse it — don't add a second telemetry path.

The runner (a dedicated OS thread) owns the `midir` input/output connections,
calls `on_midi` for each inbound message, forwards the resulting `SurfaceCommand`s
onto the existing command channel, and on each 20 Hz tick calls `feedback` and
writes the bytes to the MIDI output.

## 4. MK3 mapping (`src/control/mk3.rs`) — MIDI in only

MK3 in MIDI mode is class-compliant. Default map (make it a table so it's easy to
re-map):
- **16 pads** → `Note On/Off` (notes 36–51). → `Trigger { pad, velocity }` and/or
  per-pad node-bypass toggles (e.g. pad 1 = toggle PITCH IN/BYP on selected channel).
- **8 knobs** → `CC` (e.g. CC 16–23) → `DspParam` for the **selected channel**,
  mapped to the preset's visible params (e.g. comp_threshold, comp_ratio,
  eq_low_gain, eq_hi_gain, sat_drive, amp_drive…). Use the real param-ids from
  `audio.rs` `SetParam` (comp_threshold, comp_ratio, sat_drive, gate_threshold,
  hpf_freq, eq_low_gain, eq_lmid_gain, eq_hmid_gain, eq_hi_gain, amp_drive,
  drive_gain, drive_blend, chorus_mix, …).
- **Transport / mode buttons** → channel select (CH1/CH2), preset step, global
  bypass. Since 8 knobs < all params, add a **bank/shift** concept (a button
  shifts the 8 knobs to the next param page).
- Knob CCs are 7-bit absolute (0–127) → scale to each param's real range in the map.

## 5. QCon mapping (`src/control/qcon.rs`) — Mackie Control (MCU), bidirectional

Standard MCU protocol (give antigravity these exact values so it doesn't guess):
- **8 motorized faders** → inbound `Pitch Bend` on MIDI channels 0–7 (14-bit),
  master fader = channel 8 → channel levels / master. **Feedback:** write
  `Pitch Bend` back on the same channel to move the motor to DSP state.
- **8 V-pots** (rotary encoders) → inbound `CC 0x10–0x17` (relative/2's-complement
  increments) → `DspParam` for that channel strip. **Feedback:** V-pot ring LEDs
  via `CC 0x30–0x37`.
- **Channel buttons**: REC/SOLO/MUTE/SELECT = `Note On` (mute 0x10–0x17,
  solo 0x08–0x0F, select 0x18–0x1F, rec 0x00–0x07). Mute → node/global bypass;
  select → which channel the V-pots/knobs address. **Feedback:** light the LED by
  sending `Note On` with velocity 0x7F (on) / 0x00 (off).
- **Transport**: Play 0x5E, Stop 0x5D, Rec 0x5F, etc. → map to mode/bypass as useful.
- **Scribble strips / 7-seg** (the little displays) are **optional** — they're
  sysex; skip for v1, note as a later nicety.

The **feedback path is the whole point of the QCon** — on each 20 Hz tick, push
fader positions and mute/select LEDs so the surface mirrors the DSP. Without it the
motors are dead.

> HUI is an alternative protocol if MCU is awkward, but **MCU is simpler and the
> QCon speaks it** — do MCU.

## 6. CLI / wiring (`main.rs`)

- `--control-surface mk3|qcon|none` (repeatable, so both can run at once).
- `--midi-in "<port name>"` / `--midi-out "<port name>"` (QCon needs out).
- `--list-midi` → print available MIDI ports and exit (so the user can find names
  once the device is plugged in).
- Spawn the control-surface runner thread only when a surface is selected; it
  degrades gracefully if the named port isn't present (log + continue) — so the
  app runs fine with **no surface connected** (the dev case).

## 7. Testing WITHOUT the hardware (required)

This is how we validate before any unit is plugged in:
- **Unit-test `on_midi`** with synthetic byte slices → assert the emitted
  `SurfaceCommand`s. E.g. MK3 knob: `on_midi(&[0xB0, 16, 64])` → `DspParam` with
  the mapped param at ~50%. QCon fader: `on_midi(&[0xE0, 0x00, 0x40])` → mid-level.
  V-pot increment, mute note, transport — all byte-level tests.
- **Unit-test `feedback`**: given a `SurfaceState`, assert the outbound Pitch-Bend
  / Note bytes (fader to -6 dB → specific 14-bit value; muted channel → Note On
  0x7F).
- **Round-trip test**: state → feedback → on_midi → command, where sensible.
- **Manual (optional, no real device):** a macOS **IAC virtual MIDI bus** lets you
  send test messages from any MIDI utility into DeskDSP to watch mappings fire —
  still no MK3/QCon needed.
- Keep all parsing **pure** (bytes in, commands out) so none of it needs real MIDI
  I/O to test. The `midir` connection is a thin shell around the pure mapper.

## 8. RT-safety / discipline (unchanged rules)

- MIDI thread → existing command channel → forwarder. **Never** the audio thread.
- No allocation concerns on the audio side (this never touches it).
- Mapping tables are built once at startup.

## 9. Build order

1. `src/control/mod.rs` — trait, `SurfaceCommand`, `SurfaceState`, the runner that
   bridges to the existing command channel. Add `midir`.
2. `src/control/mk3.rs` + **pure unit tests** (no hardware). Ship this first — it's
   input-only and simpler.
3. `src/control/qcon.rs` inbound (faders/V-pots/buttons) + unit tests.
4. QCon **feedback** (motors/LEDs) from the 20 Hz state + unit tests.
5. `main.rs` CLI flags + `--list-midi`. Graceful no-device behavior.
6. (optional later) QCon scribble-strip sysex.

Everything above compiles, tests, and merges with **nothing plugged in**. The user
connects the MK3/QCon later and it lights up.
