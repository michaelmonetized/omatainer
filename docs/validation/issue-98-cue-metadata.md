# Issue 98: named, colored, persistent hot cues

Each of eight hot cues retains its source-frame position, a validated name of up
to 64 UTF-8 bytes, and an optional opaque RGB color. Empty names use the slot
number; empty colors follow the theme. The deck pads, visible waveform markers
and cue list use the same renderer-published values. Text fields are drafts;
Apply creates one undoable edit after validating the current media receipt and
set cue slot. The editor closes when that media is replaced. Queued Set, Jump,
Delete and style edits cannot apply to replacement media. All owned receipt
rejections retire on the existing worker rather than freeing on the callback.

The cue list scrolls in small windows and provides an explicit Close action.
Keyboard text editing retains the global-shortcut focus guard. Full names,
positions in source seconds and RGB values remain in accessible descriptions
when compact pad/waveform labels are shortened.

Library schema 3 stores styles with existing preparation and defaults older
schema 1/2 files to unnamed theme-colored cues. Native projects include the same
metadata. The list distinguishes session-only preparation, saved library cues,
pending saves and content not yet qualified for a future move.

Relocation changes the catalog association, never filesystem placement. The
user moves/copies the file and enters its absolute new path. Worker SHA-256
verification requires exact bytes, regular no-follow files, stable file identity
before/after reading, and an 8 GiB per-file verification bound. A prior digest
allows a missing original; otherwise the original must remain available. Names
alone never transfer preparation. A successful transaction preserves TrackId,
records the old path/fingerprint, and lets still-loaded/history-pinned old
receipts update the same verified track. Different content later occupying an
old path receives a distinct identity. The last-played navigation anchor follows
only the same verified bytes at the current location, including after filtering;
replacement bytes cannot inherit it. Conflicts, changed bytes and unavailable
unverified originals leave the old association intact.

The selected row follows that stable identity if it is still selected when the
new crate publishes. A user's intervening selection is preserved. The result
of a relocation is separately identity-qualified and retained across stale
worker results and later essential saves, so cancellation/rejection cannot turn
into a generic successful-save message. A stale request to an already-associated
destination still reports its explicit verification rejection.

Performance protection refuses new relocations, cooperatively cancels queued
verification and guards its catalog commit. Content hashes are optional work;
cue edits, preparation/history persistence and final close flush remain
essential. The worker defers automatic qualification for captured tracks and
retries after returning to Studio. This optional backlog retains at most 256
latest source identities and never starts a whole imported-library hash sweep.
Unqualified tracks retain all cue data; keep the original file available until
qualification or explicitly verify it during relocation. A committed catalog
remains truthful while protected row publication follows the existing deferred
worker policy. Every coalesced optional component rechecks its own cancellation
generation under the shared commit guard. Protected cue style edits and new cue points remain available;
cue deletion is rejected centrally at producer and renderer boundaries.

## Local evidence

Real App/egui/AccessKit tests drive all eight Set, name, color and Apply controls,
then verify exact renderer/preparation values, waveform and pad descriptions,
durable restart and restored fields. Separate cases cover malformed drafts,
queued stale Apply/Set/Delete, small-window last-row focus and shortcut isolation.
Real WAV decode and catalog workers exercise source-file move/reload, stale
publication and reordering, manual selection preservation, cancellation during
verification, and retained essential captures. Protected cue edits save before
qualification and qualify automatically after Studio resumes.

The private Linux AT-SPI fixture additionally opens the actual list through the
production Actions menu, sets slots 1/8, checks native text semantics and focus,
applies, deletes only slot 1 and closes the list. Arbitrary name/color typing is
verified by actual egui keyboard events; it is not claimed as native AT-SPI text
injection. All fixtures use temporary files/private buses. No installed desktop,
physical controller or audio-device qualification is claimed.

Normalized on the final issue-96 baseline (`acceca5`), the complete local ordinary
suite passed: **725 tests, 12 existing opt-in tests ignored**, with
`cargo test --locked -- --test-threads=1` (132.87 seconds). The five real cue GUI
groups also passed independently before the complete run. The private AT-SPI
run visited 237 native nodes and executed 100 actions across 879 actual App frames,
including the added cue workflow and existing project/help/performance flows.
`cargo build --locked`, the source/component license inventory check, and exact
embedded manifest/notices comparison against the built executable also passed.
Independent read-only peer review found no remaining blocker after the explicit
relocation-result fix.
The controlled release timing gate is run separately on the final assembled
stack; these correctness runs make no new audio deadline claim.

## Assembled stack and history eviction

On the final recovery baseline, the initial ordinary suite passed 760 tests
with 13 opt-in fixtures ignored. The first controlled gate then correctly
failed: composer callback jitter was 2.740–2.772 ms and hybrid jitter
3.347–3.423 ms, above the unchanged 2.667 ms budget. Producer and hybrid
maximum wall times also exceeded the unchanged 10.667 ms ceiling. All golden
audio hashes and behavioral checks matched; renderer CPU changed much less
than complete callback wall time. This failed run is retained separately.

The callback samples localized the regression to command blocks. Relative to
issue 97, median command-block non-render time rose from 1.543 to 2.427 ms for
producer and from 2.075 to 3.261 ms for hybrid. Adding inline cue styles enlarged
undo transactions, exposing the full-history vector's oldest-entry removal: it
shifted all retained inline entries for every eviction. History now uses a
preallocated ring. Front eviction and back redo truncation retain chronological
indices, held-recording owners, fixed capacity and worker-owned retirement.
No callback budget or benchmark workload changed.

All 29 existing undo groups passed. A new regression fills and wraps history
more than twice, verifies retained entries stay at their existing addresses,
checks zero renderer allocations/frees and unchanged capacity, then replays all
256 retained undo/redo values exactly. Independent read-only review covered
logical indexing, held-note finalization, redo suffixes, sample-rate pruning and
fixed/retired byte accounting. The assembled ordinary suite then passed
**761 tests, 13 opt-in fixtures ignored**, using four test threads (39.49 s).

## Final release gate

The corrected assembled release passed all eight fixed workload groups across
three sessions on 2026-10-01, 07:43:46–07:46:56 UTC. Its production executable
SHA-256 is `af1cd3b8fa2c251502a64722c71229d4393d5baac40f101e6339c5979ae9d7e2`.
The gate rebuilt locked/offline release and test executables, checked their
embedded source inventories, and ran the mandatory native preflight.

Host: Apple M1 Pro, 10 logical CPUs, 16 GB RAM, aarch64 Linux
7.1.13-3-2-ARCH; SCHED_OTHER, nice 0, schedutil, affinity CPUs 0–9. Other agent
CPU jobs were held; unrelated user applications were not changed. One-minute
host load was 2.852 before the workload and 1.700 afterward.

Worst p99 and maximum across the three sessions, in milliseconds:

| Workload / observation | p99 | Maximum |
| --- | ---: | ---: |
| callback_producer / callback_wall | 0.7737 | 1.3425 |
| callback_producer / render_cpu | 0.6879 | 0.7096 |
| callback_composer / callback_wall | 0.8147 | 0.9536 |
| callback_composer / render_cpu | 0.6981 | 0.7097 |
| callback_live_dj / callback_wall | 0.0930 | 0.3026 |
| callback_live_dj / render_cpu | 0.0619 | 0.0702 |
| callback_hybrid / callback_wall | 0.8455 | 1.1059 |
| callback_hybrid / render_cpu | 0.7095 | 0.7204 |
| large_crate_ui / frame_wall | 3.3504 | 5.9905 |
| multi_controller_ipc / frame_wall | 5.4359 | 7.1120 |
| multi_controller_ipc / ipc_roundtrip | 8.8438 | 10.6078 |
| multi_controller_ipc / midi_dispatch | 4.2981 | 5.9965 |
| project_roundtrip / frame_wall | 4.0831 | 4.0831 |
| long_note_recording / frame_wall | 3.9216 | 3.9216 |
| long_note_recording / renderer_wall | 2.4406 | 3.9966 |

All measured callback allocations, frees and rejected commands were zero;
golden audio hashes, note preservation, project round trips and the other
behavioral assertions passed. Worst callback jitter (p99 minus p50) was
0.1514 ms. The native AT-SPI replay executed 100 actions across 862 actual App
frames and visited 238 nodes, including the cue editor and existing project,
help, preferences and performance-safety flows. It persisted/reopened one
recorded note and the 1.25 UI scale.

Independent report verification and exact executable package verification
passed, followed by seven CLI protocol, six follow-protocol and five runtime
isolation groups. The final local package is retained as
`issue-98-final-package`; failed and corrected raw reports/logs are retained
outside the moving stack worktree. These local software measurements do not
qualify physical audio devices, controllers, live scheduling, Orca interaction
or human listening. Those remain part of the user's final role-based QA.
