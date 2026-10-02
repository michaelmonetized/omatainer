# Project dependency recovery

Project → Project dependencies checks embedded audio against its external
sources and reports unavailable effect/instrument IDs, retained state schema
and bytes, requested enable state, and available rendered clip fallbacks.
Referenced sampler slots without embedded audio remain explicit in the report.
Their source and controls survive; the existing library relocation and sampler
retry controls restore those sources.

Search selected absolute folders for exact decoded audio identity, independent
of names and encoded file bytes. Duplicate candidates remain choices for the
user. Applying rechecks every chosen file, mount, fingerprint, encoded checksum
and decoded audio before publishing the complete project source-reference
batch. A failed, cancelled or stale operation changes no references. Embedded
PCM, notes and source automation remain intact. Closing pending review requires
an explicit discard. Save/reopen retains completed relinks in native state 9.

Project source references are distinct from library catalog identities. Sampler
retry can load a relinked source without catalog membership, verifies its full
PCM identity, and retains that typed source for subsequent reusable-bank loads.
This path never credits an unrelated catalog track or changes global library
records. Native projects continue to play their embedded PCM while a source is
missing.

Unknown device IDs or unsupported serialized device state retain an offline
placeholder. Effects pass dry audio; instruments remain silent. An unavailable
track instrument uses an existing embedded rendered clip through the track's
gain, pan, EQ and effect chain. The proxy follows the clip's musical position by
resampling, including regions/loops, and waits during count-in. Notes and MIDI
source lanes stay editable. A supported ID without incompatible state restores
its actual processor on reopening. Explicit sampler-instrument replacement is
undoable; retained state counts toward History admission and worker retirement,
with no callback allocation or final-reference release.

This layer retains unknown device data; it does not introduce an external audio
plugin host. Compatible built-in effect/instrument restoration executes real
processors. Unsupported external IDs/state remain visibly offline until a
compatible implementation is available.

Bounds: 1–64 search roots, 100,000 entries, 4,096 files/candidates, depth 64,
8 GiB of source reads and conservative decoded-work credit. Errors, symlinks,
foreign nested mounts and limits mark results partial. Source references allow
256 unique audio/path keys and 96 KiB of metadata. Device IDs allow 1,024 bytes
and each opaque state allows 1 MiB; native document and aggregate processor
limits still apply.

## Verification

Native App/egui accessibility actions open the actual menu, inspect/search,
choose/apply, save and reopen. They also exercise changed candidates, stale
projects, cancellation and explicit discard during native close. Filesystem
fixtures include Unicode and renamed extensionless files, duplicate matches,
wrong same-sized audio, unavailable roots and exact PCM-bit identity. Device
fixtures cover native file persistence, long snapshot names, unsupported state,
real restored delay/synth processing, MIDI-lane retention, rendered stereo
fallback, silent pads, replacement undo/redo and History-capacity refusal.


## Frozen qualification

Source: `a61add4fd0542aab53562abf9021bd4a8a9d1494`. Linux aarch64; native
builds, callback workloads and accessibility checks ran locally.

- Frozen ordinary suite: 1,236 passed, zero failures, 27 opt-in ignored,
  one separately run exhaustive scene-index check; 252.64 seconds.
  That check passes on the optimized release executable in 473.86 seconds:
  **all 1,237 ordinary tests pass**.
- All 17 dependency, sampler recovery, undo/admission and native schema
  checks pass on the exact optimized release test executable.
- Unchanged standard gate: eight workloads, three repeats each, zero callback
  allocations/frees. Original audio hashes remain producer
  `26f84beea80f5ec5`, composer `e2f197b02633bd3d`, live DJ `d7711a2dc3b32a39`
  and hybrid `adc9540dab057fed`. Independent recheck of the preserved production
  executable passes. Gate: 2026-10-02T20:32:34.594696+00:00 to 2026-10-02T20:40:39.286104+00:00.
- Private native AT-SPI preflight: 158 actions, 253 visited nodes,
  591 frames. Eight license/package fixtures pass.

Production SHA-256: `f10a82de9b9b225dc0bb7843a6d914178947d67e82ba1c2f30210a1af0d93c56`.
Release test SHA-256: `e8afb67f9568e627694b531d5680ccae0f727180a0dea1e3dda198a50f795a53`.
License manifest SHA-256: `82aca35b543e840a663ac30822384cfe9f8e906a8cae742ebc54134cf4c45629`.
Unchanged policy SHA-256: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`.
The source inventory binds 458 files.

Preserved executables and license records:
`/home/michael/Projects/omatainer-work/issue-115-qualified-release/`.
Raw receipts in `/home/michael/Projects/omatainer-work/`:
`issue-115-final-suite.log`, `issue-115-release-dependency-tests.json`,
`issue-115-release-dependency-tests.log`, `issue-115-release-scene-boundary.log`,
`issue-115-performance.json`, `issue-115-performance.raw.json` and
`issue-115-license-tests.log`. Earlier failed/interrupted logs remain separately
labeled. One ordinary attempt overlapped a rebuild and lost its child
executable path; another exposed an incorrect expected error category in the
new admission assertion. Final runs use a frozen executable and the corrected
budget expectation.

These receipts qualify implemented native headless software paths. Physical
audio/MIDI devices, desktop screen readers and external plugin hosting are
outside this layer. The inherited issue107 supplemental quiet-host wall maxima
remain unresolved. No installation, merge or issue closure is performed here.
