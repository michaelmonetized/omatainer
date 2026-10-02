# Real audio metadata and reviewed edits

Issue #109 stacks above #108 (`8b3cf02`). Imports/scans and renderer-accepted
native deck loads inspect actual embedded title, artist, BPM and key. Catalog
schema 8 retains field provenance, filename fallbacks, parser/conflict notices,
user overrides and explicit clears. Tags are supplied metadata, never measured
tempo/key. User values precede tags, saved analysis, loader estimates and hints.

The **tags…** dialog inspects one selected track or a captured filtered batch of
at most 4,096 rows. Immutable views retain targets without copying the crate on
the GUI. A review captures exact track/version, edited fields, storage choice
and performance generation. Changed drafts invalidate it; browsing elsewhere
does not retarget it. Unchecked fields remain and checked empty values clear.

## Media and persistence

Pinned Lofty 0.24.0 has compression features disabled. Inspection is bounded to
8 GiB sources, 16 MiB cumulative tag reads, one million read/seek operations,
4 MiB parser allocations, 4,096 fields and 4,096 UTF-8 bytes per text value.
Strict inspection is preferred; incomplete tolerant reads expose a notice and
cannot qualify an embedded rewrite. Parser/decode work stays on existing owners
with cooperative cancellation, outside the GUI and audio callback.

Supported MP3, FLAC, WAV and AIFF rewrites require strict complete tags and are
limited to 128 MiB because the writer buffers container data. Read-only,
hardlinked, non-owned, special-mode or extended-attribute media remain unchanged
and use exact library sidecars. Unsupported format/field precision also uses a
sidecar; integer-only BPM tags do not silently round an exact fractional edit.
Unknown binary frames, duplicate custom comments, pictures, secondary tags and
ancillary chunks are inventoried and preserved, or the embedded write refuses.

A private same-filesystem staged copy must satisfy requested values, raw
unselected-metadata preservation and complete playback-payload equivalence.
The production decoder consumes every audio packet, validates frame extents and
available stream checksums, and rejects incomplete/invalid PCM. Payload identity
binds codec parameters, ordered packet data/timestamps/trims and codec headers;
it is separate from the changed whole-file SHA-256. These measured identities
authorize only the captured within-track rewrite, never an arbitrary replacement.
The new fingerprint/hash preserves TrackId, crates, cues/grids, analysis and
history. Late receipts for proven equivalent old playing bytes remain attributed.

Durable intent precedes atomic exchange, ordered against protection/cancellation
by the shared short commit claim. If exchange won, the actual installed write
still receives an essential catalog save. The displaced original remains private
until confirmed catalog persistence. A final exchange race retains both files
and the recovery record rather than speculatively replacing an external change.
Unconfirmed filesystem/catalog outcomes cannot become a misleading sidecar.

Startup waits for catalog ownership before recovery. Installed media reconcile
the exact journaled transaction; unchanged originals allow staged cleanup.
Conflicts remain visible and retain both versions. Removable recovery requires
fresh UUID-qualified access and exact bytes even when kernel device numbers
change. Local sources cannot guess that relationship. Cleanup and catalog save
are explicitly retryable after worker failure. Each row finishes its filesystem,
catalog and cleanup receipts before the next; partial cancellation retains the
completed rows and separates saved, unchanged-skip and unconfirmed counts.

The pinned Symphonia RIFF 0.5.5 vendor patch subtracts AIFF's eight-byte SSND
header from audio extents, verifies COMM frame count and rejects partial sample
frames while permitting an outer odd-byte chunk pad. All 13 vendor files,
upstream archive/original hashes, modified MPL notices and exact missing crate
license supplements are in the source/package inventory. Audio source files are
not changed by the decoder.

## Functional evidence

`issue-109-tags-integrated-v10.log` passed 53 tests in 6.11 seconds. Actual Unicode
MP3/FLAC/WAV/AIFF fixtures cover immutable reads, strict/tolerant parsing, stream
checksum and truncated-frame refusal, unknown metadata/artwork preservation,
full decode/payload comparison, safe writes, races, cancellation/protection,
durable intent recovery, catalog migrations and exact sidecar persistence.

Eight real App/AccessKit workflows cover selected edits, captured multi-format
batches, changed review events, read-only clears across scan/restart, native
unscanned loads, blocked tag I/O with concurrent essential cue saves, partial
batch cancel/protection/Close, and startup recovery before original retirement.
During a playing-track edit, every callback equals a separate playing reference
bit-for-bit and the resident audio Arc remains the same. This is application
and converted-callback proof, not a physical audio/controller session.

An injected filesystem-owner failure separately reports an unconfirmed outcome
and permits explicit recovery with a fresh operation ID, without scanning or
replacing a library (`issue-109-tag-worker-retry.log`). License/package fixtures
passed eight tests in 3.464 seconds (`issue-109-license-tests-v3.log`), including
altered/extra vendored-source refusal. The source inventory covers 407 files and
19 packaged assets; the offline manual was regenerated and inventory check passed.

Earlier retained test failures identified the disabled-feature compile issue in
Lofty 0.25.4, AIFF bounds, duplicate FLAC comment ordering, malformed fixture year/
vendor data and GUI startup/close-fence assumptions. Those were fixed without
skipping cases or weakening acceptance. A debug build also exhausted local disk;
only task-owned rebuildable incremental caches were removed, and subsequent
native builds disable incrementality. Evidence remains under
`/home/michael/Projects/omatainer-work/issue-109-*`.

The first full run passed 1,100 tests and exposed two outdated assertions: six
package assets before the 13 vendored source assets, and an idle scanner before
startup tag recovery. The assertions now verify the full vendored source/notices
inventory and require obsolete import events to preserve the actual completed
startup state (`issue-109-full-v1.log`). No product behavior or acceptance limit
was removed to accommodate them.

## Release qualification

The final ordinary suite passed 1,102 tests with the existing 25 opt-in checks
excluded in 110.96 seconds (`issue-109-full-v3.log`). An intermediate test edit
attempted to clone a non-Clone scanner state; that compile error was corrected
without a production change (`issue-109-full-v2.log`).

Frozen source `54a9c2655187c2c2f0bae75eac2b972e423f9e0d` passed the unchanged
standard gate on 2026-10-02, 01:05:54.127570–01:13:11.990760 UTC. Local aarch64
affinity was `[6]`, nice 0, SCHED_OTHER, with normal host processes left running.
All eight fixed groups passed three repetitions, including their audio/state/
hash/heap checks. Native accessibility passed 158 actions over 245 nodes and
561 frames using the actual App/renderer and private AT-SPI fixture. The fixed
workload test took 121.76 seconds; controlled optimized binary/test builds and
native preflight are also retained. An initial invocation refused the ordinary
test build's `CARGO_INCREMENTAL` override before executing any workload; the
qualified invocation removed that override and used the gate's controlled build.

Worst repetition values in milliseconds:

| Callback | Render CPU p99 | Callback wall p99 | Callback wall max |
| --- | ---: | ---: | ---: |
| Producer | 0.666542 | 0.785463 | 2.087013 |
| Composer | 0.675917 | 0.829464 | 0.930590 |
| Live DJ | 0.156042 | 0.200627 | 0.301335 |
| Hybrid | 0.873960 | 1.056590 | 1.319675 |

Release SHA-256:
`2ad371e75affbc54c32f4c47af8a8decc07d2b7491c83b204581342afb1aa614`.
Release-test SHA-256:
`e5d848489a09e2959d1bc9264f7e7814512b3abfac2ac9b99bf7e7b2dc55fdaf`.
Independent gate verification, immutable package creation and verification,
seven CLI, six follow-protocol, five runtime-isolation and safe-startup checks
passed. This evidence-only documentation update is outside the source inventory
and does not change the qualified executable or packaged assets.

Local immutable evidence under `/home/michael/Projects/omatainer-work`:
`issue-109-final-performance.{json,raw.json,log}`, `issue-109-final-native.json`,
`issue-109-final-post-results.json`, `issue-109-final-{gate-check,package,
package-verify,cli,follow,runtime,safe-start}.log`, and `issue-109-final-package`.

## Acceptance boundary

Physical controller/drive use, human listening, Orca and backend XRUN freedom
remain outside these software receipts. The prior #107 supplemental history/
show-load wall-max failures remain recorded; a coordinated quiet-host run and
the user's final producer/composer/live-DJ hardware QA are still outstanding.
This layer does not change callback DSP, benchmark ceilings or host priorities.
