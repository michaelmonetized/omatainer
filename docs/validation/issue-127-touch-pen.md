# Independent touch controls and reported pressure: issue 127

Source freeze: `4a5a3686cdca6cc25082e1bd51ba8e17f1ea31bd`. Base: PR #495,
`stack/bindings-only-validation`.

Current, clipped pad and fader targets own device/contact pairs independently.
Shared pads release after their last contact; faders retain their first owner.
Short taps and reused IDs preserve native event order through egui layout retries.
Pressure scales sample gain, synth attack and recorded velocity; the existing full
attack and reserved release path remain unchanged. Keyboard activation guards now
remain separate from editing guards, preserving simultaneous keyboard/touch owners.
Rejected attacks never retain a pad owner or resume after queue drain. The deck
width calculation now includes EQ knob width so deck B's fader is fully reachable.

## Local qualification

Linux aarch64, locked Rust/Cargo 1.98.0:

- 1,380 ordinary tests pass: 1,379 serial (289.18 seconds) and the exhaustive
  scene-boundary check (257.38 seconds); 30 opt-in tests remain ignored.
- 59 optimized focused checks pass: eight actual touch App fixtures, eight command
  admission fixtures, nine pad/recording fixtures, 11 keyboard fixtures, 17 native
  accessibility fixtures, four display fixtures, one bindings model fixture and
  the native bindings import/export regression from #495.
- Actual App/renderer events cover simultaneous pads and both pitch faders plus
  the crossfader, equal IDs across devices, shared-pad ownership, emulated mouse
  suppression, conflicting contacts, off-pad holds, cancellation with invalid end
  positions, short taps, layout retries, focus loss, resize/DPI, guide/Preferences,
  safety recovery, invalid forces, overflow and full queues without resumed holds.
- Pressure checks verify sample gain, synth attack and captured MIDI velocity.
  The producer/renderer pressure loop performs zero allocations/frees; reserved
  releases remain accepted when the ordinary queue is saturated.
- Eight license/package fixtures pass.
- Controlled optimized gate passes eight workloads with three repeats, reviewed
  audio hashes and zero allocations/frees in measured callback workloads.
  CPU 6: 2026-10-03T17:39:02.090571+00:00 to 2026-10-03T17:41:54.310029+00:00.
- Native AT-SPI preflight: 158 actions, 264 nodes,
  591 App frames; 520 retained source/build/gate files.
- Independent source, embedded-license and gate verification of the preserved
  production executable pass.

## Artifact bindings

`/home/michael/Projects/omatainer-work/issue-127-qualified-release` retains:

- omatainer: `cf31016d7000992496f980354a7b06bb33bf6f41674caa0dc666ac45c42aa555`
- release-tests: `feb0c8ceb1a45f57c86d5457e1e53fd2081531708af98bd5674798c114e51aa7`
- manifest.json: `39edce1966c3aff46255366707dd1086a5544ea37c36b742705df163d7c29ce9`
- notices.json: `14f1064a39065206a8c0a7fb086a7fcd63c7a4a4b041817443160c90bc24b459`
- policy.json: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`
- performance.json: `f02f9fd08c670795bf4846314b235e0e554adf7998c032537d0742b2a92e433d`
- Ordinary test executable: `c6d14312258bb0a4ff480e7c118c6ac11b2a09f03e2dc8714f5ed2cc69388e1b`

Adjacent build, package, focused, ordinary/boundary receipts and gate JSON/raw/log
retain results. Preliminary tests exposed lost events on layout retries, touch
cancellation by focused-key activation, and the clipped second fader; all were
corrected before the source freeze. Frozen qualification passes without a rerun.

## Pending physical acceptance

The native fixtures run actual App controls and renderer without an OS window.
No physical touch/pen device is present. Multi-touch hold/release, pen input,
device removal and listening on supported hardware remain pending; this receipt
does not mark the physical acceptance criterion complete. winit 0.30.13's Linux
Wayland and X11 handlers emit `force: None`, so those backends use full attack.
Reported-pressure fixtures prove the adapter event/renderer path, not Linux pen
pressure. Pen-as-mouse controls retain their existing behavior; no tablet driver
is added. A disappearance without a native cancel/focus event is not established.
Runtime gestures never persist; saved scale/bindings keep existing Apply/reopen.
`docs/touch-and-pen.md` and the native guide describe behavior and platform limits.
