# Brief (v2): additional engine DSP params for the tablet

Do this AFTER DSP_PARAMS_BRIEF.md (v1) is landed. v1 wires the command path +
WS messages; v2 just adds more `param_id`s the engine understands. No new
transport work — these ride the same `set_dsp_param` message and forwarder.

## Add these param ids to the SetParam match
In `apply_input_command` (channel strip, target ch1/ch2) and
`apply_output_command` (master) in `src/audio.rs`, extend the `SetParam` match
with the ids below, each wiring to the matching node field. VERIFY the node
exposes the field; if a node lacks a setter/public field, add a minimal one on
that node (no behaviour change beyond making the param settable). Keep the same
RT-safe style — plain field writes, no allocation, no locks.

Channel strip (target Channel1 / Channel2):
| param id        | node        | suggested range | unit |
|-----------------|-------------|-----------------|------|
| hpf_freq        | HPF         | 20 .. 300       | Hz   |
| eq_low_gain     | 4-Band EQ   | -12 .. +12      | dB   |
| eq_lmid_gain    | 4-Band EQ   | -12 .. +12      | dB   |
| eq_hmid_gain    | 4-Band EQ   | -12 .. +12      | dB   |
| eq_hi_gain      | 4-Band EQ   | -12 .. +12      | dB   |
| deess_amount    | De-esser    | 0 .. 12         | dB   |
| comp_attack     | Compressor  | 0.1 .. 100      | ms   |
| comp_release    | Compressor  | 10 .. 1000      | ms   |
| tuner_retune    | Tuner       | 0 .. 200        | ms   |

Master (target Master):
| param id          | node       | suggested range | unit |
|-------------------|------------|-----------------|------|
| master_eq_low     | Master EQ  | -12 .. +12      | dB   |
| master_eq_mid     | Master EQ  | -12 .. +12      | dB   |
| master_eq_high    | Master EQ  | -12 .. +12      | dB   |

Notes:
- If a node stores coefficients rather than raw values (e.g. EQ/HPF biquads),
  set the user-facing field and recompute coefficients the same way the node
  does today on construction — off the audio thread is not required since these
  are plain parameter writes applied in the command handler, but keep them
  allocation-free (reuse existing recompute methods; don't rebuild the node).
- Report back the EXACT param ids + real ranges/defaults you wired (some node
  fields may differ from the suggested ranges). I'll build the matching tablet
  UI knobs (EQ section, de-esser, comp attack/release, retune) to those ids.

## Done =
`set_dsp_param` with any of the above ids changes the corresponding processing
in real time, allocation-free, no new locks. Add a test that each new id maps to
the right node field. Push to main with the final id/range list; hand back for
review and I'll add the UI.
