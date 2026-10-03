# Saved workspaces and panel windows: issue 128

Source freeze: `16f370c86434d9221cdcc9af5b17aaf6e6f4b0f5`. Base: PR #498,
`stack/native-mouse-release`.

Production, Mix and DJ layouts reuse existing Decks, Sampler, Library and Session
controls. Named copies support ordering, visibility, scrollable heights and
secondary windows. The actual preference editor previews, cancels, saves and
reopens layouts and dimensions; versions 1–10 migrate to default layouts.
Transport, emergency silence, warnings and the shared controller target remain
available in each panel window. Window focus does not change MIDI destination.

## Frozen local qualification

Linux aarch64, locked Rust/Cargo 1.98.0:

- 1,396 ordinary tests pass: 1,395 serial in 292.90 seconds and the exhaustive
  scene-boundary check in 259.82 seconds. Thirty opt-in tests remain ignored.
- 103 unique optimized focused tests pass in 13.81 seconds, with zero ignored.
  Coverage includes workspace models/migration, real preference controls and
  persistence, native viewport callbacks, independent mouse/touch ownership,
  keyboard actions, accessibility, display, pad admission and licenses.
- Workspace events verify small clipped panels and focus scrolling, native
  child-only shortcuts, GUI release without cutting MIDI owners, root dialogs,
  close/focus/DPI/monitor changes and logical Wayland sizes without window
  coordinates. Closing a command palette's owner or changing layouts restores
  root shortcuts; closing another window preserves the palette.
- Five vendored native mouse-adapter regressions and eight package fixtures pass.
- Controlled optimized gate passes eight workloads with three repeats, reviewed
  audio hashes and zero allocations/frees in measured callback workloads.
  CPU 6: 2026-10-03T20:29:39.684561+00:00 to 2026-10-03T20:32:32.269328+00:00.
- Native AT-SPI preflight: 158 actions, 265 nodes and 590 actual App frames.
  The manifest retains 534 source/build/gate files, including patched native
  adapter sources.
- Independent source, embedded-license, gate, immutable package and packaged
  executable verification pass. Production and test executables are selected
  from Cargo compiler-artifact JSON, never from a remembered filename.

## Actual Wayland windows

The frozen production executable ran on Hyprland Wayland with one physical
monitor, in safe mode and isolated XDG directories. Root, Decks and Sampler
opened as three actual native windows with the application's desktop identity.
Each was resized and captured. Closing Sampler returned its actual controls to
the main window without rewriting preferences. Clean close and reopen restored
all three saved windows; no test process remained.

`issue-128-palette-owner-wayland-evidence/receipt.json` retains source/binary
identity, compositor window inventory, resize/close/reopen results and captures.
The OS-window run opened no audio/MIDI devices. Preference Apply is separately
covered by actual App/editor/worker fixtures. Physical touch/pen/controllers,
monitor unplugging, desktop DPI/server changes, audible playback and other
desktop backends remain unqualified. Synthetic viewport fixtures cover those
reported events without claiming hardware acceptance.

## Artifact bindings

`/home/michael/Projects/omatainer-work/issue-128-palette-owner-qualified-release`
retains:

- omatainer: `843e68962f81725662a311b7d62d1bf79bd918004457b2624a01d81a5e75eb75`
- release-tests: `0028d9d0b4e53cb0d91223672649901e9aa1b10bb8f3afae0b33a69fcd4744a3`
- manifest.json: `f1b40a546069d54dd406b80e68a1af119b28f8c021e69b5de76d327352443e37`
- notices.json: `501bfeeddb16dbb5b4fba2db871f3089f4df228611b4b4fab445a8c14612f3b2`
- policy.json: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`
- performance.json: `434ee6651af136f85c4fa4565378d8a47c4499d3032afb99d4ed87e8e7e5b3c4`
- Ordinary executable: `fc4076d3218091c1e76ebce5cf9bf4323ab4459fc7e81e98e187efcf4dd6d1f4`

Adjacent ordinary/boundary/focused receipts, compiler artifacts, package fixtures,
native adapter log and gate raw/log retain results. The immutable package is
`issue-128-palette-owner-package`.

Preliminary failures exposed legacy fixtures carrying a new workspace field,
viewport lookups inside an egui input lock, missing manual regeneration, missing
pad-fixture input lifecycle and stale license source counts. Each was corrected.
An unchanged undo capacity test failed once, then passed 20 isolated repetitions
and subsequent full suites; no cause is inferred and no assertion was waived.
The actual window check exposed missing child desktop identity. Final review
exposed an orphaned palette owner; both are corrected in the frozen source.
Final full and controlled qualifications pass on their first frozen attempt.
