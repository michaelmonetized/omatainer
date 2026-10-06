# Per-deck quantization

Tracking: [#188](https://github.com/michaelmonetized/omatainer/issues/188),
in the consolidated [PR #510](https://github.com/michaelmonetized/omatainer/pull/510).

Each deck's **Q** button independently enables cue and loop quantization.
The division selector beneath its waveform offers 1/8, 1/4, 1/2, 1, 2 and
4 quarter-note beats. The current manual grid supplies the downbeat and every
tempo segment. Without a manual grid, the panel explicitly identifies the
source-BPM fallback. Deck settings are transient; session launch quantization
keeps its own controls. Until a deck setting is chosen, older loop commands
retain their session defaults and cue gestures retain their original timing.

Main and hot cue creation round source positions to the chosen division.
Triggering a saved cue rounds its destination through the current grid without
rewriting the saved source point. Main Cue stops and returns immediately.
During forward playback, hot-cue triggers, loop in/out and reloop can wait for
the next local division. A renderer-confirmed label shows the queued action,
musical beat and source seconds. The renderer dispatches before the first due
output sample, through the ordinary history and DSP transition paths.
Paused gestures apply immediately. If no later boundary fits before file end
or the current loop wraps, the gesture applies immediately with normal
destination rounding. Precise source-second loop-editor edits stay exact.

Each deck retains at most one fixed pending action; rapid retriggers replace
it. A held cue captures its exact input/button owner. Releasing that owner
before onset cancels its action, while releasing another owner leaves it
queued. Stop, safety, media replacement, grid/Undo changes, seeking, vinyl
contact, reverse/bleep and temporary pad loops retire competing pending work.
Input resets, mapping changes and output-routing changes also invalidate
captured epochs. Input retirement advances the shared epoch before reserved
releases are submitted, so a full command queue cannot keep a stale onset
alive. This conservatively cancels deferred native work on either deck too.
Scratch and jog motion use their existing immediate paths.

The typed API exposes `quantize`, `quantize_division` and optional `pending`
in each deck's `controls`. `deck_control` accepts
`{"op":"quantize","enabled":true,"division":3}` for one-beat quantization.
Its job acknowledges an applied controller gesture; a deferred musical onset
has its own published pending state and may subsequently dispatch or cancel.
Unknown fields, invalid divisions and invalid value types refuse admission.

The eight new software checks pass; their source-bound result is in the
[qualification receipt](deck-quantization-receipt.json). The complete 1786-test software batch passes
(43 ignored acceptance/maintainer cases). Source-qualified ARM64 release `0.1.0+f1ff6e5d09d8` is installed; the running executable matches SHA-256 `e85b8820d2f18f614eb06e749b688eeb88fc3519bb7444b437151c1c897ff784`. Both decks and transport are paused, master is muted, and the independent status follower remains alive. The independent source-time fixtures cover
tempo anchors, click onsets at three source and three output rates,
tempo-synchronized playback, rapid replacements, independent held owners,
release before onset, input retirement under queue overload, grid history,
stop/scratch/media changes, cue creation, manual loop boundaries and exact
loop editing. Native headless UI and real IPC fixtures exercise deck isolation,
division selection and pending state. New physical captures and listening
checks remain closed.
