# Variable-tempo deck grids

Deck Actions → Beatgrid editor maps musical beat numbers to unchanged source seconds. Up to 32 anchors can be inserted in any order, replaced at the same beat or deleted. Beat 0 remains the downbeat. Every segment must increase strictly in time and stay within 20–400 BPM; invalid input retains the previous draft and disables Apply. Exact boundaries use the following segment; pickups extrapolate the first and the tail continues the last.

Slip moves all anchors. Stretch scales all segments around beat 0. Half/double tempo checks every segment. Preview and Cancel do not edit playback. Apply is receipt-qualified, acknowledged by the renderer and adds one undo entry. Replaced-media receipts and delayed Delete widget identities cannot retarget another edit.

Loops, loop half/double/reloop and Match use local beat/time coordinates. Synced advancement follows one exact musical sample step, bypassing rate smoothing for anchored maps; scratch input retains direct source-speed control. A forward synced loop wraps its overshoot in beats, preserving phase when its end and start have different tempos. Key-lock retains its existing supported-rate/fallback policy. Original PCM, analysis BPM and absolute cue positions are preserved.

Library and native project state schema 13 retain anchor maps. Supported older states/catalogs migrate with constant grids; an older header containing an anchors field is refused, including an empty field. Backups accept schema 12 or 13 only when the manifest matches the catalog header, verify the original checksum first and migrate in memory without rewriting the archive. Collected restores preserve prior versions and copy original assets.

## Fixtures

- `engine::beatgrid::variable_tests`: bounded storage/geometry, inverse conversions, 32-anchor overflow/refusal, local loops/Match/Undo, generated ramping-click PCM and original recorded human drum PCM at 44.1/48/96 kHz output. A 20↔400 BPM boundary fixture checks fractional loop wrap. Audio samples remain identical before/after playback and editing; rendered beat positions and audible click energy are checked. The ignored optimized workload measures real hybrid OutputCallback processing with two full 32-anchor maps, 8,192 notes and 128/256/1,024-frame buffers, including callback heap counts and render CPU budget.
- Native AccessKit/egui grid widgets exercise insert/replace/delete, invalid coordinates, delayed actions, Cancel, Apply acknowledgement and receipt invalidation. A copied original human drum file passes through the real decoder, crate load, native grid editing, durable library save/reopen at a changed output rate and actual deck playback.
- Library/project fixtures check unchanged source media and schema-12 constant-grid migration/new-field refusal. A real collected-backup fixture checks legacy archive migration, header mismatch rejection and schema-13 anchor restore.

The recorded test material is the unmodified Google LLC [Groove MIDI Dataset 1.0.0](https://magenta.tensorflow.org/datasets/groove), performance `drummer3/session2/11`, recorded from a person playing a Roland TD-11 electronic kit. [Retained attribution, license and hashes](../../tests/fixtures/beatgrid/README.md). It is one human electronic-drum performance, not an acoustic drum corpus or a factory sound. No FreeToUse PCM was used.

## Qualification status

Development focused builds passed. The source-bound combined full suite, private native audio paths and optimized performance gate are pending for this new source revision. The previous combined receipt remains explicitly tied to its earlier source. Ignored, queued or earlier-source workloads are not evidence for this revision.

These software fixtures do not establish human listening quality, physical hardware compatibility, display/converter latency, system suspend or USB unplug recovery.
