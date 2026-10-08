# Track input monitoring

Issue #177 adds In, Auto and Off to the selected track in **Edit session**.
In hears incoming audio routes and suppresses audio clips. Off removes software
input monitoring while leaving clips audible, for direct hardware monitoring.
Auto hears an armed track's input while no clip is sounding or a recording is
active; a pending launch keeps input until its actual onset. Arm controls this
policy independently of the MIDI/pad composition destination. Audio-clip clocks
keep their positions while suppressed. Source changes use fixed five-millisecond
input/clip gain ramps and no callback allocation or deallocation.

Factory and migrated tracks retain the original additive route until a mode is
chosen. Tracks created or duplicated in the Session editor start in Auto,
disarmed. Imported tracks start in Off because selective import excludes external
input routes. This prevents a copied In choice from suppressing clips when its
source is absent. Imports preserve the source document, and Undo/Redo retain the
new defaults. Project-version comparison includes changes in monitoring mode.

Track **Cue** adds its post-effect, pre-channel-gain signal to the separate
headphone PFL bus. Multiple track and deck selections sum before headphone level
and optional split cue. Track gain, mute and solo do not remove this cue tap.
Cue changes keep audience samples bit-identical in both renderer paths. Cue is
transient; Arm and mode participate in saved state, dirty tracking and Undo.

**Audio routing** chooses exact input and record aliases. Raw input recording
remains available with monitoring Off. An unused missing input does not
invalidate a clip-only track recording; a requested missing source remains
silent. Mode changes do not enumerate/open devices or redirect aliases. Existing
input preview, confirmation, buffer limits, cushion, gaps and overflow reporting
remain in force. Input buffers and processing add latency; this path adds no
compensation delay. Full graph compensation is tracked separately in #178.

Typed API actions `track_monitor`, `track_arm` and `track_cue` use stable track
Targets, not current display positions. `track_monitor.mode` accepts `in`, `auto`
or `off`; arm/cue values require booleans. Invalid types, fields, modes, deleted
identities and replaced projects refuse mutation. Paged track state includes
`input.mode`, `armed`, `enabled` and `cue`; enabled reports policy rather than
physical signal availability.

Project state version 16 saves the optional `input_monitor` choice. A missing
field preserves legacy additive behavior. Older version headers cannot conceal
new mode fields; unknown modes are rejected. Retain a document copy for rollback
to an older application.

This batch also fixes ordinary audio clips being skipped unless their track had
an unavailable instrument. Audio clips now render their immutable source through
track processing independently of instrument availability. Controlled input and
clip sources exercise that behavior and monitoring precedence together.

Nine new software checks and the complete unfiltered suite pass: 1806 passed,
43 explicitly ignored cases, no failures. The [source-bound receipt](input-monitoring-receipt.json)
retains exact source/binary digests, the initial failing run, its fixture
corrections and the focused native/API/import checks. Fresh release `0.1.0+0c3d84c1f15e` is installed as the default executable
for its next launch. The independent private GUI was absent at installation;
its follower was preserved. No playback or physical audio route was opened.
The receipt records the exact package manifest and installed executable digests.
No physical input, listening or overdub test is requested by this thread.
Interface/overdub timing acceptance remains open under the current scope.
