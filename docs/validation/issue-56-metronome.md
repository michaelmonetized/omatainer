# Issue 56: persistent sample-timed metronome

A transport beat now triggers a persistent 20 ms click voice instead of adding
one callback-index-dependent sample. Voice-local oscillator phase, a 1 ms
linear attack and a decay to zero are independent of callback partitioning.
Downbeats use 1200 Hz/amplitude 0.20; other beats use 800 Hz/amplitude 0.12.
The existing master effects, master level and cue blend still process the click.

Scheduling shares the MIDI half-open beat-interval convention: an event exactly
at an interval's end belongs to the following sample. Beat identity is not
stored as u8, so accent/timing cannot wrap at beat 256. Disabling/Stop clears the
voice immediately; enabling/resuming between beats waits until the next beat.
Changed sample rates rebuild/reset the voice; identical rates leave it alone.

Validation on assembled issue44 prerequisite:

- All 265 local Rust tests and production build pass; three new groups.
- At 44.1/48/96 kHz and 73/120/199 BPM, assert trigger sample positions, multiple
  audible samples, exact zero endpoints/gaps, full 20 ms duration and repeatable
  downbeat accents. An additional fixture crosses beat 256.
- Two real renderers produce exactly equal stereo output for the same 100,000
  frames using fixed 64-frame callbacks versus mixed 1/17/127/512/1024 blocks.
- Actual commands verify disabling, Stop, resume, mid-beat enable and rate reset;
  50,000 warmed click ticks allocate/free nothing.
- Peer review found no blocker; git diff --check passes. No physical audio output
  or listening test is claimed.
