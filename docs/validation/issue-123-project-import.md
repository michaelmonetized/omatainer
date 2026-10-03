# Selected native project material: issue 123

Source freeze: `3fc15db219b6cfd4a5d6e181de66fdf6de59d609`. Base: PR #486,
`stack/video-cancel-status`.

Project → Import from another project browses a read-only native document and
reviews selected tracks/scenes, clip/controller lanes, timing, routing and
unavailable devices. Apply appends fresh persistent identities in one native
undo transaction. Existing track processors, live input, decks and destination
conductor remain intact. Preparation, source verification and discarded large
allocations stay on a bounded cancellable worker. A changed source, destination,
output rate, protection state or undo budget refuses application.

## Local qualification

Linux aarch64, locked Rust/Cargo 1.98.0:

- 1,338 ordinary tests pass: 1,337 serial plus the exhaustive scene-boundary
  check on a separate CPU; zero failures, 30 opt-in ignored. Final suite time:
  280.35 seconds; boundary: 258.67 seconds.
- Five optimized focused tests pass. Model fixtures prove fresh IDs, unchanged
  source, existing processor/input continuity, allocation-free Apply/Undo/Redo,
  media and MIDI-controller remapping, unavailable serialized instruments/effects,
  rate changes through history, validation and omitted-device native defaults.
  Actual native App controls exercise Browse/Review/Apply/Undo/Redo/Save/New/Open,
  cancellation, changed sources and changed destinations.
- Eight license/package fixtures pass, including the standalone empty-PATH
  fixture. T3's interpreter executable alias requires the same explicit
  sys.executable=/usr/bin/python3.14 launcher used by the preceding layer.
- Controlled optimized gate passes eight workloads with three repeats each,
  all reviewed checks/audio hashes and zero callback allocations/frees.
  CPU 6: 2026-10-03T12:12:55.283120+00:00 to
  2026-10-03T12:15:48.541682+00:00.
- Native AT-SPI preflight: 158 actions, 264 visited nodes, 598 App frames.
- 506 retained source/build/gate files. Independent source, embedded-license and
  performance verification of the preserved production binary pass.

## Artifact bindings

`/home/michael/Projects/omatainer-work/issue-123-qualified-release` retains:

- Production: `e420f09b95cc64ad202a0794326faa1f51b1e4664b0696b14203620cd8b69b3c`
- Release tests: `c6ffadf7d2baaa8a298d9f492004a15accaa2a779a822af1da9de1ac8b79b227`
- Manifest: `be03f3253603de87124f374cf881116d0a0f306be95738e512c8d4719bffb2e4`
- Ordinary tests: `8c25afe53fa313b01f40f7c836313dc29847df27108126dfc5a7a200cb68d686`
- Policy: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`

Adjacent final-qualified ordinary receipts, focused release logs, package logs
and performance JSON/raw/log retain the actual results. Binaries were preserved
before any next-issue compilation.

The native App and renderer are exercised without an OS window. Physical MIDI,
audio devices and display behavior remain unverified. Import includes selected
Session material and its embedded dependencies. Decks, picture, conductor,
global sampler banks and hardware profiles are outside this selection. Source
beat positions play at destination tempo; the review requires that decision.
