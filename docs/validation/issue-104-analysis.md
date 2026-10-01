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

Final assembled ordinary, long-source, native and release-gate results are
recorded here after execution. Physical controllers, backend XRUNs, human
listening and screen-reader acceptance remain separate user QA.
