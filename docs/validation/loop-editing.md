# Precise loop editing

Tracking: [#190](https://github.com/michaelmonetized/omatainer/issues/190),
in the consolidated [PR #510](https://github.com/michaelmonetized/omatainer/pull/510).

Expand **Loop editor** below either deck platter. Source-second fields and
one-frame nudge buttons edit the two edges independently. The main waveform
continues to display the applied loop region. Length and movement selectors
offer 1/8, 1/4, 1/2, 1, 2, 4, 8, 16, 32 and 64 quarter-note beats. Selecting a
size changes a local draft; **Set loop length** prepares a region without
starting playback or enabling the loop. **Enable loop** and **Disable loop**
use the existing performance control. Native Reloop retains a prepared region.

Moves retain the region's musical length and the active playhead's relative
beat position. The current manual grid defines source time through all tempo
anchors; unprepared media uses its immutable source BPM. Track boundaries move
the complete region into range. A length larger than the entire track is
refused. Exact boundary edits require ordered, finite source times within the
track and at least 64 source frames. Cues, pitch and transport ownership remain
with their existing owners. Held roll/slice work refuses a competing edit.

Each request carries the renderer's media identity. A queued request for replaced
media refuses before history capture or mutation. The typed API exposes
`media_key`, `source_sample_rate` and the applied `loop_region` in deck state;
`loop_bounds`, `loop_move` and `loop_length` are immediate deck controls.
Media keys are canonical decimal strings, including keys beyond JavaScript's
exact-number range. Invalid values and unknown fields refuse admission;
stale or unusable source regions complete as rejected jobs.

Applied regions use the existing Undo, controller bank and preparation paths.
Preparation stores source seconds, so a reopened region keeps its time at a
different source sample rate. Size choices are native draft settings for this
launch. No project or preference schema changes are needed.

The seven new checks and surrounding regression modules pass; the current
source-bound result is in the [qualification receipt](loop-editing-receipt.json).
The complete 1786-test software batch passes (43 ignored cases).
Source-qualified ARM64 release `0.1.0+f1ff6e5d09d8` is installed; the running executable matches SHA-256 `e85b8820d2f18f614eb06e749b688eeb88fc3519bb7444b437151c1c897ff784`. Both decks and transport are paused, master is muted, and the independent status follower remains alive. The engine fixtures
compare independent stereo
rendering at three source and three output rates, tempo-anchor moves, exact
edges, all preset lengths, file limits, stale identities, owner conflicts,
repeated edits, transition continuity, Undo and preparation roundtrips. Native
headless UI fixtures exercise deck-specific buttons and waveform markers. Typed
IPC jobs exercise applied/rejected outcomes. No new physical captures or
listening tests are part of this software work.
