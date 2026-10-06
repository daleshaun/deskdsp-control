# Brief: DeskDSP Release-Candidate (RC1) Validation Pass

**For:** antigravity (on-Mac, has the Zen Go + running daemon)
**Goal:** confirm whether DeskDSP RC1 is release-ready. Run a full **code-health
audit** and a **live functional pass on real hardware**, then report a pass/fail
checklist. Fix regressions only; do not add features.

## 0. Execution mode — RUN TO COMPLETION, DO NOT PAUSE

Work this whole pass in one run: audit → test → live-check → fix any regression →
commit/push → report. Do not stop to ask "ready to submit?" or wait for a yes. If
something is ambiguous, pick the sensible default, note it, keep going. Only stop
for a genuine hard blocker (e.g. no hardware to test a given item — then mark that
item SKIPPED and continue). End by pushing any fixes and giving the RC report.

## 1. Code health (automated — no hardware)

- `cargo build --release` → clean, zero warnings.
- `cargo test` → all suites green (report the count).
- `cargo clippy --all-targets` → report any warnings.
- `git status` clean; confirm local `main` == `origin/main` (nothing unpushed).
- Confirm `assert_no_alloc` RT-safety tests pass (audio-thread zero-alloc).

## 2. Live functional pass (THE RELEASE GATE — needs the Zen Go)

Run the release daemon (`--remote-only`, Zen Go in/out) and verify each. Mark
PASS / FAIL / SKIPPED (if hardware for that item isn't present):

1. **Program/music** in PROGRAM mode → clean, no harshness/clipping (the original
   complaint; must stay fixed).
2. **Per-channel presets** → switch Vocal / E.Guitar / Keys / Piano / Program;
   each loads and swaps **without clicks or dropouts** (RT-safe swap).
3. **Mic voicing profiles** (Tier A) → each audibly changes the vocal tone.
4. **Tuner** on a sung/spoken vocal → tracks smoothly, no warble/choppiness
   (the comb-free rewrite).
5. **Guitar chain** (if a guitar/Hi-Z source is available) → amp/cab engage.
6. **Tablet** → connects, meters move, source picker + per-effect bypass work,
   "DSP ACTIVE" reflects state, mic-image IR slot loads a test IR.
7. **MicImage (Tier B)** → load any test transfer IR; confirm no glitch/dropout
   and the audio path stays clean.
8. **Control surfaces** (MK3 / QCon) → if connected, confirm they drive the
   engine (pads/knobs/faders map; QCon motor faders reflect state). If not
   connected → SKIPPED (the parsing is already unit-tested).

## 3. Scope of fixes

- Fix **regressions** only (something that used to work and now doesn't, or a
  real bug surfaced by the pass). Keep fixes minimal; validate before pushing.
- Do **not** add new features or refactor for this pass.
- Commit + push any fixes to `origin/main`.

## 4. Report

Give an **RC checklist** with PASS / FAIL / SKIPPED for every item in §1–§2,
note anything fixed (with commit), and end with a verdict:
- **"RC1 → RELEASE-READY"** if §1 is clean and all testable §2 items pass, or
- **"RC1 blocked"** with the specific failing items if not.

Also note which §2 items were SKIPPED for lack of hardware so the human knows
what still needs a physical check.
