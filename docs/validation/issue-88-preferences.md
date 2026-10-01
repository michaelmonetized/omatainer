# Issue 88: persisted preferences and named setup profiles

Implemented in the shipped App and startup path. Studio/Performance are real
saved profiles, with duplicate/rename/delete/default controls. The preview is a
snapshot of the exact draft submitted to the IO worker; any subsequent edit
invalidates Apply. Imported data stays a draft. The active profile can retain an
unavailable unchanged audio request while other settings are edited; changed
incompatible audio requests must be corrected before Apply.

## Runtime behavior

- The actual CPAL startup uses the resolved named/default device and advertised
  rate, channel count, sample format and buffer size. Duplicate names, missing
  devices and unsupported configurations fail explicitly. Unknown buffer limits
  are labeled. Audio stays callback-owned and is never transferred for live setup.
- Failure recovery runs before stream ownership. Explicit retry starts a fresh
  process after closing the recovery event loop; a defaults-once launch neither
  overwrites the file nor claims the requested route is running. Main-channel
  routing is shown precisely; no independent cue route is claimed.
- MIDI policy uses the companion manager implementation and its generation
  receipt. Saved preference, requested policy, applied policy and availability
  errors are distinct. Preview/Cancel does not configure the manager.
- Folder changes cancel old scans and invalidate already-staged old-root metadata
  candidates, then scan the new roots without discarding the visible crate before
  replacement is ready. Theme/font overrides and scale apply after save. Canonical
  action metadata provides defaults, override validation, dispatch and Help labels.

## Persistence and limits

Version 2 contains an active name and 1–32 profiles; names are 1–80 visible
characters. Profile audio numeric bounds, unique absolute UTF-8 line-editable
folders (at most 64), MIDI selection bounds, finite font/scale bounds and shortcut
conflicts are validated. The supported version-1 single-profile form migrates
without dropping its values; no claim is made that a previous release shipped it.
Unknown versions, fields or action IDs and duplicate map keys fail closed.

Files are at most 1 MiB, O_NOFOLLOW/O_NONBLOCK regular-file reads. Filesystem and
backend operations are worker-owned; requests/results are bounded. Saves use
0600 same-directory staging, file sync, destination-identity recheck and atomic
publication; new exports use no-clobber publication. A postcommit directory-sync
failure is saved-with-warning. Explicit invalid-file Reset backs up the original
bounded regular bytes first. Symlink/special-file destinations are never repaired.
Optimistic identity checks detect observed external changes; arbitrary concurrent
editors are not locked. Cancellation checks bracket IO, but kernel calls cannot
be forcibly interrupted.

The schema excludes credentials/environment/runtime state. Export intentionally
retains explicit device names and paths and labels them for cross-machine review.

## Evidence

- Storage tests: exact round trip, supported migration, duplicate/unknown fields,
  symlink/FIFO/oversize refusal, delayed cancellation, external modification,
  private modes and no-clobber export.
- Audio planner test: actual `StreamConfig` rate/channels/fixed buffer forwarding,
  missing/ambiguous/unsupported refusal, unknown buffer warning and mono/stereo/
  extra-channel route descriptions. This opens no real audio hardware.
- Real egui App tests: native button/checkbox actions, preview/apply/persist/reopen,
  effective shortcut reaching renderer, changed scale/font/startup flags, cancelled
  delayed preview/save, failed import/unavailable route, explicit backup Reset,
  stale editor-buffer regression, actual new-root scan and external-write failure.
- Actual MIDI handoff/manager/renderer UI test holds the same pitch from two
  synthetic physical inputs: Preview/Cancel preserves both source identities and
  notes; committed Performance selection releases only the excluded source.
- Native Linux test: `python3 scripts/check-accessibility.py --test-binary <test executable>`.
  On private D-Bus, native AT-SPI discovers Preferences, sets UI scale through Value,
  toggles theme following, previews, applies and verifies the real private file's
  saved 1.25 scale, then cancels/closes. Existing deck/cue/sample/analog pad proofs
  also run. Evidence: `/tmp/issue88-atspi-final.log` (220 initial nodes; native
  action trace includes Preferences, UI scale, Preview, Apply and Cancel).

The native fixture exercises real App output and Linux accessibility APIs, without
opening a native window or touching the desktop bus/settings. It is not Orca,
human usability or physical-controller/audio-route qualification. Project-view and
undo integration belongs to the assembled stack, not this isolated baseline.

Final isolated checks: 477 ordinary tests passed, 5 opt-in fixtures skipped by the
ordinary run; after the final completion-notice UI addition, all 12 preference
storage/UI groups passed again. Production `cargo build`, new-module rustfmt,
Python syntax and `git diff --check` passed. The final private native run passed
with 220 initial nodes, 27 native actions and a saved 1.25 scale; its 292 frames
are a sequencing observation, not a frame-rate/performance measurement.

## Final assembled stack

The complete stack passes **612 ordinary tests**, with 7 opt-in fixtures ignored,
and `cargo build --offline`. The private native AT-SPI fixture now combines the
project workflow (New, compose, Undo, Redo, Save, New, reopen) and preference
Preview/Apply/Cancel in one run: 228 initial nodes, 51 native actions, one saved
and reopened note and a saved UI scale of 1.25. Frame count is not an FPS claim.

Integration preserves durable-library opening while honoring the selected
startup scan flag. Startup Help/MIDI panel defaults establish only their initial
view values, preserving any prior engine edits and avoiding a falsely dirty fresh
project. Undo/Redo have stable preference IDs; a saved remap reaches the actual
renderer and appears in the Edit menu. Native close is cancelled with a visible
message while preference work is pending; the user can wait for its result or
cancel the operation before closing again. Project close/dialog states also
prevent starting a new preferences operation midway through close.

The rebuilt native executable passes license-record/package verification, seven
ordinary CLI protocol cases, five runtime-isolation groups and the isolated
panic-abort scene CLI/IPC/engine probe. Physical audio routes, controllers,
converter latency and human screen-reader use remain unqualified.
