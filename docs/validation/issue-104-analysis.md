# Background track analysis qualification

The application can prepare selected local tracks or a captured filtered crate
without loading them onto a deck. BPM, duration and a bounded three-band waveform
are stored against the exact catalog track version, source fingerprint, content
digest and analysis algorithm version. Unknown estimated tempo remains distinct
from an unmeasured value. Automatic key analysis is explicitly unavailable.

Analysis uses the existing single media decoder at its lowest priority. A
foreground deck or sampler request preempts it; the captured queue pauses with
Retry, Skip and Cancel. The GUI retains immutable row/index views and advances
one source at a time only after the metadata owner's save receipt. Filesystem,
hashing, decoding and cache publication remain on workers. Manual BPM, grids,
cues and unrelated fields survive selective or forced reanalysis.

The immutable waveform cache is private, digest-addressed and bounded. Inspection
verifies saved bytes without source decoding. Missing or corrupt entries become
explicit reanalysis work; unavailable sources cannot inherit another version's
result. Catalog publication has one shared cancellation/commit claim. A save that
wins the claim reports its actual outcome, including post-rename durability
uncertainty. Worker loss never fabricates a confirmed result.

Detailed focused evidence is recorded in the [decoder qualification](issue-104-analysis-worker.md),
[publication qualification](issue-104-analysis-publication.md) and
[GUI qualification](issue-104-analysis-ui.md). Those records distinguish their
earlier integration bases from final assembled checks below. Catalog schema 5
migrates schemas 1–4 without changing track identities or existing preparation.

The long-file fixture uses three independently generated 180-second stereo
48 kHz WAV, FLAC and Ogg files. It drives the actual App, decoder, metadata owner
and native egui actions through cancellation, restart, persistence and fresh-App
cache reuse. During this work a separate thread runs the production four-channel
audio callback against an independently seeded reference. Callback allocation,
finite output, sample continuity, source hashes, manual preparation and creative
revision are checked. This is functional concurrency evidence; the fixture's
external pacing does not establish a realtime deadline result.

The launcher binds the exact three-file corpus, current source inventory and
test executable before and after execution, and verifies the child's embedded
inventory. A failed child leaves its log intact; a new attempt requires a fresh
prepared destination. The peer review found and corrected a missing inventory
loader attribute before the actual long run. Python orchestration checks cover
that path, but do not substitute for running the Rust application fixture.

## Final assembled functional evidence

On the complete #104 tree based on final #103 (`33c1fc5`), all 941 ordinary tests
passed with 20 opt-in/maintainer tests ignored, in 88.86 seconds with two test
threads. The actual manual generator, source inventory verification and seven
Python qualification-orchestration checks also passed.

The long-file run passed on 2026-10-01 from 12:13:10.406862 to 12:13:12.864126
UTC. Each WAV, FLAC and Ogg source decoded to 8,640,000 stereo frames at 48 kHz,
exactly 180 seconds, with a 2,048-bin waveform. Cancellation was observed during
Decoding before the first catalog save; restart saved all three sources, and a
fresh App reused all three caches with source decoding forbidden by the fixture.
Track identities, manual preparation, creative revision 3 and all source hashes
were preserved.

The production four-channel f32 callback ran 1,471 measured 128-frame blocks
(188,288 frames), excluding 256 warmup blocks. During 1.118339319 seconds of host
overlap, its output was finite, nonzero and bit-identical to the independent
reference, with zero measured Rust allocations or frees; the deck remained
playing. The fixture sleeps outside each callback and processes source files
faster than playback time. It does not establish a three-minute wall-time show,
device playback, callback deadlines or hardware XRUNs.

Retained evidence:
`/home/michael/Projects/omatainer-work/issue-104-long-v1/qualification.json`
and its `run/report.json`, whose SHA-256 is
`5199e1521b2a6a611f168e6ad6bba17d72e5ebd0abbb83cd3827ef3fd7815794`.
The source inventory SHA-256 is
`f402cda6ee73ba8f0e0b7746430140f4515e0763277c3c7472a4dd1a8dfc402d`,
the debug test executable is
`e1708dc1212224b3e8141acf5dbc8732b8c9a3a19b81742d56bc8e7d6a059912`,
and the exact three-source manifest is
`6e4c923d252e4701a79ea262723737fdfeeabb8f2c5f3625d4e00f4445602327`.
Before/after bindings and the child's embedded inventory match.

## Final release evidence

The unchanged local release gate passed on 2026-10-01 from 12:13:54.701142 to
12:17:32.609659 UTC, including fresh production/test builds and all eight fixed
workload groups across three sessions. The native Linux AT-SPI preflight passed
129 actions, 243 visited nodes and 1,016 App frames. Its actual local FLAC went
through analysis, catalog save, cache reuse and inspection of the saved
2,048-bin/0.25-second waveform. The action capacity was explicitly expanded from
128 to 160 for the added coverage; its 70-second timeout and timing policies
were unchanged. The earlier 128-action-cap failure remains documented in the
GUI qualification record.

Worst callback wall p99 / maximum, in milliseconds, was producer
0.777254 / 0.884672, composer 0.820546 / 1.768760, live DJ
0.094375 / 0.187210 and hybrid 0.850671 / 1.352799. Those four callback groups
had zero observed wall/render-CPU deadline exceedances. Measured callback and
recording allocations/frees, rejected commands and MIDI drops were all zero.
The large-crate frame p99 was 2.729308 ms; multi-controller frame p99 was
5.218072 ms, IPC round-trip p99 8.608675 ms and MIDI-dispatch p99 3.189435 ms.
These fixed workloads and the concurrent analysis fixture above establish
different scopes; neither substitutes for the user's physical show acceptance.

Release production SHA-256:
`aacd27f66de33fbfe6507eb303044a1d8f0180614baace149d394da6225b6b9f`.
Release test SHA-256:
`4a1965862169cd94f6b87fc0082b6ea556db40e00b2d72de0c39f71e2bf4be23`.
Both bind the source inventory recorded above. Retained gate report, raw data
and log are `/home/michael/Projects/omatainer-work/issue-104-final-performance.*`.
Independent gate verification, package/license verification, seven CLI groups,
six follow-protocol groups, five runtime-isolation groups and actual release
safe-startup checks passed. The package is retained at
`/home/michael/Projects/omatainer-work/issue-104-final-package`.

Physical controllers, backend XRUNs, human listening and screen-reader
acceptance remain separate user QA.
