# Issue 18: quantized clip launch boundaries

This layer builds on issue 14's pending launch gate and absolute one-shot end.
It gives a scene batch one captured start time, treats tiny clock rounding at an
exact grid as that grid, and applies the MIDI scheduler's half-open sample
interval to arpeggiator steps and loop-local time. Before this last adjustment,
the new regression counted five arpeggiator onsets in a one-beat one-shot that
must have four: the final sample replayed beat zero.

Snapshots now expose `clip_pending`. Pending progress is exactly zero, and the
sequencer labels the incoming clip `queued` until its first rendered sample.
After launch, progress follows the rendered clip-local position. Incoming clips
produce no events while pending; existing release envelopes, drum one-shots,
effect tails, decks, and live inputs remain independent sources.

Validation:

- `cargo test`: 70 tests passed on this isolated issue-14 prerequisite checkout.
- The six new launch tests include 144 combinations: 44.1/48 kHz, 60/123/180 BPM,
  0.25/0.5/1/4-beat launch grids, synth/drum/arpeggiator, looping/one-shot.
- Every pending sample is silent with zero onsets, and the expected first sample
  is checked against the analytical frame distance. The first onset occurs once,
  a complete one-beat one-shot ends without replay, and a loop restarts once.
- Tests also cover exact-grid roundoff, scene/add/restart batches from running
  and stopped off-grid transport, queued replacement/cancellation, retriggering,
  pending/active/stopped snapshots, and actual interleaved callback output.
- `cargo build` and `git diff --check` passed.

Synth onset counting is test-only instrumentation of the actual Poly trigger
path; drum onset counting observes newly advanced one-shot positions. The
callback test unloads independent deck sources to isolate clip output. This is
local sample-level and build validation, not hardware or visual UI QA. Audio
clip playback remains outside this test scope; the covered audible clip paths
are the engine's existing MIDI synth, drum, and arpeggiator paths.
