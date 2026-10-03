# Lost audio recovery: issue 129

Source freeze: `7b6104201b0bdc681160f26d911b0552b3e04d83`. Base: PR #499,
`stack/issue-128-workspaces`.

Reported errors, stopped callbacks and long owner/suspend gaps retire the unique
renderer into stopped input recovery. Held recording gates finish at the captured
position; project capture, Save and Close remain available. Reconnect confirms
the retained physical identity and exact accepted configuration. Unverifiable
aliases require an explicitly previewed fallback. Identity is checked again
after the stream opens and plays while muted, before enabling output.
Reconnect preserves saved preferences, project routes and emergency mute.
Inputs require separate acknowledgment; playback requires Play. Managed transport
starts ramp over exactly two milliseconds without callback heap work.

## Frozen local qualification

Linux aarch64, locked Rust/Cargo 1.98.0:

- 1,407 ordinary tests pass: 1,406 serial in 283.06 seconds and the exhaustive
  scene-boundary check in 260.04 seconds. Thirty-one opt-in tests are ignored in
  the ordinary suite; the new native recovery test is exercised separately.
- 85 unique optimized focused tests pass in 21.97 seconds, with zero ignored.
  Coverage includes owner retirement/rollback, renamed and replaced identities,
  replacement during opening, cancellation, retained recordings, exact ramps,
  actual App/worker confirmation and stale consent, emergency mute, input
  acknowledgment, support privacy, localization and the synchronized manual.
- Eight package fixtures pass. Independent source, embedded-license, performance,
  immutable package and packaged-executable verification pass.
- Controlled optimized gate passes eight workloads with three repeats, reviewed
  audio hashes and zero allocations/frees in measured callback workloads.
  CPU 6: 2026-10-03T22:12:51.430439+00:00 to 22:15:44.324239+00:00.
- Native AT-SPI preflight: 158 actions, 265 visited nodes and 593 actual App frames.
  This exercises the native accessibility API without a desktop window or Orca.
- The manifest retains 537 source/build/gate files. Compiler-artifact JSON selects
  production and test executables by target and profile; source hashes match.

## Real backend faults

`scripts/check-audio-recovery.py` creates a private PipeWire 1.6.8 null sink and
uses real CPAL/ALSA output streams. No physical audio stream is opened. Both the
ordinary and optimized frozen executables pass server restart, stream removal
and a six-second process freeze while recording and performance protection are
active. All owned children exit.

For each optimized fault, the project reopens with one completed recorded note;
the same note survives explicit fallback, input acknowledgment and Play. Backend
error and device-lost counters remain zero in all three cases: callback and
owner-gap observation detect failures the backend did not report.

Adjacent `129-rel-restart`, `129-rel-removal` and `129-rel-gap` directories retain
the source/binary-bound receipts, private configurations and logs. Ordinary
receipts are `129-activation-restart`, `129-activation-removal` and
`129-activation-gap`. Physical USB unplug/replug, actual system suspend/resume,
external-interface listening and other desktop backends remain unqualified.
Process freeze proves the owner-gap path; it does not prove system suspend.

## Artifact bindings

`/home/michael/Projects/omatainer-work/issue-129-activation-qualified-release`:

- omatainer: `029ef8404b0d8d914e6b7922a12434999f0d8842ff7ad277b4d1d4747ecaf4c8`
- release-tests: `42f3737d07ecd67f4f9d64802b3efe667e842ba00537f8682089585adab62e7a`
- manifest.json: `30818a3d2b93aa3c65ff0d59d8c82f960e5da01f0356d85aa557d5c9854bbae0`
- notices.json: `501bfeeddb16dbb5b4fba2db871f3089f4df228611b4b4fab445a8c14612f3b2`
- policy.json: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`
- performance.json: `a2b8cf03d8759449567a4fa9f5073524862d0963e8a140b9b3078a0849ca56f4`
- Ordinary executable: `33ddfb8777da2996985db8d458f3958af6f1dfc9e42fd53bf1a993772ccf4474`

The immutable package is `issue-129-activation-package`; its receipt records the
frozen revision and an unmodified source tree. Adjacent compiler, ordinary,
boundary, focused, package and gate records retain the exact commands/results.

Provisional checks exposed floating-point ramp length, asynchronous recovery
acknowledgment and fixtures assuming the old offline error priority. Those were
corrected without waiving protection. Final review added the opening-race guard;
its regression requires zero rendered replacement callbacks, unique retirement,
retained identity, offline capture and successful later retry. The final frozen
full suite, real backend scenarios and controlled gate pass.
