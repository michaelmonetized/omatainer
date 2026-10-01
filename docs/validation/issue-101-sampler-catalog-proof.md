# Sampler content proof persistence

A fresh sampler hash/decode result can qualify exactly one existing catalog
track/version after its renderer acknowledgement is Applied. The producer must
pass only the preparation result's measured proof list. Copying a bank, opening
a project or reading a reusable definition does not establish fresh evidence.

The GUI queues at most 256 deduplicated proofs, with at most one additional
256-proof worker job in flight. It does no filesystem reads or hashing. The
metadata worker requires the stable TrackId, exact source/fingerprint and a
present digest; a conflicting saved digest is refused. Qualification changes
only that version's content hash. It cannot change current-version selection,
BPM, title, play history or manual preparation. An unavailable/full queue reports
an explicit error; worker rejection remains visible in library save status.

The already measured digest is essential metadata. Applying it to the existing
worker baseline preserves it across a cancelled optional scan/import and a
newer UI revision. No optional background hash starts while protected. A failed
save retains the accepted proof in the worker's catalog for explicit retry;
ordinary library durability status remains authoritative.

All 65 library-related groups passed with four test threads in 1.04 seconds.
The new regressions cover exact archived-version qualification after replacement,
wrong identity/source/conflicting digest rejection, bounded/deduplicated admission,
and an actual held-worker cancellation/rebase followed by restart-readable storage
and verified relocation after the original file disappears. The race also proves
that a late heuristic cannot replace saved user tempo or lose newer play history
and manual preparation. The sampler editor's real decoder→Applied integration
and complete assembled sampler suite are validated separately.
