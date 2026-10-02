# Issue 110: native MIDI piano-roll editing

Issue: https://github.com/michaelmonetized/omatainer/issues/110
Base: final issue-109 source, ff3110e950a649fd9c83022990547361cd9d9c63 (PR465).

## Delivered behavior

Select an empty/MIDI cell and open Piano roll. The actual App renders a pitched,
source-time grid, individual stable note controls, source clip/loop markers and a
virtualized native note list. Drawing, selecting, body/edge drags, rectangular
selection, movement, transposition, duplication, mute and deletion share the same
draft. Straight/triplet grids and Free control pointer and keyboard movement.
Numeric fields independently edit pitch, start, length, velocity, mute and clip /
loop bounds. Existing one-frame or zero-length notes remain editable. Every note
is accessible even outside the painted viewport. Folding includes used pitches,
major/minor scales and existing off-scale notes; named zoom/scroll controls and
pitch/time rulers expose the view. Focused roll keys include arrows,
Shift+Left/Right, Ctrl+A, Ctrl+D, M and Delete.

Apply is one renderer-qualified History transaction. Capture waits run on the
project worker, never the GUI; editable notes do not enter the general snapshot.
The private prepared request validates values and unique IDs on its producer.
Before taking an inverse, the renderer compares captured target, epoch, kind,
name, bars, region and exact notes, and refuses active recording into that cell.
Unrelated mixer/clip gains are retained. Atomic cancellation races renderer
claim; an acknowledgement reports the actual apply/rejection, independently of
later changes. Complete owned requests, including their last document/ack owners,
retire on the existing worker, through both accepted and rejected paths.

Audition uses a distinct preview input and producer gate key through the captured
track, without entering recording. Releases reserve admission and remain safe
during protection/recovery. Focus loss, outside focus and close release it;
physical MIDI gates remain independent. Drum one-shots keep their natural decay.
Close/Escape preserves pending or dirty work until explicit discard/cancellation;
a renderer-owned edit finishes and remains in History.

## Persisted musical data and playback

Engine schema 5 stores per-clip note identity and mute alongside pitch, timing
and velocity, plus optional source-beat clip/loop ranges. New/duplicated/recorded
notes get distinct IDs; moves/value changes/Undo/Redo retain them. Legacy project
IDs migrate deterministically. New fields, including null, are rejected when
advertised as legacy schemas. Existing schema-1/2/3/4 project and recovery fixtures
strip fields unavailable in their declared versions. Absent regions retain the
original clip timing. IDs are scoped by captured project epoch and clip cell.

Explicit regions chase notes crossing clip start, play intro notes once, close
gates at explicit ends and optionally repeat loop coordinates. Muted notes are
excluded from normal and arpeggiated playback. Recording uses the source cursor
and suppresses held/first-pass clip retriggers. ARP visibility distinguishes intro
and repeating passes. A separate compensated MIDI clock prevents accumulated rounding from moving
later explicit-region boundaries one sample early. Legacy clips, metronome and
decks retain the original transport clock and audio reference hashes. Launches
capture matching origins on both clocks, including quantized launches. Capture
translates explicit-region cursor into saved transport coordinates; prepare and
installation restore its clock, and rate changes retain the same cursor.

The existing 8192-note clip and 65536-note project bounds apply before new MIDI
edits/recording mutate persistent work. Explicit clip/loop spans are at least
1/1024 beat and active repeating density is at most 8192 notes per beat, matching
the existing legacy maximum note count at its minimum period. Note ranges remain
0..262144 source quarter-note beats. Empty clips can be made into MIDI; audio
slots are refused. Native project Save/Open embeds and restores supported data;
there is no new external project dependency.

## Workflow evidence

Seven real App/AccessKit/pointer tests passed in 3.14 seconds (ui-v8):
- Create 64 notes spanning 16 bars, revise pitch/length through controls, Apply,
  Save through the actual Project menu, reopen in a fresh App/engine, inspect the
  editor and compare 128 emitted onset/off events, pitches and velocities exactly
  against independently constructed note-data expectations at 48 kHz / 120 BPM.
- Draw/move/resize by pointer, retain selection IDs, resize/transpose by keyboard,
  duplicate with new ID, mute/delete, use triplet and Free/numeric editing.
- Preserve the captured destination and unrelated gain; refuse stale content.
- Cancel a queued Apply via explicit discard after native Close postponed it.
- Stop audition on real window-focus loss without recording or releasing a
  separate physical MIDI gate.
- Drag loop markers, edit clip/loop/view fields and fold scales without hiding
  existing off-scale notes from the accessible list.

The first full suite passed 1120 tests and exposed an outdated legacy recovery
fixture and a parallel sampler reservation conflict. The fixture now represents
its declared old schema accurately; an isolated exact sampler rerun passed.
Aggregate sampler limits remain unchanged. The sequential second run passed 1122
and exposed a new total-note test fixture that counted 46 factory notes in
addition to its intended 65536. Its setup now clears those factory notes before
filling eight clips; bounds/assertions were not relaxed. The final sequential suite passed **1125 tests, 0 failures and 25 opt-in ignored**
in 230.22 seconds. Two additional integration tests retain the shared quantized
launch grid with deliberately distinct clocks, and verify the source cursor plus
remaining note gates after capture, installation and a 48 to 96 kHz rate change.
The first standard gate passed all workload budgets/state/allocation checks but
failed nine original audio hashes after the global clock change. The correction
scopes the compensated clock to explicit regions; no original hashes, performance
budgets or musical assertions were changed. The corrected frozen gate and every final package/protocol check passed, as
recorded below. Current evidence is stored under /home/michael/Projects/omatainer-work/
issue-110-*; it is not a physical hardware or listening acceptance.


## Frozen local qualification

Qualified source: `7580f09308a60c4d97ae4dcc167db6b939909017`, including the
feature source `24aa5022282e0291865064d0842f81e43a14f6c9`. Linux aarch64,
release profile, debug assertions off; no AppImage/loader override, inherited
Cargo incremental override or hosted native build. The controlled gate used the
unchanged policy and CPU 6, from 2026-10-02 03:26:57 UTC to 03:33:41 UTC.

- Full ordinary local suite: **1125 passed, 0 failed, 25 opt-in ignored**, 230.22 s.
- Standard gate: **all eight workloads × three repeats passed**. Producer,
  composer, live-DJ and hybrid callback allocation/free counts remain zero.
  Their four original audio reference hashes are unchanged. State, input,
  crate, recording and project-roundtrip checks all passed.
- Native private Linux AT-SPI: **158 actions, 246 visited nodes, 573 frames**.
  Actual App/renderer controls, baseline project/History, sampler, preferences,
  analysis, library, help and performance recovery paths passed. The seven
  piano-roll workflows separately use actual App/egui/AccessKit and pointer/key
  input. This does not claim a desktop window, Orca or hardware acceptance.
- License inventory update/check: **411 source files**, with retained notices,
  packaged assets and the pinned source supplements. All **8** license/package
  script fixtures passed in 3.509 s.
- Frozen gate recheck, immutable package creation, package verification, real
  CLI/IPC, follow protocol, runtime isolation and safe startup each returned 0.

| Workload | Maximum measured wall time across three repeats |
| --- | --- |
| Producer callback | 0.964 ms |
| Composer callback | 1.282 ms |
| Live-DJ callback | 0.298 ms |
| Hybrid callback | 1.843 ms |
| Large-crate frame | 2.554 ms |
| Controller/IPC frame | 2.686 ms |
| Project roundtrip frame | 4.269 ms |
| Long-note recording frame / renderer | 4.084 / 4.231 ms |

Release executable SHA-256:
`10430043bfcbb1355f48bbd169cd53400a2e3c76fae15eabebfe78465130eec4`.
Controlled test executable SHA-256:
`d21e663f87e7151ad7c7a73aecaf00dda8c2decf8859070f36f8fd3be256c3c1`.

Receipts remain under `/home/michael/Projects/omatainer-work/`: final performance
JSON/raw/log and native JSON, `issue-110-final-post-results.json`, each
`issue-110-final-*.log`, full-v4/UI-v10 logs and immutable
`issue-110-final-package`. Failed earlier gate receipts remain separately in
`issue-110-standard-gate-v1.*`; the latest passing gate did not replace them.
The final validation commit changes documentation only, outside the qualified
source inventory.

## Remaining acceptance

#107's previously recorded supplemental quiet-host wall-max failures remain
unresolved. User producer/composer/live-DJ QA with DDJ-FX, NS7 Mark II, APC40 mkII,
MPD232 and a MIDI keyboard, physical routing/latency/XRUN behavior and Orca remain
unverified. No issue is closed or PR merged by this work.
