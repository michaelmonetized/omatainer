# Issue 57: cached mixer gains and finite transitions

The callback prepares crossfader and per-track pan/gain pairs once at each block
boundary. Unchanged control keys reuse their exact pair; sample processing only
reads the pair and advances an active fixed-size ramp. The original power-law
crossfader and square-root pan formulas remain unchanged.

After initialization, a changed pair ramps linearly from its current audible
value to the exact target over 5 ms, rounded to the nearest output sample. A
mid-ramp edit retargets from the current value. Repeated block preparation cannot
restart the ramp, and zero/end values become exact at its final sample. Muted
tracks advance their ramps. Stopped sample-rate replacement resets ramps so the
new-rate callback starts at the current target.

## Validation

Five new test groups cover:

- Bit-exact static coefficients at 257 crossfader positions for three curves and
  257 pan positions for six gains. Unchanged keys calculate once.
- Transition/reversal step bounds, no overshoot, exact endpoints and unchanged
  block preparation at 44.1, 48 and 96 kHz.
- Bit-exact complete renderer output versus a test-only previous-math path for
  1, 4 and 8 active tracks plus two active decks across 8192 frames per case.
- Actual renderer control sweeps with different callback partitions, warmed
  callback zero allocations/frees during sweeps, and sample-rate reset behavior.
- Nine alternating local timing runs per fixed track count, rendering 32,768
  frames in 128-frame callbacks. Each track has one held synth voice; two long
  constant decks stay active. These are full headless renderer elapsed times,
  not isolated mathematical-instruction timings, desktop FPS or device latency.

The previous-math test path evaluates the original power/square-root formulas
inside the real sample loop; production has no mode switch. The existing pad
release-routing regression now permits the deliberate 5 ms gain fade and then
requires exact silence, preserving its original destination/lifetime checks.

| Active tracks | Previous median | Cached median |
| --- | --- | --- |
| 1 | 9.997 ms | 9.628 ms |
| 4 | 17.010 ms | 17.201 ms |
| 8 | 27.830 ms | 27.327 ms |

Full-renderer timing differences are small and mixed: the four-track run was
slightly slower. These local measurements do not establish a universal speedup.
The deterministic improvement is removal of repeated powers/square roots for
unchanged controls, with bounded smooth transitions and identical static audio.

Final validation: **267 tests passed**, `cargo build` passed, new modules pass
`rustfmt --check`, and `git diff --check` passes. The reported focused timing run
used dev opt-level 1 and `--test-threads=1`. Root read-only review found no blocker.
