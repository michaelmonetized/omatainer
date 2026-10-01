# Issue 61: SVF frequency-domain cutoff limit

The shared trapezoidal state-variable filter clamps cutoff in Hz before computing
`tan(pi * cutoff / sample_rate)`. Its supported range is the retained numerical
floor `48000 * 0.0001 / pi` (about 1.53 Hz) through `0.45 * sample_rate`, or 90%
of Nyquist. Requested frequencies below/above this range saturate at its bounds.
The engine supplies a finite positive output rate and finite controls; resonance
retains its existing 0..0.95 clamp and LP/BP/HP morph is unchanged.

Issue 21 preserved the old upper limit in Hz across output rates. That limit was
itself erroneous: a 0.45-radian cap flattened requests above roughly 6.875 kHz.
This explicitly corrects that behavior. A supported cutoff now retains its
requested Hz value across rates, and the upper supported range reaches 19.845,
21.6 and 43.2 kHz at 44.1, 48 and 96 kHz. The issue 21 frequency regression now
checks an actual requested 18 kHz cutoff rather than perpetuating the old cap.
Decks use the separate issue 50 filter; this change applies to shared synth/FX
SVFs and intentionally changes their previously flattened upper responses.

The independent numerical reference solves the two integrator nodal equations
in double precision, rather than reusing the production coefficient expansion.
The equations and prewarping are documented in Andrew Simper's
[Cytomic derivation](https://cytomic.com/files/dsp/SvfLinearTrapOptimised2.pdf),
equations (2), (3), (4), and the implementation on page 6.

Three new test groups plus the revised cross-rate regression cover:

- Impulses and swept sines through LP, BP, HP and intermediate morphs, three
  resonance settings, seven cutoff requests and five rates from 8 to 192 kHz.
  The measured maximum absolute difference from the double-precision reference
  is 0.00000511, below the 0.00002 sample-amplitude bound.
- Settled sine measurements compared with the analytical bilinear transfer
  response at five probe frequencies, six cutoffs and both resonance limits.
  Absolute amplitude error stays below 0.0002. At 48 kHz, LP amplitude at 10 kHz
  for requested cutoffs 3/6/6.875/8.2/12/18 kHz is respectively
  0.062968/0.225646/0.283792/0.375448/0.629410/0.908248. Upper controls therefore
  produce distinct responses. Equivalent measurements run at 44.1 and 96 kHz.
- Negative, zero, minimum, high and maximum finite cutoff values, resonance
  below/inside/above its clamp, five rates and all three filter outputs. The
  output matches explicitly clamped controls bit-for-bit, stays finite, and
  stored impulse history decays.

These are numerical renderer checks, not physical listening or controller QA.

Local validation: all 293 Rust tests and the production build pass. The new
module passes rustfmt, the diff passes whitespace checks, and independent source
review found no blocker. This preparation includes the issue 50 deck processor.
