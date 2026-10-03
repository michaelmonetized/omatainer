# Native video scoring: issue 122

Source freeze: `02e20275c1f93ec9c0b7f5cd2cb4b2db5671d1c5`. Base: PR #484, `stack/annotation-query-guard`.

One external constant-rate picture has frame-based trim, placement, signed timecode,
drop-frame timecode, preview latency calibration and named locators. The resizable
native preview can use an eframe detached viewport. Decoder/probe and renderer work
stay on bounded, cancellable workers. Audio transport publishes a per-callback
sample clock; seek releases clip notes and rebuilds schedules without heap work.
Native Save/New/Open persists and verifies the picture. Offline scoring owns a
separate native scene renderer and publishes WAV, lossless picture and alignment
metadata together. The live project and source picture remain intact.

## Local qualification

Linux aarch64; locked Rust/Cargo 1.98.0:

- **1,333 ordinary tests pass**: 1,332 serial plus the exhaustive scene-boundary
  check on a separate CPU; zero failures, 30 opt-in ignored. The final recovery
  migration fixture omits timeline_seconds from legacy schemas.
- **Eight optimized video/transport checks pass**: eight constant rates with
  sequential-pixel comparisons, H.264, FFV1, 10-bit ProRes, two-hour final-frame
  seeking, nonzero timestamps, tagged HDR and variable-rate refusal, corrupt or
  changed sources, actual native controls, render cancellation, Save/New/Open,
  sample-aligned picture/WAV and zero-allocation clip seeking.
- Controlled release gate: **eight workloads, three repeats each, zero callback
  allocations and frees**; all reviewed checks and expected audio hashes pass.
  CPU 6: `2026-10-03T11:30:13.058826+00:00` to `2026-10-03T11:33:08.743835+00:00`.
- Native AT-SPI preflight: **158 actions, 264 visited nodes, 588 App frames**.
- Eight license/package fixtures pass. T3's interpreter executable alias required
  launching the Python checks with sys.executable set to /usr/bin/python3.14;
  the standalone package check still ran with an empty PATH. No product or
  validation policy was changed for this runtime workaround.
- 501 retained source/build/gate files; git diff --check and independent
  source/embedded-license/performance verification of the preserved binary pass.

## Artifact bindings

`/home/michael/Projects/omatainer-work/issue-122-qualified-release` retains:

- Production: `25f8668a0fd8a5b3ba4d8ee224f8f6156ae8682cee85f20495176ba91f4b1c0b`
- Release tests: `5ad08886005a35fb7927e222d50dc27b74b2faca43024cb60b22e35bd542d782`
- License manifest: `0b1e3cfed68dfb67a76bdd375adfef8340417f45b9292b9b6e581c01b5f97697`
- Ordinary tests: `240f484212e0c9fde8ae24f2cb23a1c463589292db26443bc93c106cdb38b9c4`
- Policy: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`

Adjacent issue-122 receipts retain corrected ordinary-test logs and JSON,
release video results, corrected license fixtures, build logs, performance.json,
performance.raw.json and performance.log. Production/tests were copied before
any next-issue compilation.

The actual native App is exercised without an OS window. Detached desktop
viewport behavior and physical audio/display latency remain unverified. Tested
codec combinations are narrower than declared allowlisted support. This renders
a selected Session scene loop, with native mixer/effects and fresh DSP, rather
than an Arrange timeline. External picture remains external; portable export
refuses it explicitly. See docs/video-scoring.md for exact limits.
