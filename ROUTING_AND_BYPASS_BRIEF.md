# Brief: route everything through DeskDSP + global bypass

Two asks from the user: (1) ALL audio (incl. system audio like YouTube) should
pass through DeskDSP so it's processed and metered; (2) a global DSP BYPASS.

## Part A — UI: swap in tablet_v3.html
`tablet_v3.html` (repo root) is the new 3D/photoreal console UI. It already
includes a header **BYPASS** toggle that sends `{"type":"set_global_bypass",
"enabled":true/false}` and reflects a `global_bypass` field from `state_sync`.
Replace the `TABLET_TOUCH_HTML` constant in `src/remote/assets.rs` with its
contents, rebuild. (Same drop-in process as v2.)

## Part B — Routing: make system audio flow through DeskDSP (macOS)
Today the meters sit at -inf for YouTube because DeskDSP only captures the Zen
Go's mic/line INPUTS. System audio (YouTube, DAW, etc.) is computer-playback and
never enters DeskDSP. To route "everything" through the DSP on macOS:

1. Install a virtual audio device — **BlackHole 2ch** (`brew install blackhole-2ch`).
2. Set the Mac's **system output = BlackHole** (so all app audio goes there).
3. Run DeskDSP with **input = BlackHole, output = Zen Go**. Add CLI flags so the
   devices are selectable instead of always "default":
   `--input-device "BlackHole 2ch"` and `--output-device "Zen Go"`.
   In `src/audio.rs`, match the requested device names in
   `host.input_devices()` / `host.output_devices()` instead of only matching
   "Zen Go" / falling back to default.
4. Result: YouTube/any app -> BlackHole -> DeskDSP (strip+master) -> Zen Go ->
   speakers. The PROGRAM meters now move and the DSP processes everything.
   (To also blend a live mic through the same chain you'd use an Aggregate
   Device combining BlackHole + the Zen Go inputs — do that as a follow-up only
   if the user asks; the system-audio path above is the core request.)

Document the BlackHole setup steps for the user; it's a one-time system config,
not just code.

## Part C — Global bypass (engine)
Add an RT-safe global bypass so BYPASS = clean passthrough (no processing):
- Add an `Arc<AtomicBool> global_bypass` shared with the audio callbacks
  (store it next to `AudioMeters`, same pattern — NO locks).
- In the input callback: when bypass is true, skip `cs1/cs2.process(...)` and
  push the raw sample into the ring. In the output callback: when bypass is
  true, skip `master.process_stereo(...)` and pass `(in_l,in_r)` straight out.
  Keep it branch-only in the callback; no allocation, no lock.
- Add a WebSocket message `set_global_bypass { enabled }` in `src/remote/mod.rs`
  that stores to the atomic (Relaxed). Include `global_bypass` in the
  `state_sync` broadcast so the tablet reflects the real state.
- Add a test asserting bypass path is bit-identical passthrough (in == out).

## Done =
System audio plays through DeskDSP (meters move, processing applies); the tablet
BYPASS button toggles clean passthrough and the badge/label reflects it; no
locks or allocations added to the audio callback. Push to main; hand back for review.
