# Issue 85 — persistent DJ catalog

This adds a DJ catalog independently of performance/project serialization. Tests
use private directories, headless engines and real egui controls; no desktop,
user library, hardware or provider configuration is changed.

## Identity and implemented preparation

`library::TrackId` is 16 random bytes represented as 32 lowercase hexadecimal
characters for media locations. The two shared built-in stems use reserved global
IDs 1 and 2, so importing a full catalog can coexist with a fresh default crate.
Pristine default stems yield to imported preparation; imported defaults cannot
erase a prepared stem. Other identity/preparation conflicts fail explicitly.
IDs persist independently of a location's media versions.
`engine::media_source::LibSource` is the shared typed location: `Builtin`, `File`
(absolute UTF-8 path), `Removable { volume_id, relative_path }`, or
`Provider { provider, media_id }`. Relative removable paths reject traversal;
provider IDs never enter the local decoder. Removable/provider loads fail with a
visible unsupported-resolution message. There is no network/provider adapter.

A track stores a current version and an archive of prior filesystem fingerprints.
Each version contains title, artist, BPM with provenance, key, optional duration,
last-play timestamp, main cue, eight optional hot cues, and optional loop
start/length/enabled. Times are source seconds, independent of output sample rate
and pitch. Loading restores markers and loop arming but never starts playback.
Old media bytes cannot donate analysis/preparation/history to replacement bytes.
IDs stay stable at that typed path; moves and content equivalence are not inferred.
Fingerprint metadata is cache identity, not a content hash.

No editable beat-grid structure exists in the current engine. The catalog stores
implemented BPM/provenance and cue/loop preparation. Unknown future grid fields
are rejected explicitly instead of being silently dropped.

## Format and recovery

The writer emits JSON schema 2:

```
{schema: 2, tracks: [
  {id, source, current, versions: [
    {fingerprint, metadata: {title, artist, bpm: {value, origin}, key,
      duration, last_play},
     preparation: {cue, hotcues: [8 optional seconds],
      loop_region: null | {start, length, enabled}}}
  ]}
]}
```

This is a shape description, not literal import JSON. Serde's externally tagged
source names are `Builtin`, `File`, `Removable`, and `Provider`. Built-in names are
`Drums` and `Harmony`; BPM origins are `Unknown`, `FilenameHint`, `Heuristic`,
`User`, and `Builtin`. `null` duration is unknown, distinct from measured zero.
SystemTime uses serde's seconds/nanoseconds-since-epoch representation.

Flat schema 1 is a supported migration input with
`{schema: 1, tracks: [{id, source, fingerprint, metadata, preparation}]}`. Migration
wraps each record in one version at current index zero and preserves every field.
It does not claim that a previous released app already wrote this format.
All DTOs reject unknown fields. Newer schemas, malformed data, duplicate/conflicting
IDs or locations, invalid numbers, and traversal paths fail closed. Imports reject
conflicts atomically; rejected imports do not discard unrelated queued cue/history
updates. Opening a bad primary never falls back to startup defaults or silently
replaces it with its backup. A missing primary plus existing backup also fails
closed, requiring explicit recovery while the app is closed.

The store caps JSON at 64 MiB, tracks at 100,000, versions at 1,000,000 and metadata
strings at 4096 bytes. These are validated bounds, not a claim that all such
libraries perform equally. A single metadata worker owns all disk operations,
with one job/result in flight and coalesced updates. Its existing retirement
acknowledgment keeps final large catalog/crate deallocation off the GUI.

An advisory writer lock prevents cooperating concurrent writers. Writes create a
private 0600 same-directory temporary file, sync its content, retain the previous
committed file at `library.backup.json`, rename the complete replacement, then
sync the directory. Unexpected external changes are checked before publication.
A failure after rename says the replacement committed but durability is
unconfirmed; retry syncs a complete replacement. Temporary leftovers from a dead
writer are ignored, never parsed as the library. Recovery never deletes the only
prepared copy. Ordinary filesystem/OS guarantees apply; the test does not emulate
a storage device lying about sync or physical power loss.

## GUI, renderer and close ordering

App startup opens the store on the existing metadata worker. Scan discoveries,
accepted decode metadata, import records and receipt capture merge there before
immutable crate publication. Persisted locations survive scans that omit them;
a missing local file reports the existing decoder failure when loaded.
Source-based selection and virtual viewport behavior remain in place.

Each admitted media load carries its own receipt and matching saved preparation.
The renderer publishes cue/loop edits through twelve atomic words and a sequence
counter. GUI reads make one attempt, never spin. A terminal receipt is read before
its final preparation/history, so unload/replacement before polling cannot retarget
or erase the prior load's edits. Unapplied/cancelled/failed requests cannot create
preparation. Initial builtin restoration checks receipt identity and skips decks
already edited before background catalog opening finishes. Queue backpressure
keeps eligible startup restores pending for retry. Successful restore advances
the native project's untracked checkpoint; stale receipt identities are inert.
Undo/redo publishes preparation only after all patches and media receipt swaps,
so the restored source retains its own preparation. Reopening native projects
publishes their restored deck preparation to the same library capture path.
Owned restore and fence commands retire through the undo recycler, including
stale requests, instead of releasing their final shared allocation on the callback.

Normal close first resolves the native project's Save/Discard/Cancel decision
and holds its engine close guard, then uses a FIFO `LibraryFence` and the
library worker's durable save receipt. The engine acknowledges a successful
close seal only at the callback tail, after that block's actual source-play
history publication. A project discard does not discard a library save failure;
that requires its own explicit decision. Keep working wins even on the same
frame that library saving completes. Import controls are disabled while close
is committing. Releasing the guard reopens admission on the next callback. Neither
renderer nor GUI performs filesystem work or waits for disk. The GUI remains
open and responsive until ready, with Retry, Keep working and explicit Close
without saving controls. `prepare_library_close` returns Pending/Ready/Failed for
the project close coordinator; it has no UI calls. A filesystem syscall already
in progress is not cancellable. Force-killing the process preserves the last
committed catalog, not changes that had not reached a successful save receipt.

## Validation

- Private mixed catalog: local mono/stereo WAV files, removable location and
  provider identity import through the actual egui Import catalog button. IDs and
  all metadata/preparation survive restart. Real decodes restore cue/hotcue/loop
  positions on the renderer; unavailable namespaces never enter the decoder.
- Cue/hotcue/loop edits, real source playback and unload occur before any GUI poll.
  The correct version retains final preparation, measured duration and history.
  Replacement bytes start fresh and leave the old prepared version archived.
- Flat schema 1 migration preserves all fields and the exact old bytes in backup.
  Unknown fields/newer schema/malformed primary cannot be overwritten by startup,
  scanning, retries or imports. A missing primary preserves the existing backup.
- Write-error injection at four commit boundaries verifies complete old/new
  catalogs. A fail-after-backup → retry → later-save regression covers POSIX
  same-inode rename no-ops, owned temporary-link cleanup and primary ctime changes. A separate native child writer is actually killed at all four
  checkpoints; a new writer reopens complete data, checks every identity and
  untouched preparation, and continues saving despite the orphan temporary file.
- A held metadata worker allows 24 real egui crate frames and admitted deck/master
  commands with actual rendering; the measured whole fixture stays below two
  seconds. Close remains Pending until both FIFO application and durable save
  completion; external modification produces persistent Failed, never Ready.
- Fixed preparation publication records zero calling-thread heap allocations or
  frees across 128 rounds of seek/hotcue/loop commands. A concurrent 49,999-update
  receipt test rejects torn cue/hotcue/loop snapshots.

The final assembled local suite passes 565 tests (6 opt-in fixtures ignored).
It includes actual GUI Save/Discard/Cancel close coordination, library failure
and same-frame cancellation, startup restoration retry, undo/media identity,
native project preparation reopening, and final-block close/history ordering.
The latter verifies zero callback allocations and frees. `cargo build` passes. No hardware
compatibility, network library access, power-loss simulation or vendor parity is
claimed.
