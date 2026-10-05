# Brief: control DeskDSP processing from the tablet (not just Zen Go hardware)

Goal: the tablet's new PROCESSING view sends channel-strip + master-chain param
changes to the engine. The engine already APPLIES these via
`AudioCommand::SetParam` / `SetTunerScale` — this brief adds the lock-free path
from the web server to the engine's command queue, plus the WS messages.

## Part A — UI
Re-bake the updated `tablet_v3.html` into `TABLET_TOUCH_HTML` (it adds a
CONTROL / PROCESSING tab toggle and the DSP knobs; front-end only).

## Part B — Lock-free server -> engine command path (RT-critical)
The audio thread already drains `input_cmd_producer` / `output_cmd_producer`
(rtrb SPSC) at the top of each callback. Reuse that — DO NOT add any lock or
touch the audio thread.

1. `AudioEngine`: add `pub fn take_command_producers(&mut self) ->
   Option<(rtrb::Producer<AudioCommand>, rtrb::Producer<AudioCommand>)>` that
   moves the two producers out once (store them in `Option`s so this returns
   `Some` the first time, `None` after). Keeps `AudioEngine` otherwise intact so
   the streams stay alive.
2. `main.rs` (remote path): call `take_command_producers()` and hand the pair to
   `TabletRemoteServer::new(...)`.
3. In `mod.rs`, add a **dedicated engine-command forwarder** modeled exactly on
   the existing `tablet-hid-writer` thread: it OWNS both rtrb producers (single
   producer each — invariant preserved) and drains a `std::sync::mpsc` /
   `tokio::mpsc` of `AudioCommand`. For each command, route input-target commands
   to the input producer and master-target to the output producer (same split as
   `AudioEngine::send_command`). `push` is non-blocking; on `Full`, drop and
   optionally count. No locks, no audio-thread contact.
4. `AppState` gets the `Sender<AudioCommand>`. If the engine isn't present
   (hardware-only / no audio), the sender is absent and DSP messages are no-ops.

## Part C — WebSocket messages (add to the RemoteMessage enum)
- `{"type":"set_dsp_param","target":"ch1"|"ch2"|"master","param":"<id>","value":<f32>}`
  Map target -> CommandTarget (ch1=Channel1, ch2=Channel2, master=Master).
  Map `param` (String) to the matching `&'static str` id with a match over the
  KNOWN ids below (reject unknown). Build `AudioCommand::SetParam{target,param_id,value}`
  and send to the forwarder.
- `{"type":"set_tuner_scale","target":"ch1"|"ch2","scale":"<name>"}`
  -> `AudioCommand::SetTunerScale{target, scale}` (map name to the `Scale` enum).

Known param ids the engine already applies (confirm ranges/defaults from the
node fields and seed the UI defaults to match):

| target    | param            | suggested UI range | unit  |
|-----------|------------------|--------------------|-------|
| ch1/ch2   | gate_threshold   | -80 .. 0           | dB    |
| ch1/ch2   | comp_threshold   | -40 .. 0           | dB    |
| ch1/ch2   | comp_ratio       | 1 .. 20            | :1    |
| ch1/ch2   | sat_drive        | 1 .. 5             | x     |
| master    | glue_threshold   | -40 .. 0           | dB    |
| master    | stereo_width     | 0 .. 2             | x     |
| master    | limiter_ceiling  | -12 .. 0           | dBTP  |

Tuner scale: map to the existing `Scale` variants (e.g. Chromatic, Major,
NaturalMinor...). Use the real variant names.

## Part D — State (v1: tablet-authoritative)
DSP params have no physical control, so for v1 the tablet is the source of truth:
seed the knobs to the engine's actual defaults (read them from
`ChannelStrip::new` / `MasterChain::new`) and don't require readback. (v2 could
add current values to `state_sync`.) The LUFS / true-peak readouts already stream
in `state_sync.meters` (integrated_lufs, master_true_peak_dbtp) — surface them on
the PROCESSING page; add them to the meters payload if not already present.

## Done =
Moving a PROCESSING knob on the tablet changes the processing in real time, with
NO lock or allocation added to the audio thread (the forwarder reuses the
existing SPSC producers, single-producer preserved). Add a test that a
set_dsp_param message produces the right AudioCommand. Push to main; hand back
for review — I'll verify the SPSC single-producer invariant and that nothing new
touches the callback.
