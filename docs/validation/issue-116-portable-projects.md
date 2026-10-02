# Portable native projects: issue 116

Source freeze: `d14887b54106a0606c301cfaa980b34d53c85bb9`.

Project → Portable project captures the actual native session and lists every
embedded sample, saved device and preset/settings identity, unavailable plugin
identifier and referenced sampler source without audio. The existing application
dependency catalogue and full distribution notices travel inside the manifest.
Audio and external plugin rights stay explicitly unverified; plugin binaries are
not copied.

Every embedded PCM sample is included. The user selects verified original files
individually. Read-only source collection verifies retained files and mount
identities, encoded SHA-256 and decoded PCM, and deduplicates identical source
bytes into relative checksum names. The live project and its saved baseline stay
intact. Changed musical revisions or source choices require inspection again.

The flat versioned `.ompack` format accepts only `session.omat` and exact lowercase
SHA-256 source names. Metadata, entry counts, source reads and aggregate payload
are bounded. Extraction uses private staging, new regular files, per-entry and
whole-archive checksums, native schema/media validation and actual DSP preparation.
Import compares reviewed metadata, saved device/settings identities and unresolved
source declarations with the session. Collected originals are rebound to the new
folder, including sampler-bank retry sources. Existing destinations are refused;
publication uses no-replace operations. Cancellation removes unpublished staging;
a completed publication reports success even if cancellation arrives later.

Open imported project uses the existing unsaved-work and editor-draft guards and
starts stopped. Unavailable effects bypass; unavailable instruments preserve
notes, automation and opaque state. Existing rendered clip audio remains playable.
Uncollected sources without audio remain explicit. Global hardware preferences and
controller profiles are local.

## Native workflow fixtures

The actual App/egui menu and accessibility handlers inspect, choose original
sources, select dependency notices, export, review, import and open. Two distinct
original paths containing identical bytes become one collected entry. The fixture
removes the entire original directory and moves the archive to an isolated fresh
profile before reopening. It verifies exact PCM, MIDI notes/source automation,
sampler controls, unavailable instrument state, a retained rendered proxy and a
missing sampler slot. Sampler retry succeeds with an empty library catalogue and
no separate alias book. Deck and rendered clip callbacks produce finite nonzero
audio. A fresh test process repeats import/open/playback with original paths absent.

Failure fixtures cover corrupt/truncated archives, absolute/traversal/duplicate
entry names, unsupported versions, incomplete/oversized metadata, missing notices,
oversized notice labels, wrong PCM, source limits, symlinks, changed reviewed
metadata, forged device/preset lists, cancellation and existing files/folders.
Rejected operations preserve current state and clean their staging directories.

## Frozen qualification

Linux aarch64, Rust 1.98.0, locked local builds.

- Frozen ordinary suite: 1,250 passed, zero failures, 28 opt-in ignored,
  274.63 seconds. The separately run exhaustive scene-index check passes in
  256.46 seconds: **all 1,251 ordinary tests pass**.
- 59 checks pass on the preserved optimized release test executable: portable
  codec 4, portable GUI 3, dependency core 3, dependency GUI 4, timing GUI 5,
  timeline 6 and native project workflows 34. The portable GUI round trip also
  invokes the private fresh-process import/playback probe.
- Standard gate: eight workloads, three repeats each, zero callback
  allocations/frees, unchanged original audio hashes: producer
  `26f84beea80f5ec5`, composer `e2f197b02633bd3d`, live DJ `d7711a2dc3b32a39`
  and hybrid `adc9540dab057fed`. The policy was not changed. Independent recheck
  of the preserved production executable passes.
- Private native AT-SPI preflight: 158 actions,
  253 visited nodes, 568 frames. All eight
  license/package fixtures pass. License inventory/check and diff checks pass.

Gate: 2026-10-02T22:46:50.722599+00:00 to 2026-10-02T22:49:41.040015+00:00.

Production SHA-256: `48c88062da27a103b14444b0072c222aa5b3a526b8efdff867b8b06c837ca591`.
Release test SHA-256: `9bc1a3655fb9450c2b0c5f5c45f888144f9656e8632aa11d27bfc01921aa7e80`.
License manifest SHA-256: `98a5524a1f16bea57fae36920d0f2973ddb2166f6dbf1f3edca316b7e336af75`.
Unchanged policy SHA-256: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`.
The source inventory binds 463 files and the distribution
catalogue contains 330 records.

Preserved release executables and license records:
`/home/michael/Projects/omatainer-work/issue-116-qualified-release/`.
Frozen debug test SHA-256:
`4b39da92a44ed668b6590f65ef726c76ecb8053252d60071e4b2d7ee3a519405`.
Receipts under `/home/michael/Projects/omatainer-work/`:
`issue-116-ordinary-tests.json`, `issue-116-final-suite.log`,
`issue-116-scene-boundary.log`, `issue-116-release-focused-tests.json`,
`issue-116-release-focused-tests.log`, `issue-116-performance.json`,
`issue-116-performance.raw.json` and `issue-116-license-tests.log`.
The first optimized filter attempt retains 19 passing checks; its incorrect
zero-match timeline filter is excluded. The final receipt uses the exact native
timeline module and contains 59 nonzero passing checks.

These checks run locally on Linux aarch64. The isolated profile and fresh-process
fixtures establish software portability on this supported host; a second physical
machine, physical audio/MIDI devices, desktop screen readers and external plugin
hosting are not claimed. The inherited issue 107 supplemental quiet-host wall
maxima remain unresolved. No installation, merge or issue closure is performed.
