# Issue 67: BPM provenance and crate reconciliation

Crate BPM is now a typed optional value with one of five origins: unknown,
filename hint, heuristic estimate, explicit user correction, or built-in tempo.
Rows display `hint`, `est`, `user`, `built-in`, or `unknown`; their tooltip explains
that filename hints and heuristic estimates are unverified. The existing BPM
heuristic still returns its existing estimate/fallback. No calibrated confidence
score or tempo-detection accuracy is claimed. Loaded deck status includes the
chosen BPM and its provenance, while the actual submitted sample uses that same
value.

Successful decoding alone does not change crate metadata. The original load
state retains its source and fingerprint, and reconciliation occurs only after
its renderer receipt is Current and its loader token is still current. Queue
rejection, cancellation, superseded receipts, and changed files cannot assign
analysis to the currently browsed row. An explicit User value wins over analysis
before sample submission, worker reconciliation, and same-file rescans. This
change does not introduce a BPM correction editor or an active-deck correction
command.

The shared file fingerprint includes device, inode, size, modification time and
change time (both with nanoseconds). Decoder I/O runs on its existing worker and
only reports a fingerprint if pre/post-decode fingerprints match. Fingerprints
are cache invalidation identities, not content hashes. Metadata is cached in
memory per source/fingerprint for this application session; a same-path file
replacement invalidates old analysis and corrections.

A single additional crate metadata worker clones and sorts immutable library
versions. Scans also pass through that worker before publication, preventing an
older scan from briefly replacing accepted analysis with a filename hint. The UI
publishes only candidates whose base Arc and metadata revision are still current.
The worker retains both candidate and old crate allocations until publication
or discard is acknowledged, keeping their large deallocations off the UI thread.
Existing source-based selection and viewport restoration survive the swap; cell
format caches refresh on the new Arc. Load completion still searches the current
crate once for an explicit correction, and the existing view rebuild visits the
crate on publication. This is not a claim of constant-time whole-UI work.

Validation uses the real scanner, decoder worker, command admission, renderer
receipts, and egui rendering. A real WAV named with a 155 BPM hint decodes to the
existing heuristic's 120 BPM fallback; tests verify the sample/deck value, labeled
crate/deck output, new sort position, unchanged browsed source and rescan cache.
The intentionally short signal tests provenance rather than heuristic accuracy.
Additional cases cover explicit user precedence, same-path replacement,
replacement during decode, rejected/cancelled/superseded loads, invalid/unknown
values, an intervening user correction, revision ordering and a delayed scan that
must never republish stale metadata.

Final validation: `cargo test` passed all 319 tests (eight new reconciliation
cases), `cargo build` passed, and `git diff --check` passed. An independent
read-only review of receipt gating, fingerprints, worker ownership, and staged
scan publication found no blocker. No live audio-device or BPM accuracy claim
is made by these tests.
