# Native reusable templates: issue 117

Source freeze: `27f4bf977cc24da722536f3b0fbff06add493316`.

Project → Project and track templates saves independent `.omtemplate` files.
Project templates retain the complete creative document and embedded audio.
Track templates retain the selected track's devices, mixer, required drum audio
and exact scene-bus alias, excluding clips, notes, launch state and deck content.
Applying a track template requires one matching active bus; missing or ambiguous
bus names fail without changing the session. Other tracks and existing clips stay
intact. Applying configuration stops playback and starts fresh undo history.

Inspect retains the native file fingerprint, metadata and dependency manifest.
Use and duplication recheck that record. Duplicate writes a new named file with
no replacement. Project use creates a fresh session namespace, stopped and
unsaved, and never makes the template the live project's Save destination.
Ordinary project Open refuses the template wrapper.

Exact desired audio and MIDI aliases remain in template metadata. Inspection
shows unavailable or ambiguous routing endpoints, with incomplete discovery
labelled unverified. Review hardware creates a Preferences draft for explicit
Preview / Apply; active connections are unchanged until that step. Track review
binds to the current native target identity and refuses a replaced/deleted target
before creating a draft. Cancel changes preserves the active profile.

Version-7 preferences persist Demo, Empty or a project template as the startup
choice. Existing preferences migrate to Demo without changing audio settings.
Empty startup uses the same prepared native graph as New Project. Project
startup validates the template before opening backends and creates a fresh,
stopped, unsaved copy. Hardware stays on the active profile for explicit review.
Missing/invalid templates preserve saved settings and offer an empty session for
this launch. Safe mode ignores external templates. Explicit empty/audio recovery
choices survive a second independent setup failure without rewriting preferences.

Portable template backup uses the existing checksum-verified `.ompack` codec.
All embedded playable PCM, desired hardware, native device state and dependency
notices travel with the wrapper. Import verifies the wrapper and prepared native
graph, stages privately and publishes a new folder without replacement. User
source files are not required for playback. Cancellation before publication
cleans staging; completed publication is reported honestly.

## Qualification

Linux aarch64, Rust/Cargo 1.98.0, locked local builds.

- Final ordinary suite: 1,264 passed, zero failures, 29 opt-in ignored,
  257.05 seconds. The exhaustive scene-index check passes separately in
  254.89 seconds: **all 1,265 ordinary tests pass**. Both independent processes
  use the same immutable test executable; their runs overlap on separate CPUs.
- **92 optimized checks pass** on the preserved release test executable:
  template GUI 6, template engine 2, Preferences GUI 15, preference storage 8,
  startup recovery 1, MIDI routing model 3, project GUI 35, portable GUI 3,
  dependency core 3, dependency GUI 5, timing GUI 5 and timeline 6.
- The actual egui menu/widgets exercise save, inspect, duplicate, project use,
  selected-track application, exact missing/ambiguous MIDI reports, cancellation,
  source changes and existing destinations. Startup choices persist/reopen through
  Preferences. Portable import/playback also runs in a fresh process after the
  source template is deleted. Original template bytes and existing destinations
  remain unchanged in the tested workflows.
- Standard gate: eight workloads, three repeats, zero callback allocations and
  frees. Original hashes remain producer `26f84beea80f5ec5`, composer
  `e2f197b02633bd3d`, live DJ `d7711a2dc3b32a39`, hybrid `adc9540dab057fed`.
  Policy is unchanged. The controlled run is pinned to CPU 6; ordinary and
  focused tests finish before measurements. Independent check of the preserved
  production executable passes.
- Private native AT-SPI API preflight: 158 actions, 253 visited nodes,
  582 egui/renderer frames. Eight license/package fixtures pass. The retained
  inventory contains 468 source/build/gate files and 330 license entries.

The earlier broad attempt found one version-6 MIDI fixture that carried the new
startup field under an old version. Its legacy fixture was corrected, with
old-version rejection retained; the complete final run above passes. The gate's
first copied-path invocation was rejected before measurement. The successful
run uses the exact controlled build artifact.

## Artifact bindings

Preserved in `/home/michael/Projects/omatainer-work/issue-117-qualified-release`:

- Production: `4ce06ba44216bb545e85c02a0c2f49f66316532773f009588a4e25e8767e7ec5`
- Release tests: `65472e8aabe8bf0263ee69c43d413dd39cfaec23a6feb066fc788d732f51b59a`
- License manifest: `1330bf587f82f1bccf1f1a108af265e3253ce0460d75633243542040dd1d403c`
- Ordinary tests: `98b5013dbb8f742a6442dfcb75edfeb099423d7342b1c26758f30d521b0e7229`
- Policy: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`

Local receipts share `/home/michael/Projects/omatainer-work/issue-117-`:
`ordinary-tests.json`, `final-suite.log`, `scene-boundary.log`,
`release-focused-final.json`, `release-focused-final.log`, `performance.json`,
`performance.raw.json`, `performance.log`, `license-tests-final.log`.
The controlled gate ran from `2026-10-03T00:27:19.595079Z` to
`2026-10-03T00:34:16.329271Z` (October 2 local time).

This qualifies the actual App/renderer, native accessibility API and local
persistence/worker paths. No desktop window, Orca, physical audio/MIDI device,
second physical computer or external plugin host QA is claimed. The inherited
issue-107 supplemental quiet-host wall limit remains a separate unresolved
requirement; it is not replaced by this standard gate.
