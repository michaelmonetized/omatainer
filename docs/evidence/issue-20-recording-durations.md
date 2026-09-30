# Issue 20: record held input durations

Both MIDI capture and pad compose/record now pair releases with the original
onset using issue 16's input identity: MIDI connection, channel and pitch, or
physical pad index. Selection changes do not redirect the stored note update.
Overlapping equal pitches from separate inputs retain their own velocity,
onset and length. Retriggering one held input finalizes its previous note before
starting another.

## Timing and lifecycle policy

- A monotonic recording clock integrates `BPM / (60 * sample_rate)` for every
  rendered audio frame, including stopped compose. It follows tempo and sample
  rate changes without wall-clock sleeps, clip wrap or transport rewind.
- Onset positions use issue 19's clip-local cursor. A pending launch is monitor
  only; its later release does not create an onset retroactively.
- The onset is visible immediately, preserving compose contract C5 and the
  existing quarter-beat preview. Release replaces that provisional duration
  with the complete hold. An instantaneous down/up pair has a one-frame minimum
  at onset tempo.
- Holds crossing loop seams retain their complete unwrapped duration, even if
  longer than the clip. Issue 14 repeats the recorded onset each clip cycle;
  overlapping same-pitch copies release only after their final held count ends.
- Transport stop, record-off, explicit track/scene stop, clip replacement or
  restart, and single-shot completion finalize affected captures immediately.
  A later physical release does not extend or reopen those captured notes.
  These recording boundaries do not change issue 16's live voice ownership.
- `SetNotes` cancels captures targeting its replaced list. An old release cannot
  change a new note that happens to occupy the same vector index. Captures in
  other clips continue.
- Pad pitch and velocity are captured at onset. Octave/instrument changes while
  held preserve the original capture destination and pairing; pitch automation
  is not introduced by this change.

Capture bookkeeping uses 256 fixed slots, matching issue 11's maximum admitted
held gate count. The existing clip note vector still grows at note-on; allocation
of recorded/project data is not claimed allocation-free. Timing resolution is
the engine's command application boundary; hardware timestamps are not added.

## Validation

`cargo test recording_duration_tests -- --nocapture`: nine tests pass.

- Short and long holds, overlapping equal pitches across sources/channels,
  repeated same-input retriggers, velocity preservation and zero-velocity off.
- Original clip/pad pitch retention after selection, octave and instrument
  changes; stopped compose at 120 and 60 BPM across 48 kHz to 44.1 kHz changes.
- Stop/record-off/track/scene/restart/replacement/single-shot finalization and
  cancellation of stale `SetNotes` captures; instantaneous and pending gates.
- A synthetic multi-input schedule records 0.75, 1.0, 0.5 and 6.5 beat holds,
  including one- and two-seam crossings. Stored onset, pitch, velocity and length
  are checked, then the recorded notes are replayed through the actual renderer.
  Emitted on/off frame indices match independently specified expected events at
  both 44.1 and 48 kHz, including equal-pitch overlap and the next loop's retrigger.

`cargo test`: 84 tests pass on the isolated branch with issues 14, 16 and 19
included as prerequisites. `cargo build` passes locally.
This is synthetic engine validation; physical MIDI controller QA remains for
the user's final producer/composer/DJ runs. Capture monitoring's single-route and
no-chase policy remains issue 22, as assigned separately.
