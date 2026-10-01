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

### Final assembled release gate

The published layer is based on #95 `ce771d6`. Its independent assembled ordinary
suite passed **708 tests / 12 ignored** in 17.08 s. The controlled release build,
mandatory native preflight and all eight fixed workload groups across three fresh
sessions passed on 2026-10-01 06:31:32–06:34:53 UTC; the workload fixture itself ran
134.03 s. The native run visited 234 nodes and exercised 88 actions over 857 App
frames. It persisted/reopened one note and UI scale 1.25 and ended with output
mute still latched after deliberate protection exit.

The source-bound release binary SHA-256 is
`96257c7dd03899a4c4d39a4a15a9989f2983c072f3bbeeafa6fbe255a9280f65`.
`target/performance.json`, its raw samples and log retain full evidence. Independent
report checking and native package verification passed; the package contains that
report. Seven CLI envelope checks, six status/follow protocol groups and five
runtime-isolation groups also passed.

The host was Linux aarch64, Apple M1 Pro (16-inch MacBook Pro, 2021), 10 logical
CPUs, 16,141,549,568 bytes RAM, kernel `7.1.13-3-2-ARCH`, schedutil, SCHED_OTHER/nice 0
with CPUs 0–9 available. Other agent builds/tests were paused during qualification;
unrelated host applications remained running. One-minute load was 3.471 before
and 1.999 after the timed workload. Every #95 budget remained unchanged.

Worst p99 / maximum across the three sessions, in milliseconds:

| Work | p99 | Maximum |
| --- | ---: | ---: |
| Producer callback wall | 2.254 | 8.188 |
| Composer callback wall | 2.439 | 5.902 |
| Live DJ callback wall | 0.452 | 0.513 |
| Hybrid callback wall | 2.842 | 3.127 |
| 50,000-track App frame | 3.634 | 5.648 |
| Multi-input App frame | 4.483 | 6.160 |
| Private IPC roundtrip | 9.099 | 10.344 |
| MIDI worker dispatch | 3.197 | 8.614 |
| Project roundtrip App frame | 5.020 | 5.020 |
| Long recording App frame | 6.240 | 6.240 |
| Long recording render block | 2.675 | 4.007 |

All measured callback allocation/free, rejection and MIDI-drop counters were zero;
exact state/audio checks passed. These fixed workloads preserve their #95 operation
schedule; the separate protected-versus-Studio hybrid regression directly qualifies
the protection policy's audio/state equivalence. Headless App timings are not
compositor FPS, host callback work is not physical driver deadline evidence, and
ten minutes of virtual note recording is not a ten-minute wall-clock set. Real
controller/audio interaction, human screen-reader use and final listening remain
for the user's role-based physical QA.
