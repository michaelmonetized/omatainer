# Issue 53: neutral FX identity and bypass transitions

Empty chains already returned their input after issue 9. This layer extends exact
identity to complete neutral racks: flat EQ returns input while retaining warm
filter histories, the audio side of Arp and unchanged dynamics avoid unnecessary
interpolation, and common mix endpoints return exact dry/wet samples. Spread at
noon returns before accessing delay taps or advancing its buffers.

Every slot now owns a small allocation-free bypass envelope. Its first rendered
sample uses the configured on/off state immediately. Later on/off changes use a
linear processed/dry crossfade lasting `ceil(sample_rate * 0.005)` samples.
Reversal restarts that bounded transition from its current level, so it cannot
jump to the other endpoint. Processor state advances while its fade is audible,
freezes at fully dry output, and resumes during re-enabling. Other slots keep
processing independently. Sample-rate changes reset the fade alongside history.

Common mix=0 still advances enabled processor histories, preserving future mix
changes. This does not alter the time-effects' separate double-interpolation fix
in issue 54 or Spread/Balance's amount-vs-mix policy in issue 60. Parameter edits
retain their existing behavior; this fade specifically covers the on/off switch.

## Validation

Five new production FX tests cover:

- Empty and whole neutral racks are bit-identical to stereo impulses and 16,384
  seeded pseudorandom frames, including negative zero, at 32/44.1/48/96 kHz.
- Neutral and initially disabled Spread leave delay history untouched; intentional
  enabled width produces the expected channel delay.
- Constant asymmetric stereo through Balance verifies the exact linear fade,
  5 ms endpoint, and per-sample bound at all four sample rates.
- Repeated reversals continue from the current level and reach the final target.
- Neutral EQ retains the same filter history as independently processed filters
  when nonneutral gains are subsequently selected.

Existing primitive-reference tests now model the deliberate fade before freezing;
they still verify effect history and unaffected serial slots. Allocation tests
measure the fade, settled bypass, and resume separately and require zero heap
allocations/frees. Listening and physical controller QA remain pending.

Local `cargo test` passed all 271 tests, including 16 focused FX/reference/
allocation tests; `cargo build` and `git diff --check` passed.
