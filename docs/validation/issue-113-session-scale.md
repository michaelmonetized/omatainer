# Issue 113: editable tracks, scenes and persistent session identities

This layer follows issue112 / PR468. Producers can create audio or MIDI tracks
and scenes, name, color, reorder, duplicate and delete them, and undo or redo
those edits. A session has up to128 tracks and512 scenes. The initial demo
remains eight by eight; its original musical output and performance policy
remain unchanged.

## Identity, playback and limits

A persistent session namespace and monotonically assigned object IDs separate
display order from backing slots. Reordering changes metadata without moving
the playing DSP node, source automation, recording destination, clip schedule,
selected controller target or held-note ownership. Deleted slots can be reused,
but their IDs cannot. Track duplication assigns fresh note/object IDs and shares
immutable PCM; the copy starts stopped and unarmed.

Project schema7 stores dimensions, active objects, order, IDs, names and colors.
Legacy schemas1–6 migrate the original eight-by-eight music without writing an
automatic conversion. Legacy identity incorporates metadata and embedded PCM;
current files validate the complete layout and reject duplicate/stale identities.
Actual native container save/reopen preserves every state field, MIDI channel
automation and embedded PCM bits at the maximum dimensions.

Structural edits prepare their future state and affected nodes on a worker.
Publication qualifies the session, generation, revision, rate and undo epoch
before the renderer swaps reserved owners at a block boundary. Inverses retain
the affected music, FX, bus/focus and asset pins; unrelated playing nodes remain
in place. Deleting an object stops its owned voices/streams and invalidates its
references. Retired allocations return to the worker, including rejected edits.

The resulting processor graph has an explicit256 MiB bound, enforced before
track/scene creation, project preparation, rate changes and undo/redo. A refused
operation reports a visible limit and leaves the current music unchanged.
The inherited8192 notes per clip,65536 total notes,16 MiB automation,64 MiB
metadata,128 effects per rack,16 banks,256 native PCM assets and1 GiB PCM bounds
remain independent limits. Maximum dimensions do not imply every combination
of those independent resources will fit. No silent truncation is used.

Targeted commands retain namespace/object references, including queued GUI,
MIDI and clip-edit controls. The input worker uses the exact configured route
reference captured when it matches a packet. Controls painted by the native
editor use the snapshot's references. A project swap or deletion/reuse cannot
restamp an old action onto a replacement object. Reordering preserves valid
references; clip gain and SMF review refuse stale destinations. MIDI ownership
and critical stops cover128 tracks; clearing one track preserves the other
owners of a shared pitch or sustain gate.

## Native workflow

The native session toolbar creates tracks/scenes and opens Edit Session.
The inspector exposes track and scene names, display positions, custom RGB
colors, duplication and deletion, with normal undo/redo. All controls have
native accessible names and keyboard navigation. Accessible Go to track/scene
controls reveal offscreen destinations. Stable widget identities follow objects
across reordering. The grid virtualizes both axes; full names remain available
in the editor. Fixed hardware presets retain their backing destinations; this
layer does not add physical controller banking or microphone recording.

CLI and shell scene arguments accept1–512 and address active backing slots.
Invalid or inactive destinations refuse. Internal scene indices and MIDI
bindings no longer alias above255. Existing eight-row keyboard shortcuts use
the first eight displayed rows. SMF import/export uses display pages and retained
cell references, preserving the existing64-cell transaction bound.

Requirements reference: [Ableton Session View manual](https://www.ableton.com/en/manual/session-view/).
This original MIT implementation imports no reference-product code/assets and
does not claim reference-product or physical controller parity.

## Qualification

The complete ordinary suite at corrected source
`2427b2ba592ebffd7a7467ec870913ce82e05710` passed **1210 tests, zero failures,
27 opt-in ignored**,500.85 s. It includes creation/duplication/deletion/reuse,
both directions of undo, maximum storage, rate-dependent processor refusal,
targeted stop saturation, unchanged IPC frame bounds, stale SMF review, live
MIDI ownership and actual native session editor actions.

A later shared-fixture-only correction replaces the test-only renderer
constructor with the identical fallible production constructor, allowing the
standalone panic-abort executable to compile. It changes no production behavior.
The final source-bound release gate and supplemental receipts are recorded below.

Earlier failed checks remain in the work directory. The first full suite had
two obsolete admission/retirement fixture expectations; the corrected tests
retain exact ownership and zero callback heap assertions. Native UI failures
exposed scroll/layout and old focus assumptions and were corrected before the
full passing suite. The first standalone probe failed to compile because its
shared fixture called a cfg(test)-only constructor. The first shell argv harness
encountered a GTK display dependency; the offscreen Qt generic/Fusion run and
actual private Quickshell-to-renderer workflow subsequently passed. No budget,
IPC bound, original audio hash or performance policy was relaxed.

## Frozen release receipts

Qualified source: `4c074b80d8cab6f493535084f65fc436e6c0b4d2`.
Linux aarch64, local release profile with debug assertions off. The unchanged
standard gate ran on CPU6 from2026-10-02 15:56:23 UTC through16:04:01 UTC.

- Eight workloads × three repeats passed, with zero callback allocations/frees.
  Original audio hashes: producer `26f84beea80f5ec5`, composer `e2f197b02633bd3d`,
  live DJ `d7711a2dc3b32a39`, hybrid `adc9540dab057fed`.
- Private native AT-SPI:158 actions,253 visited nodes,578 frames. This uses the
  actual App/renderer and native accessibility API without a desktop window,
  Orca, audio backend or physical hardware.
- Frozen native container test:128 tracks,512 scenes,65536 cells,
  17,803,187-byte actual `.omat` file and56 bitwise-identical embedded media.
  All state fields and four source automation messages survive reopen. Playing
  reorder preserves DSP address, automation Arc, routed held note, clip origin,
  selected controller focus and scene bus, with zero callback heap activity.
- Three actual native session UI tests pass, including15 editor operations,
  old-snapshot project replacement and accessible maximum-set navigation.
  Only168 clip nodes are painted at1440×1400. Thirty-two headless App updates
  (snapshot clone, egui paint/accessibility and a64-frame renderer block) measured
  minimum3.239632 ms, median3.340757 ms, p95 3.670924 ms, maximum3.686091 ms.
  These are observations, not a maximum-set hardware callback deadline claim.
- Eleven ordinary routing-output tests pass; the private ALSA opt-in test also
  passes. Two private source clients deliver all12 expected packets, plus reset
  bytes (172 observed packets). Every task-created port retires; no user
  hardware port opens. The paused actual input worker and last-track shared
  pitch/sustain ownership regressions pass on this same release test executable.
- The standalone production-constructor CLI/IPC/renderer boundary probe passes
  with panic-abort, rejecting every out-of-range u16 scene without mutation.

Release binary SHA-256:
`75007c0d91397704e0ef4bae1506647468019e7c33d36b513d12be49232a6246`.
Release test binary SHA-256:
`9426fcdd8607c45e7e6287a30bb5c5800ebca9b7a779a4c2dbd873f981c62ded`.
Unchanged policy SHA-256:
`fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`.
License manifest SHA-256:
`c9c76e82fc678175f9b775db6f316406f0c0439d08da05c3f2bf6ea4f2c265e3`.

All four scene boundary/IPC/native renderer tests pass on the final release test
binary (211.77 s). Source inventory/check binds447 files; all eight license and
package script fixtures pass. Final gate recheck, immutable package creation
and verification, seven CLI groups, six follow groups, runtime isolation, actual
headless safe startup and private Quickshell-to-renderer scene actions all pass
on the actual packaged executable. Its checksum equals the passing gate binary.

Final immutable artifact: `/home/michael/Projects/omatainer-work/issue-113-final-package-v2`.
The earlier package remains retained separately. The source/artifact-bound
summary is `issue-113-final-qualification.json`; raw gate/native accessibility
receipt is retained independently as `issue-113-performance-final-v3.json`.
Final maximum native container/editor/navigation JSON receipts and the `.omat`
file are in `issue-113-native-release-evidence-v2/`. Actual captured wire packets
are `issue-113-linux-virtual-v2.json` and `issue-113-routing-software-trace-v2.json`.
All retained logs and artifacts live under `/home/michael/Projects/omatainer-work/`.

Inherited issue107 supplemental quiet-host wall maxima remain unresolved.
Physical Pioneer DDJ-FX, Numark NS7 MarkII, APC40 mkII, MPD232 and MIDI keyboard,
listening, actual desktop Orca and audio-backend XRUN qualification remain for
the user's producer/composer/live-DJ QA. No installation, merge or issue closure
is performed by this layer.
