# Issue 50: continuous deck filter sweep

The deck filter is transparent throughout `[0.47, 0.53]`. Moving left lowers the
low-pass cutoff toward 60 Hz; moving right independently raises the high-pass
cutoff toward 8 kHz. The first fifth of each active half eases out of bypass with
a smoothstep coefficient. A full-range control jump slews over at most 5 ms,
independent of callback size, and returns to the exact target value.

This uses two nonresonant one-pole stages per channel (12 dB/octave asymptotic
slope), dedicated to decks. The shared synth/FX SVF is unchanged. Its existing
upper cutoff clamp cannot provide an open low-pass at bypass, and mixing that
resonant filter with dry audio introduced measurable phase-cancellation dips in
an otherwise descending sweep. The dedicated coefficient curves avoid those dips:
LP retention grows monotonically from zero; HP low-frequency subtraction grows
monotonically from zero. Near center the HP subtraction weight also retires stored
DC smoothly, before history is cleared at exact bypass.

Each deck/channel owns its two history values. Source jumps and sample-rate
changes clear both channels; sample-rate changes preserve the control target.
Neutral bypass clears old branch history so changing sides cannot replay it.

## Validation

Four new test groups use actual deck commands/rendering plus independent scalar
references at 44.1, 48 and 96 kHz:

- A ten-tone broadband fixture, with different channel phases, visits 41 positions
  per active half on both decks. DFT energy in the 5–16 kHz band decreases
  monotonically leftward; energy in the 65–550 Hz band decreases monotonically
  rightward. Both rejected-band endpoint energies fall below 0.0001 of dry
  (more than 40 dB attenuation).
- Five positions across the deadband match rendered dry audio sample-for-sample.
  Positions 0.00001 on either side of each threshold differ from dry by less than
  0.000002 sample amplitude.
- Full control jumps and direction reversals settle within 5 ms. A stereo DC
  fixture bounds individual sample changes below 0.025 full scale; returning to
  center restores the exact source level and clears old history.
- Impulse and seeded pseudorandom samples match separate scalar references.
  Neutral processor output is bit-identical to each channel's input.

Existing independent stereo impulse/sweep and seek/rate reset tests are updated
to reference the dedicated processor. This measures rendered response and control
continuity; physical controller feel and listening QA remain pending.

Local `cargo test` passed all 266 tests, and `cargo build` passed.
