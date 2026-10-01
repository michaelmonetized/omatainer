# Issue 54: one time-effect wet interpolation

Delay, Reverb and Chorus now request fully wet primitive output. The existing
slot interpolation then applies dry * (1 - mix) + wet * mix exactly once.
Mix 0 emits dry input; mix 1 emits the delayed/reverberant/modulated signal;
0.25 and 0.5 give linear wet coefficients rather than squared coefficients.
The processor still receives input and advances at mix 0, so changing mix
reveals the existing history without changing its stored amplitude.

This matches the existing processed-slot law for dynamics, filtering, drive
and EQ. Spread and Balance still have their separate pre-existing amount path;
their ignored slot mix is tracked by issue 60. Master effects use their own
single interpolation and are unchanged here.

Validation on assembled issue 42 prerequisite:

- All 253 local Rust tests and a production build pass.
- Three new test groups compare dry and wet impulse windows at mixes
  0/0.25/0.5/1 for all three time effects at 32/44.1/48/96 kHz. Fully wet output
  has no immediate dry impulse; a nonempty later wet window is required.
- Mix modulation over sustained input and tails matches a fully wet reference
  at every sample. Other processed slot types match the same linear law.
- Existing independent primitive/series/state-isolation references now use the
  corrected fully wet primitive output. Existing callback allocation and stereo
  isolation tests remain in the passing suite.
- `git diff --check` passes. This is deterministic signal evidence, not a
  listening comparison or hardware qualification.
