# Performance session history

Issue #106 adds explicit renderer-confirmed performance sessions above final
#105 (a06038b), persistent measured deck activity, manual status and external
entries, and a path-omitting JSON export. Final-source qualification is recorded
below. Earlier checkpoints and failed diagnostic runs remain identified as such.

## Measurement foundation

The source observer uses fixed four-lane attribution of the existing deck
replacement/seek envelope. Process-local monotonic keys distinguish load
receipts from explicitly unresolved raw-media loads; these are not persisted
catalog identities. A bounded failed key registration returns unknown instead
of reusing an identity. A fifth overlapping source records incompleteness.

Four fallibly allocated stereo master-effect lanes retain old-source tails.
Their storage is checked against 96 MiB at the supported 8–384 kHz rates.
Ring-rotation peak envelopes bound future tails in constant time, with
floating-point margins and explicit error envelopes. Retirement below -110 dBFS
cannot change the actual audio, and pool overflow never relabels an old tail.
The error calculation qualifies an isolated linear master contribution removed
at the limiter input; it is not a claim that a differently rounded entire mix
would be bitwise equal to that counterfactual.

Classification uses 10 ms (rounded up to an output sample) RMS windows and a
-90 dBFS floor. Actual output and the source-dependent converted difference
must qualify on the same main channel. Cancellation nulls, silent integer
codes and zero-crossing samples cannot falsely create or truncate track time.
Partial windows retain actual frame counts. Numerical ambiguity, nonfinite
samples and sample-clock exhaustion are explicit outcomes.

The first compile exposed restricted re-export visibility and an untyped test
rate. A later compile exposed the same visibility boundary for the peak helper;
both were corrected. Twelve focused tests then passed in 0.96 seconds in
`issue-106-measurement-v6.log`. Earlier passed checkpoints were five groups
(v2), seven groups (v3) and nine groups (v5). Evidence includes independent
f64 fractional-delay/filter references, unchanged observed f32 processor output,
actual deck replacement/seek attribution, future wet/tempo changes, ring-tail
retirement/reuse, overflow and negotiated-rate storage bounds. The measured
tracker loop performed zero allocations and frees.

## Output callback prototype

Only the actual OutputCallback's final converted destination buffer promotes
observations. A fixed 16,384-frame sidecar captures corresponding isolated
source-removal values; larger blocks explicitly lose measurement coverage while
audio continues. Source observers still advance before session Start, retaining
real pre-existing tails. Direct renderer export and zero-frame offline project
service cannot create output time. Rate changes close partial rational-rate
segments without replacing the observation receiver, and actual DSP/project
resets clear matching observer histories. The callback wall timer includes
observation; the existing renderer-CPU scope still excludes final conversion.

Sixteen focused groups passed in 11.12 seconds, with one opt-in timing probe
ignored (`issue-106-measurement-v9.log`). This includes 250 combinations across
all ten supported sample formats, mono/stereo/4/6/8 channels and signal, zero
gain, crossfader-zero PFL at cue mix 0/1 and anti-phase mono cancellation.
Independent assertions use actual converted output buffers, including zero
additional channels. Warmed measured callbacks had no allocations or frees.
Other cases cover starting while playing, ending partial windows, 48/44.1 kHz
segments, sidecar overflow and disconnected observation consumers.

An opt-in release-profile active-history probe passed 2,048 measured blocks in
each of three existing two-keylock show fixtures. No policy ceiling changed:

| Rate / frames / ratio | Wall p99 / max, ms | Full callback CPU p99, ms |
| --- | --- | --- |
| 44,100 / 128 / 0.50 | 0.733339 / 0.829465 | 0.726416 |
| 48,000 / 128 / 0.84 | 0.766297 / 0.838215 | 0.763167 |
| 96,000 / 128 / 1.50 | 0.756423 / 0.843007 | 0.751834 |

All original workload audio/state assertions passed, with zero measured heap
operations and command rejections. The observation receiver drains between
timed callbacks. This one-repeat, three-case prototype is not the final full
source-bound #95/#100 gate. Raw samples and log are preserved as
`issue-106-history-prototype-timing-v1.*` in the local work evidence directory.

At this early prototype checkpoint, session acknowledgment, timestamps,
persistence and product controls were still pending. They are implemented in
the subsequent sections. These software captures are not physical audible
duration, driver delivery, hardware, listening or XRUN proof.

## Session control and persistent owner checkpoint

The actual renderer accepts one Start/End request at a time. A coherent receipt
contains the actual frame boundary, monotonic-anchored civil time and current
deck load keys. Start requires a nonempty output callback; ordinary offline
project service cannot create performance time. End flushes the partial window.
A coherent progress fence precedes observation consumption, so saving cannot
claim a window whose event is still in flight. Fixed-capacity observation loss
is recorded separately from unavailable backend dropped-buffer information.

One history worker owns the private store, source registrations, classification
consumer and publication retirement. It saves recording progress about once a
second and publishes selected-session snapshots at most four times a second,
with urgent terminal receipts. Automatic recording, End and saving remain
essential during performance protection. Manual marks, external entries and
exports obtain optional-work permits at admission and filesystem commit.
Session IDs are random opaque 128-bit values, and renderer nonces never reuse a
prior session on a surviving graph. Manual actions capture session, entry and
edit revision; marks preserve every measured frame count. External entries are
explicit assertions without a measured deck or duration.

The schema-1 store accepts at most 1,024 sessions, 4,096 entries per session,
64 sample-rate segments per entry and 1,024 UTF-8 bytes per label. Each session
is bounded to 16 MiB; the 256 MiB directory budget includes staging space and
preserved crash staging files. Directories/files are private 0700/0600, owned
by the user, and a lock prevents another writer. Reads and commits check file
identity; corrupt, future-schema, changed and symlinked records remain preserved.
Publication followed by failed directory synchronization is reported committed
with durability unconfirmed, and retry confirms the same candidate. Restart
marks active records interrupted at their last saved prefix. Export uses an
explicit field allowlist and a new-file policy; it omits media locations,
fingerprints, process-local load keys and environment data. Labels themselves
are visible for review. Automatically supplied full filenames are reduced to
basenames rather than copied into exported titles.

The ordinary History panel provides renderer-confirmed boundaries, session
selection, fixed-height virtualized entries, manual status, external-track text
inputs, export and retry. Calendar timestamps are explicitly UTC. Output
telemetry since launch displays late callbacks/backend errors/device losses,
with backend dropped-buffer count unavailable; these global diagnostics are
not attributed to individual session intervals. Normal exit waits for End and
confirmed saving. Keep working cancels exit after pending operations settle;
already ended sessions remain ended. Explicit close without a confirmed save
accepts loss of recent buffered history.

The history suite passed 80 tests in `issue-106-ui-v4.log` and again in v5.
After stale-action scoping and the callback handoff correction below, all 1,002
ordinary tests passed, with 21 opt-in tests ignored, in 25.94 seconds
(`issue-106-full-v2.log`). One hundred successive project-install heap checks
and 20 successive actual history UI workflows passed after the correction.
The UI flow uses real AccessKit actions and keyboard/text events: silent decks,
actual output playback, End/save, manual unplayed preserving measured duration,
external entry, export, existing-destination refusal, stale controls against a
new session, and cancelled exit followed by a new Start. Worker cases also
cover real queue overflow, late catalog qualification after End, an End receipt
surviving graph loss, and a failed End save retaining the actual ended session
until persistence retry succeeds.

A native preflight passed 158 actions, 244 visited nodes and 1,261 App frames
(`issue-106-native-v1.log`) with the original 160-action/70-second bounds.
The child now renders actual converted output callbacks. Native history actions
confirm Start, End, durable save and manual marking without altering duration,
and expose/focus the three text inputs. The pinned Unix adapter does not provide
EditableText mutation; typing/export remain covered by the actual egui event
flow. This early preflight predates the final calendar/scoping changes; the final
source-bound gate below includes a fresh native run.

### Callback handoff defect found during qualification

The first whole-suite run observed one free at a zero-frame project install.
It did not recur across 100 isolated runs. A temporary allocation trace under
concurrent whole-suite execution eventually captured a 48-byte free:
`crossbeam_channel::waker::SyncWaker::notify` → bounded array `try_send` →
`RtEngine::project_tick` at the completed-project reply. The parked receiver's
wake context can become the last owner on the renderer after its worker exits.
The project worker now uses nonblocking reply polling and a one-millisecond
worker sleep; it never registers that parked context on the renderer-written
channel. Cancellation and atomic installation claims retain their existing
semantics. The temporary tracer was removed; the original allocation counter
and zero-free assertion are unchanged. Trace evidence is in
`issue-106-heap-trace-full-v12.log` and `issue-106-heap-trace-symbols.log`.
An earlier traced run also timed out in an existing support/recovery UI fixture;
it is retained as failed evidence rather than presented as a pass.


## Final-source UI and project identity checks

Prepared project media now retains the key of its actual restored load receipt,
so the saved UI identity can qualify that same rendered source. Empty prepared
decks have key zero and create no phantom history track. A renderer-boundary
test covers both loaded receipts and empty decks; the native history fixture
also requires a catalog-associated entry after ordinary project reopen.

History scroll views use the app's native numeric scrollbars and keyboard
Home/End/Page controls. A real private stored 4,096-entry session confirms that
only visible rows are constructed, End reaches entry 4,096, its manual status
changes, and that edit preserves the scroll position. Long labels stay on a
single truncated row. Session identity scopes the viewport; edit revision scopes
captured edit/export controls separately, so stale actions remain retired without
resetting the viewport on every mark. Both actual history UI flows passed, then
all 1,004 ordinary tests passed with 22 opt-in tests ignored in 26.33 seconds
(`issue-106-final-full-v2.log`). Earlier source-bound passing reports remain retained as checkpoints. The final
qualification below also includes first-action admission: two native actions in
one frame cannot replace an already captured End with a later manual edit. The
regression confirms an ended session and unchanged manual overrides.

## Final qualification

The exact final source passed all 1,005 ordinary Rust tests with 22 opt-in
tests ignored in 35.30 seconds (`issue-106-final-full-v3.log`). These include
actual multi-format output classification, renderer mailbox boundaries,
private-store restart/export/error handling, sole-worker lifecycle, restored
project identity and actual AccessKit/keyboard product interactions.

The unchanged source-bound release gate passed all eight groups × three runs
from 2026-10-01 15:30:38.035286 to 15:34:46.765154 UTC. Native Linux AT-SPI
validation passed 158 actions, 244 visited nodes and 1,098 App frames within the
original bounds. It is a private D-Bus App/renderer adapter run without a desktop
window or physical controller. Retained evidence: `issue-106-final-performance`
(JSON, raw JSON and log), `issue-106-final-native.json` and
`issue-106-stack-gate-v4.log` under the local work evidence directory.

| Mode | Callback wall p99 / maximum, ms | Renderer CPU p99, ms |
| --- | --- | --- |
| producer | 0.786380 / 3.810692 | 0.672250 |
| composer | 0.843714 / 3.860484 | 0.677583 |
| live_dj | 0.194585 / 0.376086 | 0.156332 |
| hybrid | 1.076799 / 4.313029 | 0.882208 |

All measured callback heap and rejected-command counts were zero; the original
audio/state goldens passed. The host was the local M1 Pro (10 CPUs, aarch64,
Linux 7.1.13-3-2-ARCH), with load average about 10.32 before and 7.94 after the
fixed workload. No app processes or scheduling priorities were changed.

The first final-source active-history attempt failed a maximum-wall-time check
at 96 kHz / 128 frames / 0.50×: 2.775060 ms exceeded the unchanged 2 × block
deadline ceiling of 2.666666 ms; p99 was 0.787297 ms. Its earlier 48 kHz /
128-frame case also contained a 3.761025 ms single callback above one block
deadline, within the reviewed maximum ceiling. `issue-106-final-active-history-v4.log`
retains the failure. OBS was observed using substantial CPU at investigation;
that observation does not establish the cause of the individual outlier. The
failed run is not qualification evidence. No policy or source was changed.

Release binary SHA-256: `9c7de27aed73c36da5ff1f370bfb184bb38183a4aff1f2db74f1881384967082`.
Release test binary SHA-256: `75dceb40842bef0a1b9711b9876cf366495f307d96856539f10453136a07651e`.

A second unchanged, unpinned attempt (`issue-106-final-active-history-v5.log`)
also failed its maximum-wall check at 96 kHz / 128 frames / 1.50×: p99
0.802006 ms, maximum 3.785942 ms versus the same 2.666666 ms ceiling. A
subsequent diagnostic uses test-process affinity to CPU 6 (kernel capacity
1024, versus 485 for the two efficiency CPUs), without changing nice level,
scheduler, source, limits or user processes. Its host conditions and executable
bindings are retained separately in `issue-106-final-active-history-v6-host.json`.

The real four-source persistence-worker probe passed all three repetitions in
3.90 seconds on the same final binary, using the original inherited CPU affinity.
At 96 kHz / 128 frames, two current and two retiring sources all accumulated
positive measured duration, followed by actual End and confirmed durable saving;
there was no incomplete measurement or lost observation. Worst wall p99/max were
0.705671 / 1.980888 ms, renderer CPU p99 0.686875 ms and full callback-thread CPU
p99 0.699541 ms. All measured callbacks performed zero allocations/frees. Two
individual callbacks exceeded one block deadline, while passing the unchanged
reviewed maximum ceiling. The persistence worker runs through its normal
autosave policy; individual filesystem synchronization calls were not separately
time-stamped. Reports: `issue-106-final-worker-history-v4.json` and `.log`.

The CPU-6 active-history matrix passed all 18 rate/frame/ratio configurations ×
three repetitions, with 2,048 measured blocks each, in 87.87 seconds. All
original state/audio checks passed; heap/free/rejected-command counters were
zero. Worst wall p99 across configurations was 1.277009 ms, maximum
5.964332 ms; renderer CPU p99 was 1.038459 ms and full callback-thread CPU
p99 1.236917 ms. Limits are evaluated per configuration; two individual wall
samples exceeded one block deadline, within its reviewed maximum ceiling.
Reports: `issue-106-final-active-history-v6.json`, `.log` and `-host.json`. This
is qualification under the recorded CPU affinity, not a claim that the two
unpinned failures disappeared or that an arbitrary busy host meets every deadline.

The existing #100 two-keylock show gate also passed all 18 groups × three
repetitions on this final test binary and CPU-6 affinity, with unchanged policy
and source bindings. The complete retained output is
`target/keylock-quality/issue-106-final-show-v4/{verified-showload.json,raw.json,execution.log}`;
the launcher log is `issue-106-final-keylock-show-v4.log`. Neither show suite
substitutes for the prior matched listening corpus or new human listening.

The immutable `issue-106-final-package` was built from committed source
807f44d and independently verified with the source-bound workload receipt,
embedded license records and package checksums. Independent source/binary-bound
gate verification passed. Seven CLI protocol,
six follow protocol, five runtime-isolation groups and actual headless safe
startup all passed against the final release executable. Their logs are
`issue-106-final-{cli,follow,runtime,safe-start}.log`.

## Remaining user QA

The digital classifier is explicitly final-buffer source activity above its
window threshold. It does not measure sound pressure, prove driver/device
delivery, certify zero XRUNs, or judge musical listening quality. The supported
format/channel capture tests include silent/cue-only loads, crossfader exclusion,
mono cancellation and additional-channel silence. Manual marks and external
entries are assertions and never manufacture measured output duration. Global
backend telemetry since launch is not a session-specific delivery audit.

During the final physical producer/composer/live-DJ runs, start History with
silent and cued decks, bring each into the main mix and crossfade replacements
with master effect tails. Compare session boundaries, identities and duration
with the actual multichannel recording, then End, restart, mark an entry and
export to a new file. Review manually entered title/artist labels before sharing.
The Pioneer DDJ-FX, Numark NS7 Mark II, APC40 mkII, MPD232 and MIDI keyboard
paths remain the user's final acceptance context; this PR does not claim those
physical controller, capture or listening runs.
