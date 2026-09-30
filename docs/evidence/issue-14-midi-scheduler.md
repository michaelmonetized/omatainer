# Issue #14: event-driven MIDI clip playback

Ordinary MIDI playback now checks the next scheduled boundary instead of
cloning and scanning the clip's note vector for every output sample. Launches
and note edits build a heap of note-on/off times once. Rendering checks its head
in constant time between boundaries; processing a boundary takes logarithmic
heap work. Repeating events reuse the reserved storage, including across loops.

Equal-time note-offs precede note-ons, with source-note order breaking remaining
ties. Same-pitch overlapping notes retrigger at each onset; their shared clip
voice stays held until the final overlapping note ends. A note's full duration
is retained across loops, including durations longer than one loop. Nonfinite
starts/durations and nonpositive durations do not schedule events.

Edits rebuild the active schedule and reconcile its gates at the next output
sample. Note additions preserve existing held gates, including unrendered
gates from other edits in the same command batch. An edit exactly on an off/on
boundary preserves the retrigger and off-before-on order. Replacing the full
note list releases the old clip notes and chases the new active notes. Disabling
the arpeggiator restores the currently active ordinary chord; note additions
while it is enabled preserve its current gate.

The renderer's beat marks the end of the current sample interval. Pending
quantized clips emit nothing and leave their local position unadvanced until
that interval begins after the launch boundary. Ordinary one-shots stop after
their full absolute clip duration, without replaying their first event. This
small launch gate is a prerequisite for the ordinary MIDI acceptance here;
issue #18 covers the complete launch/scene/arp matrix and pending UI state.
Issue #15 separately changes mute/solo scheduling and output gating.

## Local validation

`cargo test -- --nocapture` passed all 64 tests on the stack through issue #10.
The eight new scheduler regressions exercise the real renderer and event gates:

- The default four-clip scene matches an independently expanded/sorted event
  reference for 216,001 frames with 1-, 127- and 1,024-frame blocks.
- Sparse, 256-note dense and overlapping fixtures match that reference for
  240,001 frames with 1-, 257- and 1,024-frame blocks. These include a 6.5-beat
  note in a four-beat loop, distinct-pitch notes and simultaneous off/on events.
- SetNotes, recorded-note additions and pad composition update playback once
  without stale gates. Two edits at the same exact boundary preserve all four
  expected gates in order. A pending one-shot waits 12,000 frames, plays its
  full 24,000-frame beat, then releases without a loop restart.
- Invalid note ranges and nonfinite transport probes produce no event loop.
- A warmed 1,024-frame `RtEngine::process` block with all four default clips
  measures **zero allocations, zero frees and zero requested bytes**, replacing
  the audit's 4,096 allocations / 368,640 requested bytes. That short block does
  not include the separate periodic snapshot publication (#25/#59).
- With one or 4,096 notes, 24,000 `render_track` frames between boundaries visit
  **zero events, perform zero schedule rebuilds, and allocate/free nothing**.
  A further 192,001 frames crossing dense boundaries and loop wraps also allocate
  and free nothing. One local full-suite run took 0.743 ms and 0.536 ms for the
  respective between-boundary measurements; these are elapsed CPU-side test
  observations, not an audio-device latency or underrun claim.

Schedule preparation/edit allocations remain outside the steady-state playback
measurement. The existing counting allocator's positive control and arp/FX
allocation regressions also pass. `cargo build` and `git diff --check` passed. No hardware or live
controller QA is claimed.
