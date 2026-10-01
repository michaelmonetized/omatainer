# Missing-media search and reviewed relocation

Issue #108 stacks above #107 (`ac905f5`). The existing selected-path relocation
now also has a bounded replacement-root search and an explicit review of every
byte-identical copy. No path is chosen automatically, including a sole match.
This layer does not move or delete audio files.

Search runs on the existing filesystem scanner. Its read-only catalog capture,
progress, cooperative cancellation and immutable results do not occupy the
metadata writer; essential cue/history saves remain available. Work admission
and completed reviews retain their performance generation. Root edits, new
searches and cancelled/protected work invalidate old choices. Result storage is
bounded and pinned for final destruction on the filesystem worker.

Saved content hashes allow genuinely missing originals. Otherwise the original
must remain readable, is hashed with stable descriptor/path checks, and its
measured digest must be saved before a searched candidate can commit. Search
compares complete SHA-256 bytes, with length used only to exclude impossible
matches. It includes renamed files regardless of extension. Multiple identical
copies remain ambiguous locations requiring the user's selection.

Bounds are 64 roots, depth 64, one million visits, 100,000 inspected files,
4,096 guarded directories, 64 GiB hashed and 256 matches. Descendant symlinks,
unavailable entries, foreign nested mounts and limits are disclosed with
incomplete coverage, counts and up to 32 bounded samples. An explicitly selected
root on a nested mount is searched separately. Changed directories, mounted
locations or candidate fingerprints discard the result rather than publishing
an apparently complete review.

The sole catalog writer rechecks a selected candidate's captured mount access,
fingerprint and full digest before saving the latest catalog state. Stable
TrackId, cue/grid preparation, crate membership, history and old attributable
aliases survive. A pre-rename failure restores the old optional association and
retains essential edits for retry. An already replaced catalog remains the
actual state when final durability is unconfirmed, with an explicit error.

## Functional evidence

Five bounded backend tests cover moved/renamed trees, duplicate filenames with
different bytes, identical copies, overlapping roots, stale target identities,
missing unqualified originals, measured original digests, cancellation, limits,
skipped inputs, changed mount inventories and changed reviewed files. The
58-test library run passed in 0.40 seconds (`issue-108-search-v2.log`). These
fixtures use actual files with deterministic byte content; they are not physical
USB or listening tests.

Production-loader GUI tests use actual WAVs, AccessKit focus/click events and
text entry. Every rendered callback equals a separate playing reference
bit-for-bit and includes nonzero audio. They cover explicit ambiguous selection,
obsolete root/choice events, candidate replacement, a fresh successful review,
original-proof persistence, save-conflict rollback, and completed-review
invalidation across Performance protection and edited roots. A held filesystem
inventory call proves cue edits can save while search remains blocked; cancelling
that search leaves its association unchanged. Metadata is reopened from disk to
verify persistence. Software mount inventory coverage checks that an explicit
nested root is not pruned merely because its parent is also selected.

The final ordinary suite passed 1,048 tests with 25 opt-in tests excluded in
105.11 seconds (`issue-108-full-v2.log`). The earlier full run passed 1,047
tests before the explicit nested-mount regression was added. The offline manual
was regenerated and the source/license inventory validates.

## Source-bound release qualification

Source commit `4db0f53` passed the unchanged standard gate on 2026-10-01,
21:45:16.113186–21:51:30.255814 UTC. The local aarch64 process used affinity
`[6]`, nice 0, SCHED_OTHER, with normal host/user processes left running. All eight
fixed groups passed three repeats each, including their audio/state/hash/heap
checks. Native accessibility passed 158 actions over 244 nodes and 565 frames;
it is the actual private AT-SPI/App/renderer fixture, not a desktop Orca run.

Worst repeat values in milliseconds:

| Callback | Render CPU p99 | Callback wall p99 | Callback wall max |
| --- | ---: | ---: | ---: |
| Producer | 0.664875 | 0.771546 | 0.991755 |
| Composer | 0.674290 | 0.818838 | 0.933214 |
| Live DJ | 0.156041 | 0.196126 | 0.267293 |
| Hybrid | 0.872583 | 1.044964 | 4.790611 |

Release SHA-256:
`9c919b9dc53d1524dab893c5e09bb94df9add2eab77c124139fb40b3aa5e0b1f`.
Release-test SHA-256:
`2c8625d497c458a08765d5a19522d605469c5bbe1fb91f2a83ae78420a0c1d8d`.
Independent gate verification, immutable package creation/verification, seven
CLI, six follow-protocol, five runtime-isolation and safe-startup checks passed.
The package records source `4db0f53`; this evidence-only documentation update
is outside the source inventory and does not change the tested executable.

Local evidence under `/home/michael/Projects/omatainer-work`:
`issue-108-final-performance.{json,raw.json,log}`, `issue-108-final-native.json`,
`issue-108-final-post-results.json`, `issue-108-final-{gate-check,package,
package-verify,cli,follow,runtime,safe-start}.log`, and immutable
`issue-108-final-package`. The gate's workload test completed in 120.85 seconds;
the preceding controlled release/test builds are also retained in its log.

## Acceptance boundary

No physical drive removal, controller/hardware run, human listening, Orca session
or backend XRUN qualification is claimed. The prior #107 supplemental
history/show-load wall-max failures remain recorded in `issue-107-import.md`;
the coordinated quiet-host checks and the user's final producer/composer/live-DJ
hardware runs remain outstanding. This change does not modify callback DSP,
benchmark ceilings, host priorities or user processes.
