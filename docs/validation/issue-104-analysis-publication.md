# Analysis cache inspection and catalog publication

This is foundation evidence; the full UI and ordered-stack qualification are separate.

The existing metadata owner performs all catalog/cache I/O. One analysis operation
remains outstanding until its terminal receipt is consumed; a second inspection or
save cannot overwrite that receipt. Inspections return the captured request ID,
source/version reference, optional-work permit, user-cancel flag, partial stored
record, validated waveform and only the selected fields still requiring analysis.
A missing/corrupt waveform requires fresh analysis while retaining its prior record.
Captured row/index views and inspected payloads return through two bounded
retirement slots to the same worker. A disconnected owner returns an explicit
unconfirmed outcome, refuses new work and leaves retirement payloads with the
caller instead of hanging or claiming that a write did not happen.

Freshly prepared analysis first writes its immutable bounded waveform blob. The
metadata owner validates a candidate catalog, acquires the original optional-work
commit guard, then claims the decoder token's shared cancellation state. Only then
can it replace/save the catalog. Cancellation before claim is inert; after claim,
the actual persistence result wins even if a new request changes token generation.
Unused complete cache blobs may remain after a refused catalog publication.

Catalog storage tracks whether replacement actually occurred. A pre-rename failure
restores the in-memory baseline; a post-rename error retains the candidate and
reports committed but durability unconfirmed. It does not become a confirmed save.
Performance protection and another operation's active commit have distinct errors.
Exact-version measured fields remain authoritative during later automatic upserts;
user BPM corrections, manual grids/cues, unrequested fields and unrelated versions
remain intact.

Nine actual decoder/store/metadata groups passed in 0.02 seconds after compilation:

- Silent-source Unknown BPM, duration and waveform persist, reopen and reuse the
  validated immutable cache through actual store/cache owners.
- Partial inspection selects missing fields only; a corrupt cached waveform asks
  for waveform reanalysis without erasing valid stored BPM/duration.
- Cancel at four controlled publication boundaries, plus newer request admission
  after claim, preserve exact disk state and truthful receipts.
- A protection cycle and another irreversible commit retain distinct outcomes.
- Actual catalog fault injection before/after rename distinguishes unchanged
  memory/disk from a committed but unconfirmed replacement.
- An older decoded metadata patch followed by analysis, delayed GUI publication,
  an essential history capture and protected/unprotected rebase cannot restore
  old automatic BPM/duration or erase manual preparation.
- An unconsumed terminal receipt rejects replacement; a completed inspection
  retains its original cancelled permit through GUI publication.
- Controlled metadata-worker failure returns an explicit uncertain terminal
  result instead of hanging.
- Captured rows and indices have their final remaining owner on the existing
  metadata worker and are retired after GUI acknowledgement.

Log: `/home/michael/Projects/omatainer-work/issue-104-analysis-publication-tests-v2.log`.
This is local functional evidence, not a performance, physical-controller,
hardware-latency, listening or screen-reader qualification.
