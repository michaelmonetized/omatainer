# Issue 96 — performance protection and controlled recovery

The visible **Performance** bar protects an active show through shared producer
and renderer checks. This is software safety evidence, not qualification of a
Pioneer, Numark, Akai, MIDI keyboard, audio interface, or physical output.

## Operator behavior

- **Enable performance mode** protects playing or touched deck media replacement,
  note replacement, Undo/Redo, rack additions, hot-cue deletion, project New/Open/
  Close, and device reconfiguration. Already accepted commands are checked again
  against the renderer's actual deck state before history capture or mutation.
- Continuous mixer/FX parameters, pitch/jog, transport, clip launches, live input,
  composition and held-note recording remain available. Stopped, untouched deck
  loads remain available. Save/Save copy, bounded diagnostics capture, snapshot
  publication, retirement and durable library/preparation/play-history writes are
  essential and continue. Project Open never clears protection.
- **Safe stop…** requires a deliberate decision. It finalizes captured durations,
  disarms recording/compose, stops session and both decks, and releases all synth
  input owners. Finite sample hits and effect tails continue naturally.
- **Emergency silence…** additionally applies a 2 ms output ramp to latched mute.
  The requested and renderer-applied states are separate. A two-second observation
  reports consecutive frames below −80 dBFS and any nonfinite tail; this cannot
  establish that delayed/bypassed feedback history is empty and never unmutes.
- **Recover inputs…** requires the operator's explicit report that physical keys,
  pads and touches have been released. It reopens admission after pre-stop work
  drains, without resuming playback. This report is not a device-health assertion.
  Emergency mute remains latched, including after deliberately leaving protection.
- **Reset stopped DSP and unmute…** uses the audio-owner worker and existing graph
  handoff, clears histories off the callback, and reopens the same output stopped.
  It is a deliberate action, not an automatic response to quiet samples. Failed
  reset retains mute, including successful rollback. If no owner/output can recover,
  acknowledge inputs and retain mute to Save and deliberately exit/close.
- No emergency shortcut or hardware mapping was invented. Ordinary Session Stop
  retains its session-only semantics. Accessible ordinary buttons expose every
  safety decision; keyboard text/dialog guards remain effective.

Preferences version 4 stores only `startup.performance_mode` (default false;
factory Performance profile true). Versions 1–3 migrate without discarding fields;
old profiles missing the field stay unprotected until explicitly changed. Runtime
emergency state and input acknowledgments are not project or preference content.

## Admission, ownership and background work

A fixed atomic safety mailbox bypasses full command/payload queues and is checked
before the existing 32-command callback batch. Rejected owned commands use the
existing bounded retirement path; callback rejection never destroys their final
Vec/Box/Arc payload. Writer leases and a safety generation close admission/drain/
recovery races. A newer request cannot inherit an older STOPPED publication or
acknowledgment. Raw native MIDI packets carry an input epoch; packets buffered
before or during recovery cannot start old notes after recovery. The native input
callback adds only an atomic read and a fixed scalar field, with no lock/allocation.

Project/device changes acquire an exclusive permit before destructive job admission
and retain it through the actual seal/commit/rollback. This is separate from the
existing CloseGuard: one operation cannot release another's seal. Mode entry
reports **Changing** while such a job or irreversible optional commit is in progress.
Explicit MIDI rediscovery and policy changes use the same protection lifetime.

Optional jobs use 32 preallocated cancellation slots. Scans, imports, theme/font
reloads, BPM analysis, preferences publication and diagnostic file work refuse new
jobs or cancel before commit. Current filesystem/OS reads may finish cooperatively.
An exclusive commit claim plus cancellation and generation rechecks orders each
irreversible publication against mode entry; a committed result stays truthful.
Forced theme IPC returns `protected`, not an accepted-as-applied success. A reload
already installed in egui holds its guard through next-frame acknowledgment.

Catalog imports/scans committed before protection retain their durable data, but
optional row/metadata visibility waits for Studio. Worker-built restricted views
include only prior visible identities and accepted essential updates, including
stale-result capture overlays; same-path replacement and same-identity imports do
not leak optional metadata. All large candidate retirement stays on the worker.
Theme candidates rejected during a quick protection toggle are retried. Explicit
stopped-deck decoding remains useful show work; deferred BPM analysis is marked
unknown with an explanatory warning rather than an invented analyzed tempo.

The mode does not change OS scheduling priority, guarantee real-time scheduling,
or expose an unavailable backend XRUN count. Essential writes are never suspended
indefinitely behind a blanket worker pause.

## IPC

The strict typed schema accepts `performanceMode` with boolean `enabled`,
`safeStop`, `emergencySilence` with `confirm: true`, and `recoverPerformance` with
`inputsReleased: true`. Missing/false/wrong-type confirmations reject without
mutation. Both ordinary status and cached follow frames contain the same bounded
`performance` object; requested/applied safety counters distinguish admission from
execution. These operations do not bypass a disconnected renderer.

## Validation

Local Rust tests cover renderer rechecks with last-owned payload retirement,
full-queue emergency admission, actual held-note finalization, independent input
owners, two-millisecond latched output and bounded/nonfinite tail observation,
producer mutex waiters, raw MIDI packets across recovery, new-stop/old-completion
interleavings, commit cancellation/generation races, private IPC schema/status,
real App decisions and protected Open cancellation while Save remains available.
Audio-owner fixtures exercise pending device permits, reset failure/rollback and
successful explicit unmute. Worker fixtures exercise actual private theme IPC,
blocked resolution, import/scan publication and stale essential metadata. Private
files verify preferences/export cancellation and committed-result preservation.

The hybrid callback regression renders 2,048 × 512 frames (21.845 seconds of
48 kHz source time), mixing two decks while recording two independent same-pitch
inputs and changing faders/pitch/jog. Protected and Studio outputs must be exactly
equal for every f32 sample, all 256 recorded lengths are checked, and the protected
production callback must allocate/free zero objects. It is an exact behavioral
comparison, not a wall-clock deadline or hardware XRUN claim.

`scripts/check-accessibility.py` extends the existing private Linux AT-SPI fixture
through Enable → Safe stop → explicit recovery → Emergency silence → keep-muted
recovery → deliberate exit, preserving the existing private project/help workflow.
The fixture uses private buses/files and does not alter desktop settings or operate
physical hardware. Final command results are recorded with the issue handoff.

### Completed local checks

On the final software stack based on #95 `ce771d6`, `cargo test` passed **708
ordinary tests / 12 existing opt-in tests ignored**. The default parallel harness
passed twice after the rejected-load watch retirement fix; final elapsed time was
28.50 s. The focused MIDI suite passed 60 tests. The offline manual/catalogue golden,
ordinary/follow parity and the full protected hybrid comparison are included.

One earlier development broad run hit the existing calibration fixture's four-second
wait under the normal parallel harness (host load was not isolated/measured). Its
isolated repeat passed in 0.15 s, and the subsequent full runs passed with that wait
unchanged. No deadline or assertion was weakened. These are correctness fixtures;
#95's controlled timing gate remains a separate final assembly step.

The private native AT-SPI run completed **88 real actions**, including the existing
project/preferences/lesson sequence and both safety/recovery paths. It ended with
protection deliberately off, recovery acknowledged, and emergency output mute still
latched. Production build and source/license validation are recorded in the final
issue handoff; no installed desktop, physical audio probe, or controller was changed.
