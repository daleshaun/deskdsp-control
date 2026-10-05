# Harsh / scratchy playback — IT'S THE DSP CHAIN (bypass is smooth)

UPDATE: BYPASS is smooth, ACTIVE is harsh. So routing / dual-clock / sample
rates are FINE — ignore the earlier clock hypothesis. The harshness is produced
by the DSP processing itself on real program material.

## Context
The user routes system audio (music / YouTube) IN via "Bridge 2-A" and listens
on the Zen Go. That stereo music currently runs through the **vocal CHANNEL
STRIP** (cs1 on L, cs2 on R: HPF -> Gate -> De-Ess -> 4-Band EQ -> Comp ->
TUNER -> Saturation) and then the master chain. A vocal strip on full-range
polyphonic music will sound harsh — above all the **TUNER** (a monophonic pitch
corrector: YIN detect + granular/WSOLA shift) mangles polyphonic music into
scratchy/warbly artifacts, and **Saturation** distorts loud program.

## Isolate (do this and report which node is the offender)
With music playing, disable nodes in the default channel strip one at a time and
listen:
1. Disable the **Tuner** node only -> expect most of the harshness to vanish.
2. Then Saturation, then Gate, then EQ.
Also A/B: bypass the whole channel strip but keep the master chain (glue/EQ/
width/limiter) -> should be smooth and musical. Report findings.

## Fixes
1. **Program vs vocal path.** For program material the vocal strip (especially
   the Tuner) must not be in the path. Options, pick the cleanest:
   - A "source mode" (VOCAL vs PROGRAM): PROGRAM routes input straight to the
     master chain, skipping the vocal strip; VOCAL uses the full strip.
   - OR default the vocal strip nodes (Tuner, Saturation, Gate) to BYPASSED and
     let the user enable them. The Tuner must NOT be on by default.
2. **Per-node bypass on the tablet.** Expose per-effect bypass so the user can
   switch off the Tuner/Sat/Gate individually (the UI modules are ready for a
   bypass control). Needs a WS message + the existing SetBypass/ToggleBypass
   AudioCommand (reference the node by stable position/type, not a brittle
   index).
3. **Tuner safety:** when the Tuner can't find a stable monophonic pitch (low
   YIN confidence / polyphonic input), it must pass audio through CLEAN rather
   than shifting — verify it does; if not, add a confidence gate so it never
   mangles program material.
4. Sanity-check each node for NaN/denormal/overflow on hot stereo input; clamp
   where needed. Saturation on loud input should soft-clip musically, not alias
   harshly (oversample the nonlinearity if it's aliasing).

## Report
Which node(s) cause it, whether master-chain-only is clean, and the chosen
program-vs-vocal routing fix.
