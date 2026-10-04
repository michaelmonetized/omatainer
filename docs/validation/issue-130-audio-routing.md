# Independent audio routing: issue 130

Source freeze: `21655715f3691224c7ff114a8a36b5b6af15921a`. Base: PR #501,
`stack/issue-129-audio-recovery`.

Setup → Audio routing applies a reviewed worker-prepared graph while stopped.
Stable physical aliases, virtual buses, track/scene/deck/main tap points and record
sources persist through native Save/Open and Undo/Redo. Invalid maps, cycles,
changed revisions and cancellation preserve the applied state. Device absence
leaves retained channel addresses silent. Input preview and activation are separate
explicit actions; reopening never opens an input.

## Frozen local qualification

Linux aarch64, locked Rust/Cargo 1.98.0:

- 1,426 ordinary tests pass: 1,425 serial in 300.69 seconds and the exhaustive
  scene-boundary check in 257.59 seconds. Thirty-two opt-in tests are ignored in
  the ordinary suite; the new native routing test runs separately below.
- 169 unique optimized focused tests pass in 32.06 seconds, with zero ignored.
  Actual App controls cover route confirmation, cancellation, cycles, stale
  revisions, native numeric channel edits, Undo/Redo, project reopen, WAV Stop and
  Cancel. Engine tests cover every saved physical address, independent pre/post
  taps, retired identities, source boundaries and callback heap work.
- A real decoded-WAV regression proves licensed provider preview finishes an
  existing capture before preview frames render and refuses new captures while
  preview plays. Native project capture also excludes transient preview media.
- Eight package fixtures pass in 3.821 seconds. Source/embedded records,
  independently recomputed performance evidence, the immutable package and its
  actual executable's embedded manifest verify.
- The controlled gate passes eight workloads with three repeats, reviewed audio
  hashes and zero allocations/frees in measured callback workloads. CPU 6:
  2026-10-04T01:28:36.276706+00:00 to 2026-10-04T01:31:28.102488+00:00.
- Native AT-SPI preflight: 158 actions, 266 visited nodes and 593 actual App frames.
  This uses the native accessibility API without a desktop window or Orca.
- The manifest retains 550 source/build/gate files. Compiler-artifact JSON selects
  production and test executables by target and profile; retained hashes match.

## Captured native channels

`scripts/check-audio-routing.py` creates an owned private PipeWire 1.6.8 server,
opens real CPAL/ALSA output and input streams and connects 32 matching playback
and 32 matching capture ports. Both frozen ordinary and optimized executables
capture every output at 48 kHz through the renderer's record aliases and decode
the resulting 26-channel and six-channel floating-point WAVs. Every channel has
measured test-signal energy and a peak between 0.009 and 0.011; no frame contains
more than one channel above the 0.0002 comparison threshold. All owned children
exit; both complete WAVs, configurations, logs and binary-bound receipts remain
in adjacent `130-216-debug` and `130-216-release` directories.

Input queue overflow is zero. Missing-input-frame counters are 384 ordinary and
3785 optimized; these become silent input frames and do not establish exact
physical dropouts, independent-clock stability or dropout-free playback.
The pinned Linux CPAL/ALSA backend advertises at most 32 live channels. Saved
addresses through channel 64 are checked separately by a synthetic 64-output
renderer regression; they are not 64-channel native-device qualification.
The pinned WAV decoder supports 26 channels per file; wider native captures use
separate aliases. Physical converters, USB interfaces, external listening and
other desktop backends remain unqualified.

## Maximum saved graph

Three isolated CPU-6 runs retain 128 tracks, 512 scenes, 96 ports, 32 buses,
256 connections and 4,096 maps. Rendering 128 frames at logical width 32 takes
2.343–2.410 ms against a 2.667 ms nominal 48 kHz block interval,
with zero callback allocations/frees. This is a quiet capacity graph, not a fully
voiced maximum show or a universal hardware deadline guarantee.

Provisional measurements exposed repeated wide tap copies and per-sample alias
searches. Prepared indices, native stereo taps and direct terminal-port frames
remove those costs while keeping the same route bounds. An earlier optimized
support-recovery UI batch timed out under concurrent tests; its standalone check,
clean batch rerun and final frozen suite pass. The first final boundary supervisor
was terminated after printing a pass; the direct rerun above confirms exit zero.
Failed/interrupted logs remain separate from final qualification receipts.

Explicit routing marks existing stereo performance-source measurements incomplete;
old attribution is never reused as proof of custom routes. Playlist events remain
available. See [the routing workflow](../audio-routing.md) for the shipped limits.

## Artifact bindings

`/home/michael/Projects/omatainer-work/issue-130-final-routing-qualified-release`:

- omatainer: `44ed5875eac983330c31e830a425be60547081d402f1fb723ca435c1d8ad0be9`
- release-tests: `8a319c31478e1aa7ecd69eb7689d7133637e2736d1729bde9e65deaa40ec5ca2`
- manifest.json: `86d08186bddcdd4d5671087046f335702d7a48c645681a313a730e5f8903db71`
- notices.json: `501bfeeddb16dbb5b4fba2db871f3089f4df228611b4b4fab445a8c14612f3b2`
- policy.json: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`
- performance.json: `85b09dd9d7ebee7d122d82c319977e3f4b3a4a51a4c89dc3b468c91a951e5b76`
- Ordinary executable: `e6a612a69977645ab70b4e5892e504b3a10c91019fa1f6c23a93115d90b7d9a3`

The immutable package is `issue-130-final-routing-package`; its release receipt
records the frozen revision and clean source tree. Adjacent compiler-artifact,
ordinary, boundary, optimized, capacity, native, package and gate records retain
the commands and results. Documentation added after this freeze changes no
manifested executable source.
