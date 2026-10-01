# Issue 68: decoded duration in the crate

Filesystem crate entries now carry `Option<f64>` duration instead of using zero
as a sentinel. Unknown lengths visibly say `unknown`; known zero is `0:00` and
fractional seconds remain in metadata while the column displays whole minutes
and seconds. Built-ins retain their known duration.

The accepted file-load path captures duration from the decoded playable buffer's
frame count divided by its own sample rate. Mono and stereo therefore agree with
their actual frame counts, regardless of output rate. Unknown or estimated source
length in decoder diagnostics does not erase a measured buffer length; the
existing incomplete-source warning remains unchanged. Invalid sample rate,
channel count or incomplete interleaved frame cannot fabricate a duration.

Issue 67's existing metadata patch and worker carry this value. Reconciliation
still waits for a Current renderer receipt and current load token, and matches
the original source plus device/inode/size/mtime/ctime fingerprint. A same-file
rescan retains known duration; replacing the bytes invalidates it. A later
BPM-only patch cannot erase a known duration for that same fingerprint. Old scan
candidates merge cached duration before publication, so they cannot briefly flash
an unknown length over a successfully loaded row.

No second decoder or metadata worker was added. Clone, merge and sort remain on
the existing worker; the UI polls and atomically swaps the result, then its normal
source-based selection/cache refresh runs. Selection refresh still scans filtered
indices on publication; this is not a claim that all UI work is constant time.

Five new regression groups cover:

- Real PCM files at mono 44.1 kHz (3.25 s), stereo 48 kHz (4.5 s), and stereo
  96 kHz (1.125 s), through scan, real decoder, renderer receipt and metadata
  publication. Actual egui crate rows display `0:03`, `0:04`, and `0:01`; browsing
  to another item before publication preserves that selection and source.
- A scan held with pre-decode metadata cannot publish an unknown old value over
  the new 3.5-second duration. A BPM-only patch retains that cache. Replacing the
  same path returns to unknown, and loading its new stereo bytes reports 1.25 s.
- Twenty-four real crate frames and Master control submissions/renderer blocks
  complete while a decoder is deliberately held. Changing the file before that
  decode returns prevents its old duration from entering the crate.
- Unknown, known zero, fractional and invalid display values, plus a real invalid
  audio file that stays unknown after failure. Existing issue-67 rejected,
  cancelled and superseded load tests also assert no duration is published.
- Two patches queued before the first worker poll retain a pending duration when
  a BPM-only patch replaces it for the same fingerprint; a new fingerprint
  cannot inherit either pending or cached duration from the old bytes.

These are private local files and headless native UI/engine tests. No hardware,
live library, live audio server or user media was changed.

Validation: all 324 tests passed and the production build passed. The strengthened
pending-coalescing regression was also rerun after adding the different-identity
queued pair. The new test module passes rustfmt and `git diff --check` passes.
