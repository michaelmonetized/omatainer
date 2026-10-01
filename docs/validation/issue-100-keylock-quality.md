# Issue 100: key-lock implementation and quality evidence

## Scope and corpus

Qualification uses the production deck renderer and real OutputCallback in a
private offline process. It does not open an audio device. CPU, software content
displacement and startup behavior are separate observations; none establishes
converter latency, backend XRUN performance or a human transparency judgment.

The corpus contains two complete recorded VocalSet 1.2 singing-vowel files,
plus original analytical stereo bass, stereo transient pulses and the complete
procedural Omatainer drums/harmony mix. The recorded files cover one soprano
and one baritone performing the same slow forte arpeggio vowel; they are a
limited musical sample. The source license is CC BY 4.0, with author/DOI
attribution, exact member names, member CRC32/SHA-256 and complete terms in
`tests/fixtures/keylock`. Selected bytes were retrieved from the authoritative
archive and verified individually; the full 6 GB archive MD5 was not checked.
The recordings are test fixtures, not factory-library or production-embedded
media. Original generated corpus material follows the application's MIT license.

The fixed matrix uses forward ratios 0.50, 0.84, 0.92, 1.00, 1.08, 1.16 and
1.50 at 44.1/48/96 kHz output, blocks 64/128/512, with lock on/off. Each run
contains 630 isolated renderer cases, 126 actual callback cases and nine
transition traces. Decode, analysis and file writes are outside timed blocks.
Timers and Rust allocator instrumentation add measured overhead. Canonical
Float32 WAVs use block 128; the other block sizes must produce exact matching
PCM. Every sample must be finite and timed sections heap-free.

`scripts/check-keylock-quality.py` accepts an explicit evidence root through
`OMATAINER_KEYLOCK_EVIDENCE_ROOT` (default: repository target/keylock-quality).
It requires fresh descendant output directories and a 2 GiB export/pack bound.
Native exports embed the compiled source/license inventory. Before and after
execution, the script validates that inventory against the reviewed tree and
checks source, executable, manifest and notices stability. A verified-run receipt
binds these checks to the report hash; comparisons require matching workload
and corpus identities. Runtime checkout fields are explicitly informational.

## Original implementation baseline

The baseline ran from 2026-10-01 07:49:25 to 07:50:09 UTC with other agent CPU
jobs held, on the same local M1 Pro/Linux host used by the release gate. The
preexisting arbitrary-origin overlap-add implementation was compiled with the
new qualification harness. Source identity is embedded in the retained report;
the corresponding clean checkout was `cee1f96`.

- Workload SHA-256: `fd4682051d4a83a1d900b04e205886e3ab9b1c15dca3c9076acd9fd99a5b521b`.
- Executable SHA-256: `d2d66e0a08516bd439bc7fd80baeedcb8b14bf832c41986366905f77716db498`.
- Report SHA-256: `a14686bdf3ec9d470e3b12885d3a95e545395570b5ed520551a37a2618759aaf`.
- Evidence directory: `issue-100-baseline-cee1f96` under the local work directory.

At 48 kHz/block 128, lock enabled, the analytical bass revealed failures that
the prior zero-crossing contract did not catch:

| Playback ratio | 55 Hz source dominant peak (Hz) | 93.75 Hz source dominant peak (Hz) | Projection amplitude at intended 93.75 Hz |
| --- | ---: | ---: | ---: |
| 0.50 | 27.50 | 46.90 | 0.000442 |
| 0.84 | 46.20 | 78.75 | 0.002389 |
| 0.92 | 50.60 | 86.25 | 0.006172 |
| 1.00 | 55.00 | 93.75 | 0.349959 |
| 1.08 | 59.40 | 101.25 | 0.010344 |
| 1.16 | 63.80 | 108.75 | 0.005171 |
| 1.50 | 82.50 | 46.85 | 0.001090 |

The intended tone's source amplitude is approximately 0.35. These are objective
spectral/projection results, not listener ratings. The ordinary one-deck callback
at 48 kHz/block 128 had worst CPU p99 67.08 microseconds and wall p99
72.88 microseconds across these locked ratios. All 765 measured groups had zero
Rust allocations/frees, all rendered samples were finite, all 420 cross-block
comparisons were exact, and no callback deadline exceedance was observed in
this neutral offline workload. Those results do not rescue the musical defect
or qualify a loaded show/hardware configuration.

## Listening acceptance

The comparison script creates 35 deterministic blinded A/B pairs at 48 kHz,
with a separate implementation key and an explicitly unlocked rate-shifted
reference. It carries the corpus attribution and supplies an empty score sheet.
No level normalization, alignment correction or fabricated listening score is
applied. Human blind listening remains part of the user's final QA; objective
render measurements alone cannot mark that listening criterion passed.
