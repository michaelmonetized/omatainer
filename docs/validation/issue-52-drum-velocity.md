# Issue 52: independent drum-hit velocity

Drum voices now store normalized velocity and multiply each sample by its own
velocity and captured clip gain. Zero, negative and nonfinite trigger values are
rejected before free-slot selection or voice stealing; values above unity clamp
to unity. Actual MIDI velocity-zero input retains the existing release path and
cannot create a full-volume hit. Finite one-shot tails continue after release.

Arpeggiated drums also preserve note dynamics. The existing boundary-driven
chord cache retains one velocity per pitch: the maximum among currently active,
visible duplicate notes. Its existing note-boundary, edit, visibility and loop
invalidations refresh that value. Synth arpeggiation keeps its existing fixed
velocity. The cache adds 256 bytes per track (2 KiB for eight tracks); the new
voice field fits the existing drum voice's alignment padding on this target.

## Numerical evidence

Four new test groups exercise all six drum families through live command and
MIDI clip scheduling paths at velocities 1, 64 and 127. The rendered absolute
sample sums over 4096 frames have normalized ratios 1/127, 64/127 and 1 within
1e-7. Kick sums are **14.782860, 946.103065 and 1877.423276**, identically for live
and clip paths. These are local numerical render measurements, not loudness
measurements from a physical audio system.

Independent sample-bank references verify overlapping same-pitch hits with
different velocities and clip gains, including offset start times. Full-pool
zero-velocity input cannot replace any existing hit, and note-off preserves
finite tails. Arp tests cover overlapping duplicate strengths, note ends, edits,
visibility suppression, loop rebuilds and unchanged cache reuse between them.

The sample-bank Arc clone loop remains for the separate issue 51 optimization.

Final local validation: **219 tests passed**, `cargo build` passed, modified/new
standalone modules pass `rustfmt --check`, and `git diff --check` passes.
