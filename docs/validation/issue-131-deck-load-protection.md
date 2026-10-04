# Deck load protection: issue 131

Source freeze: `a450c6b68a23951808e478c0031cb91ef89f6975`. Base: PR #502,
`stack/issue-130-audio-routing`.

Explicit per-deck locks guard playing, touched and audibly fading targets in
Studio and performance mode. Mouse, keyboard, dropped-file, MIDI, IPC and media
history replay share producer/renderer checks. Quiet paused decks remain usable.
Locks are transient app-session choices; project/output changes retain them and
restart starts unlocked. Performance protection also guards live unlocked decks.

The review identifies the captured target, current track and replacement. Cancel
starts no decode and preserves existing playback. Confirm retains the old audio
through preparation, then applies a ready replacement stopped. Source, lock and
safety changes invalidate retained reviews; each review is consumed atomically
at its media boundary. A review cannot be reused or target another deck/owner.
Confirmed eject reports queued separately from renderer completion.

## Frozen local qualification

Linux aarch64, locked Rust/Cargo 1.98.0:

- 1,436 ordinary tests pass: 1,435 serial in
  323.25 seconds and the direct exhaustive scene-boundary check in
  280.03 seconds. Thirty-two opt-in checks remain separate.
- 260 unique optimized focused checks pass in 39.37 seconds,
  with zero ignored. These include actual App/native control actions, keyboard,
  drop, MIDI admission, real Unix socket requests, CLI validation, delayed decode,
  media Undo, load receipts, controller queue/display workflows, localization
  and the synchronized offline manual, plus inherited routing/recovery checks.
- Locked Studio refusals preserve the exact source, playing state and advancing
  cursor. A rendered constant-signal regression verifies finite, nonzero output
  after refusals and zero callback allocations/frees. Race hooks cover entering
  a lock after ordinary receipt claim and invalidating an already reviewed claim.
- App tests exercise review/cancel/confirm for load and eject. They distinguish
  queue acceptance from actual media mutation and verify a changed source cannot
  be replaced by a delayed approved decode. Each input-origin attempt dismisses
  the preceding refusal dialog before trying the next origin.
- Eight package fixtures pass in 3.686 seconds. Source inventory, actual production
  embedded records, the independently checked gate, immutable package and its
  actual executable verify. The manifest retains 552 source/build/gate files.
- The CPU-6 gate passes eight workloads with three repeats, reviewed audio hashes
  and zero allocations/frees in measured callback workloads:
  2026-10-04T02:51:31.098500+00:00 to 2026-10-04T02:54:22.360699+00:00.
- Native AT-SPI preflight exercises 158 actions, 270 visited nodes
  and 602 actual App frames. It uses an owned private accessibility session
  without a desktop window or Orca.

These are software/UI/socket and rendered-sample checks. Physical MIDI hardware,
audio interfaces, external listening and other desktop backends remain unqualified.
The inherited native routing/backend receipts belong to their own frozen sources.

## Artifact bindings

`/home/michael/Projects/omatainer-work/issue-131-qualified-qualified-release`:

- omatainer: `48f19783b57b178e254dd2382710cbb03eb334070c810d0444c51e00032157b7`
- release-tests: `bbfe0ef4a28f51c2bf85f3ab857b009a43ead3102a8925dfd1a8b0f19a53dcab`
- manifest.json: `6e5e67b6710576956bf84bbb020e3f3d9d00dedb539c1d13a9844020b13ad812`
- notices.json: `501bfeeddb16dbb5b4fba2db871f3089f4df228611b4b4fab445a8c14612f3b2`
- policy.json: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`
- performance.json: `1310114f43cf19630c74f40ccc6dad893b580632a63eb66fb0861520c54e7810`
- Ordinary executable: `63f120b78e73d11d70e3cb5433e95200e20433330ce82680e9489f6bbdfbb557`

The immutable package is `issue-131-qualified-package`; its release receipt
records the frozen source and clean manifested source tree. Adjacent compiler-
artifact JSON, ordinary/boundary/focused receipts, gate logs and package records
retain the commands and results. Validation prose added after the source freeze
changes no manifested executable input. See [the workflow](../deck-load-protection.md).
