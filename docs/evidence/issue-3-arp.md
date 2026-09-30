# Issue #3: arpeggiator chord caching

Validated locally on 2026-09-30 against the initial `cae546c` implementation.

The arpeggiator now borrows the clip's notes and caches a sorted, deduplicated
chord in fixed storage. Rebuilds occur at note starts/ends, loop or launch
boundaries, and after edits to the playing clip. Enabling the arp or returning
from mute/solo masking refreshes the cache. Sixteenth-note advances reuse the
chord. A rest or an edit removing the sounding pitch releases its gate without
introducing an onset between steps.

The enabled arp slot itself previously caused an FX-vector clone every sample.
`FxChain::tick` now borrows its slots while updating the separate DSP fields.
This prerequisite preserves slot order, bypass behavior and numerical DSP.
Issue #10 still owns the broader FX control handoff and bypass/tail policy work;
issue #14 still owns ordinary MIDI scheduling and its per-sample note clone.

## Validation

`cargo test -- --nocapture`: 21 passed. The eight arp regressions cover:

- One chord rebuild across 48,000 rendered frames and eight sixteenth steps;
  subsequent note end and loop wrap each rebuild once.
- Sorted/deduplicated held chords, off-grid rests and release envelopes over
  three loops; the built-in C/E/G and D/F/A fixture's emitted pitches across
  three eight-beat loops.
- `SetNotes` between steps (including clearing the clip and editing a different
  scene), recorded notes and composed pad additions.
- Disabling/re-enabling arp and mute/solo masking across loop boundaries.
- A thread-local counting allocator, with an allocation/free negative control,
  around the actual renderer and an invalidated cache after a note edit.

Allocator results at 48 kHz / 120 BPM:

| Measured path | Allocations | Frees | Requested bytes |
| --- | ---: | ---: | ---: |
| Warmed `RtEngine::process`, existing chord fixture, 1,024 frames | 0 | 0 | 0 |
| `render_track`, 576,000 frames / three eight-beat loops | 0 | 0 | 0 |
| First `render_track` after `SetNotes` invalidates the chord | 0 | 0 | 0 |

The 1,024-frame measurement matches the original issue fixture, which reported
3,072 allocations / 106,496 requested bytes. The longer renderer measurement
includes synth, EQ and track FX processing and crosses chord, rest, step and
loop boundaries. Its observed local elapsed time was about 45 ms; that is a
local renderer timing, not an audio-device underrun or latency measurement.

The short `process` block deliberately does not cross periodic snapshot
publication. Snapshot allocation/locking remains separate work in #25/#59;
these results do not claim the entire callback is allocation-free on every
block. Command application and note-vector replacement also remain outside
the steady-state measurement.

An independent local Rust replay compiled the original `FxChain` from
`git show cae546c:src/engine/fx.rs` and the revised chain against the same DSP
primitives. Empty, all-enabled, all-disabled and alternating-enabled chains
each processed 100,000 stereo frames. Both output channels were bit-identical
for every frame in all four cases.

The existing C6 arp contract now samples every 64 frames instead of only at
the end of each sixteenth. This observes the second note before its actual
0.45-beat gate expires; its original end-of-step observation missed the new
correct rest release.

`git diff --check` passed. Existing unrelated compiler warnings remain. Hardware
controller and audio-device QA have not been performed for this change.
