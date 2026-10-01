# Issue 99: persistent manual beatgrids

A grid records a downbeat in source seconds and a uniform period independently
of the decoded BPM hint. Beat coordinates before the downbeat are negative, so
pickups and leading silence do not redefine musical beat zero. Manual tempo is
validated from 20 to 400 BPM. The initial model uses four-beat bar labels; it does
not implement tempo maps, changing meter or automatic downbeat detection.

The deck editor previews Set at playhead, signed slip, stretch, half/double tempo
and Reset. Its waveform preview displays the symmetric mean magnitude of all
three analyzed bands, with at most 512 columns and 128 visible grid markers.
Applied and draft markers are distinct. Fields preserve f64 precision. Cancel,
Escape, closing the window and replacing the media discard unapplied drafts.
An explicit scroll area and fixed footer keep the final controls reachable in a
640-by-360 window. The controls have native names, roles, focus and contextual F1
help generated from the same catalogue as the offline manual.

Apply is one receipt-qualified undoable command. Its own atomic acknowledgement
is read before the coherent preparation snapshot. An applied command remains
acknowledged even if another grid edit or Undo changes the current preparation
before the GUI polls. A pending command is not completed by somebody else's
preparation revision, and receiver loss reports an unknown outcome. Stale media,
invalid input and protected-mode rejection cannot edit a replacement source.

The manual grid controls effective tempo, Match phase and quantized loop origin;
cue positions remain absolute source positions. Reset recovers the immutable
decoded BPM hint even if an older effective-tempo snapshot is still published.
Catalog schema 4 and native project State 3 retain grids. Older supported catalog
and project states default to no manual grid; invalid and future states fail
before installation. The recovery journal and container remain version 1 because
their existing generic state payload already carries the versioned project.

Grid-only preparation follows the same stable track identity and exact-byte
relocation rules as cues. Essential catalog saving continues while performance
protection defers optional source hashing. Both qualification admission and
worker hashing include grids without any hot cues; returning to Studio retries
the bounded backlog. Reanalysis can update its own metadata without replacing
manual geometry. Engine receipts publish preparation in 87 fixed atomic words.

## Evidence and limits

The actual App tests use real decoded 16 kHz PCM fixtures with pickups and leading
silence, while explicitly injecting ambiguous 60/240 BPM analysis hints. They
correct both to 120 BPM, edit the origin, persist, restart at 96 kHz output and
compare every cue's source position and beat coordinate. The injected hints
exercise correction of ambiguity; they do not qualify the tempo detector.

Further actual GUI/worker paths cover a grid with no hot cues saved under
protection, deferred hashing, Studio resumption, verified relocation and reload.
Recovery tests create durable records through the real worker, preview and
restore State 3, migrate legacy State 1/2, and reject future/invalid states while
preserving the current session. Restored transport remains stopped.

Renderer tests cover pickup arithmetic, sample-rate independence, Match and loop
phase, coherent publication, stale/no-op edits, Undo/Redo, reanalysis and catalog
identity. A warmed callback allocation fixture verifies both an accepted grid
edit and an already-queued command rejected after protection begins. The private
Linux AT-SPI workflow exercises opening, field focus, half/double, Set, slip,
Apply, draft Reset and Cancel through the actual GUI and renderer evidence.

All files and buses used by these fixtures are private. These checks do not
claim physical-controller, screen-reader listening, audio-device deadline or
vendor-algorithm parity. The fixed release timing gate is qualified separately
on the assembled stack.

On the normalized issue-98 prerequisite `442d7cd`, the complete local run passed
**784 ordinary tests, with 13 opt-in tests ignored**, using
`cargo test --locked --offline -- --test-threads=1` (152.43 seconds). This includes
the three added protection/relocation/recovery integration groups and the prior
grid model, renderer and GUI groups. The explicit offline-manual regeneration
test and `cargo build --locked --offline` also passed.

`scripts/check-accessibility.py` against that freshly compiled test executable
visited **240 native nodes** and performed **109 actions across 1,011 App frames**.
The union retains the original 128-action fixture cap and the existing cue,
project, recovery, help and performance workflows. The source/component license
inventory validated, and the production binary's embedded manifest/notices
matched the regenerated records exactly. No timed release-gate result is inferred
from these correctness runs.

## Initial assembled release verification

On final issue 98 (`e9b8d02`), the complete ordinary suite passed **784 tests**,
with 13 existing opt-in fixtures ignored, in 46.36 seconds using four threads.
The source/component inventory check also passed.

The controlled release gate passed all eight fixed workload groups across three
sessions on 2026-10-01, 08:01:24–08:04:43 UTC. It rebuilt locked/offline release
and test executables and bound their embedded inventories to the tested source.
Production executable SHA-256:
`487da79a6b68cf3b40cf298c305604627401dc65d9f7bedb72ff558ce03ca42a`.

Host: Apple M1 Pro, 10 logical CPUs, 16 GB RAM, aarch64 Linux
7.1.13-3-2-ARCH; SCHED_OTHER, nice 0, schedutil and affinity CPUs 0–9. Other
agent CPU jobs were held; unrelated user applications were not changed.
One-minute load was 3.015 before the workload and 1.727 afterward.

Worst p99 and maximum across the three sessions, in milliseconds:

| Workload / observation | p99 | Maximum |
| --- | ---: | ---: |
| callback_producer / callback_wall | 0.8413 | 1.0430 |
| callback_producer / render_cpu | 0.6867 | 0.7070 |
| callback_composer / callback_wall | 0.9164 | 1.6926 |
| callback_composer / render_cpu | 0.6983 | 0.7307 |
| callback_live_dj / callback_wall | 0.0963 | 0.1650 |
| callback_live_dj / render_cpu | 0.0600 | 0.0662 |
| callback_hybrid / callback_wall | 0.8700 | 0.9770 |
| callback_hybrid / render_cpu | 0.7083 | 0.7207 |
| large_crate_ui / frame_wall | 3.7575 | 5.5815 |
| multi_controller_ipc / frame_wall | 4.9847 | 6.3974 |
| multi_controller_ipc / ipc_roundtrip | 8.9686 | 10.0268 |
| multi_controller_ipc / midi_dispatch | 2.2938 | 3.5418 |
| project_roundtrip / frame_wall | 4.2176 | 4.2176 |
| long_note_recording / frame_wall | 5.2074 | 5.2074 |
| long_note_recording / renderer_wall | 2.3989 | 3.5629 |

Golden audio hashes, exact notes, project round trips and all other mandatory
behavioral assertions passed. Callback allocations, frees and rejected commands
were zero. The native AT-SPI union replay completed 109 actions, visiting 240
nodes over 953 actual App frames; it retained the original 128-action ceiling.
The replay covers both cue and grid editors plus existing project, recovery,
help, preferences and performance-safety flows. It persisted/reopened one note
and the 1.25 UI scale.

Independent source-bound report verification, exact executable package
verification, seven CLI protocol, six follow-protocol and five runtime-isolation
groups passed. The package is retained as `issue-99-final-package`; raw report,
summary and logs are retained separately from the moving stack worktree.
These are local software checks. Physical audio/controller operation, hardware
deadlines and human screen-reader/listening behavior remain for final user QA.

## History-capacity acknowledgement correction

Integration review found a rejection path that bypassed the usual command
retirement helper. A valid grid change refused by Undo capacity preserved the
grid and history, but its per-request acknowledgement remained Pending. The UI
could therefore keep waiting for an operation the renderer had already rejected.

The common retirement path now rejects pending grid acknowledgements before
passing owned commands to the worker. Applied acknowledgements remain Applied.
A regression first reproduced Pending instead of Rejected using real prior
note-edit history and an exhausted Undo budget. With the correction, all 31
ordinary Undo groups pass (one explicit fixture ignored); the new path preserves
the grid/history checkpoint and performs zero callback allocations or frees.
The corrected assembled release is verified separately below.

On the corrected assembled source (`bede882`, including `703b35a`), **785 ordinary
tests passed**, with 13 explicit fixtures ignored, in 43.16 seconds. The fresh
controlled release gate passed all eight unchanged groups across three sessions
from 2026-10-01 08:55:18.888912 to 08:58:38.954261 UTC. The exact production
binary SHA-256 is
`d4180c1e96e4b9568b01c5f8b980e9f8e7e3d049aff3d18223fe9eef3da7676a`.
The same local M1 Pro host and scheduler settings described above were used;
other agent CPU jobs were held, with one-minute load 2.747 before and 2.540 after.

Worst callback wall p99/maximum across the three sessions, in milliseconds:
producer 0.8703/1.0606, composer 0.8684/1.4058, live DJ 0.0969/0.1238 and
hybrid 0.9024/1.2093. All original audio/state goldens and callback heap/admission
checks passed. The native replay completed 109 actions over 963 App frames,
visiting 240 nodes and retaining one persisted note and UI scale 1.25.

Independent report validation, source-bound package verification, seven CLI,
six follow-protocol and five runtime-isolation groups passed. The corrected
package is `issue-99-ack-final-package`, and report/raw/log copies are
`issue-99-ack-performance.*`. The earlier release evidence remains available;
these corrected artifacts identify the latest qualified source and executable.
