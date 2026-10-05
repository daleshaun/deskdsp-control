# URGENT: harsh / clippy / scratchy playback — dual-clock ring underrun

Symptom: playback through DeskDSP (input = "Bridge 2-A", output = "Zen Go") is
harsh, clippy, scratchy — classic buffer under/overrun.

## Root cause (confirmed in code)
`AudioEngine::new` (src/audio.rs ~184-187) does:
- `in_config = in_dev.default_input_config()`  (Bridge's own default rate)
- `out_config = out_dev.default_output_config()` (Zen Go's own default rate)
- builds the DSP at the INPUT rate, runs input and output as TWO independent
  cpal streams joined by a plain rtrb ring — with NO rate check, NO resampling,
  and NO shared clock. Two separate device clocks (a software Bridge + the Zen Go
  hardware) drift and/or differ in rate, so the ring constantly empties
  (`pop().unwrap_or(0.0)` injects silence) or overflows (`let _ = push`) =
  continuous discontinuities = the harsh/scratchy sound.

## Do these
1. **Instrument startup (ship first, it's the smoking gun):** print
   `in_config.sample_rate()`, `out_config.sample_rate()`, both `channels()`, and
   the selected buffer size. Log a loud `⚠️ SAMPLE RATE MISMATCH` if in != out.
   Run on the Mac and report the two rates.
2. **Isolate DSP vs routing:** confirm whether BYPASS is ALSO harsh (it will be,
   if this is the cause). Report.
3. **Primary fix — single clock via one device.** The correct way to bridge a
   software input + hardware output is ONE macOS **Aggregate Device** containing
   both "Pro Tools Audio Bridge 2-A" and "Zen Go", with **Drift Correction**
   enabled on the Zen Go, run with BOTH `--input-device` and `--output-device`
   pointed at that aggregate. Document this clearly for the user; it gives one
   clock and one rate. Verify playback is clean on the aggregate.
4. **Code robustness (so it can't silently glitch):**
   - If `in_rate != out_rate`, do NOT just run — either refuse with a clear
     error naming both rates, OR insert a resampler on the ring so the output
     always has data at its own rate.
   - Build BOTH `ChannelStrip` and `MasterChain` at a single consistent rate
     (today the master chain runs on the output stream but is built at the input
     rate — wrong if they differ).
   - Request a fixed, sane buffer size (`cpal::BufferSize::Fixed`) on both streams
     where supported, and prime the ring (write a few ms of silence) before
     `play()` to avoid startup underrun.
   - Consider drift handling even when rates match (two hardware clocks still
     drift): a slightly larger ring plus dropping/duplicating a sample when the
     fill level crosses high/low watermarks, or a proper async resampler.

## Report back
The printed input vs output sample rates, whether BYPASS is also harsh, and
whether the single Aggregate Device makes it clean.
