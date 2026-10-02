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

Qualification receipts follow below after the frozen local checks complete.
