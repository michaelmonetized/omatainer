# Searchable commands and portable shortcuts: issue 126

Source freeze: `44df1725bfd2365e688e2adafc237ff3a7670736`. Base: PR #493,
`stack/localized-safety-confirmation`.

Setup → Commands searches the existing bindable command registry by description,
context and effective binding, with duplicate aliases grouped. Keyboard opening
uses the first unused Ctrl+Shift+P/K/F3 chord, respecting saved overrides. Search,
IME composition and cancellation never dispatch musical typing. Explicit command
execution uses ordinary admission, protection, safe-mode and undo paths.
Preferences capture logical keys into a draft, report collisions, reset defaults
and export/import a strict bindings-only document using the existing cancellable
worker and atomic publication. Preview/Apply persists; Cancel retains the applied
profile. `docs/shortcuts.md` documents contexts, layout behavior and platform limits.

## Local qualification

Linux aarch64, locked Rust/Cargo 1.98.0:

- 1,370 ordinary tests pass: 1,369 serial (288.27 seconds) plus the
  exhaustive scene-boundary check (256.45 seconds); 30 opt-in ignored.
- 54 optimized focused tests pass: 11 keyboard fixtures, 34 preference/model/
  storage/native UI fixtures, one registry/help/default check, five actual safety
  fixtures and three catalogue fixtures.
- Actual App event fixtures exercise keyboard-only palette open/search/cancel/run,
  saved opening-chord collisions, logical/physical key disagreement, IME Enter,
  capture cancellation, collision refusal, Apply/save/reopened dispatch, reset,
  bindings-only export, existing-file refusal, import/cancel and invalid JSON.
  Other profile settings and existing files are checked for preservation.
- Eight license/package fixtures pass. Binding exports reuse the existing bounded
  staging/fsync/publication path; imports reject oversized, unknown-version,
  unknown-action, reserved-key, colliding and duplicate-key documents.
- Controlled optimized gate passes eight workloads with three repeats, reviewed
  audio hashes/checks and zero allocations/frees in measured callback workloads.
  CPU 6: 2026-10-03T16:29:53.966730+00:00 to 2026-10-03T16:32:46.894371+00:00.
- Native AT-SPI preflight: 158 actions, 265 nodes, 587 App frames.
- 518 retained source/build/gate files. Independent source, embedded-license
  and performance verification of the preserved production binary pass.

## Artifact bindings

`/home/michael/Projects/omatainer-work/issue-126-qualified-release` retains:

- omatainer: `62a31dd47e453c757dd5a398c46cf7bfee4297b0468654b8e7b73b3bfb146dd2`
- release-tests: `41133258468f32ccc9bb46785192cc4024af269290e27fca3dfaaa819a568a4c`
- manifest.json: `a7d0cf706d343679eb663636483c48b269d1c7b25079ef9097b8748525ae81a7`
- notices.json: `14f1064a39065206a8c0a7fb086a7fcd63c7a4a4b041817443160c90bc24b459`
- policy.json: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`
- performance.json: `5b7d6b2762e028dd52a63f9f910c5acb04a2255f16220cf1bf9c277b356f5128`
- Ordinary tests: `6af8d8487cbb579a8d1f9c425c981ab522cfe38ed107b6ee146e0d6ba93095c1`

Adjacent ordinary receipts, optimized focused logs, package log and performance
JSON/raw/log retain the frozen results. Preliminary native lock/target fixture
failures were corrected before this source freeze; all frozen checks pass.

Actual App controls and renderer run without an OS window. The layout fixture
uses distinct logical and physical egui keys; it does not establish a physical
AZERTY or non-Latin keyboard/IME check. egui-winit 0.32.3's unsupported non-Latin
key fallback, clipboard interception and reserved project/navigation keys remain
as documented. The palette lists existing bindable actions; project file actions
retain their Project menu and reserved keys. Physical input/audio/display remains
unverified. Artifacts were preserved before next-layer compilation.
