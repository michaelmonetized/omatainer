# Issue 89: contextual offline help and observed lessons

The application now has an ordinary Help button and an offline **Help and
lessons** window. Its typed control catalogue is attached at the actual widget
handlers: hover text, accessibility descriptions, focused-control context,
physical units, and implemented boundaries all use the same definitions.
Disabled controls retain explanations. Text help paints no input-owning Area or
widget, so a long tooltip cannot steal a neighboring pad press. Rich crate, platter, and pad tooltips
compose their existing dynamic information with the reference text; overlapping
viewport regions provide semantic context without a competing tooltip.

The eight guides cover output setup, held-note recording, clip-gain editing and
Undo/Redo, track mixing, DJ preparation, controller checks, separate session/deck
stopping, and native-project Save As/New/Open. Starting or cancelling a guide
never submits a musical command or replaces a document. Each Next button waits
for observed renderer, receipt, history, or project state. Queue admission and
elapsed time cannot complete a step. The original clip/deck/receipt identity is
retained; changing the document invalidates a non-project guide. Recording
completion uses two scalar snapshot fields (`note_count`, `recording_held`),
not access to live callback-owned note vectors.

Save As requires an unchanged project epoch and a successfully saved new current
path; opening an existing project does not satisfy it. DJ cue comparison starts
with the newly accepted receipt's coherent preparation, even before its GUI
snapshot arrives, so restored preparation cannot masquerade
as a new cue edit. Hardware/listening steps explicitly require self-reported
physical confirmation in addition to any stated software evidence. No guide
certifies a controller, output latency, clock synchronization, or audio quality.

`docs/manual.md` is generated from the same topics, lesson instructions, control
catalogue, and canonical default bindings. A golden test prevents drift. The
in-app shortcut reference instead shows the active preference profile, including
remappings. F1 remains available while typing only when the active binding maps
F1 to Help; text and musical input remain intact. Other remapped help keys obey
the existing text-focus guard. Help visibility uses the existing persisted view
field; guide progress itself is session-local and does not modify a project.

## Local validation

- Twelve new actual App/CommandPort/RtEngine/AccessKit groups cover every guide,
  original-target held/released state, capacity rejection with live monitoring,
  failed/cancelled loads and saves, restored cue baselines, replacement identity,
  real Undo/Redo, private atomic Save As/New/Open, remapped shortcuts, disabled
  controls, focused units, and manual synchronization.
- Four dialog annotation groups exercise project decisions, disabled history
  controls, diagnostics/library actions, offline license notices, and per-deck
  retry/dismiss identities. Existing precise playback tooltip tests now require
  both dynamic time information and contextual reference text.
- The extended `scripts/check-accessibility.py` fixture drives the actual native
  Linux AT-SPI tree on private D-Bus/XDG roots. It reads pitch units and keyboard
  instructions, completes Arm → held note → release → disarm → launch through
  the real Actions menu, and verifies the held stage cannot satisfy release. It
  also retains the prior project persistence and preference round-trip workflow.
- Final local `cargo test`: **629 passed, 0 failed, 8 ignored** (637 discovered).
  The ignored cases are explicit native/benchmark/manual-generation opt-ins; the
  Linux AT-SPI case was run separately with the expanded script and passed.
  `cargo build`, `git diff --check`, Python syntax, and refreshed license-record
  verification passed. Snapshot allocator, input ownership, and accessibility
  regressions remain part of the suite; no physical audio or MIDI devices opened.

The native fixture is evidence for the application's Linux accessibility API,
not a window-rendering, Orca, physical controller, or listening QA result. MIDI
Learn, external clock synchronization/output, timeline/piano-roll editing,
audio recording, warp, automation, and third-party hosting are not taught as
implemented features. The separate clip-gain idle-rounding regression discovered
by the editing guide is fixed in the preceding accessibility layer.

The assembled stack also passed the native package check and retained the existing preferences close/startup guards, effective remapped Undo/Redo menu hints, and library CloseGuard/cancellation ordering. The private native AT-SPI run visited 230 initial nodes and completed 75 actions through project, preferences, and the recording lesson.
