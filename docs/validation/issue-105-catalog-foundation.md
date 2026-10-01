# Issue 105 catalog and worker foundation

This layer implements persistence and the catalog-owner API. It does not yet
provide the crate manager, selected-crate browser/controller view, or native
accessibility workflow. Issue 105 remains open pending those UI and integration
checks.

Catalog schema 6 adds an explicit ordered CrateForest<TrackId>. Schemas 1–5
migrate to an empty forest without rewriting during read. The next catalog
save backs up the prior bytes. Schema 6 requires its forest; malformed, unknown-field and future files
fail closed. Membership names a stable TrackId, so refresh, same-path version
replacement, missing source media and explicitly verified relocation do not
rewrite it. Manual preparation and per-version analysis stay separate.

Catalog import validates incoming track identities and the independent forest,
then commits their whole candidate together. Disjoint trees append in imported
root order; identical imports are no-ops. Conflicting track IDs, shared crate
nodes/parentage/order or sibling names reject the whole import. Creating a new
child beneath an imported existing root is a conflict if that changes the
root's saved child list; import does not guess an order or merge shared edits.

Metadata::edit_crates(expected_revision, CollectionAction) returns a
cancellable token. Actions are Create (fresh worker-generated identity), Edit
(the stable-ID model operation), or Read. The sole existing metadata worker
handles all work. One outstanding collection operation includes its unconsumed
terminal result, so a later edit cannot overwrite an earlier outcome. Read is
available under performance protection. New requests are refused during
library close; existing accepted work is included in the normal close wait.

A request admits at most 4096 selected members. The pure model still supports
100000 members per crate, 250000 total memberships, 4096 crates and depth 32.
Admission checks only bounded payload shape; candidate cloning, duplicate
checks, tree validation, JSON and disk work occur on the worker. No new audio,
MIDI, filesystem or decoder thread is introduced.

Mutation first prepares and validates a private candidate. A WorkPermit commit
guard orders mode entry against the transaction, then a pending-to-claimed
atomic transition orders user Cancel against publication. Cancel before the
claim rejects the request; after it, the actual file outcome remains
authoritative. A pre-rename failure rolls back only that candidate, retaining
essential metadata saved before it. A post-rename failure retains the published
catalog and reports CommittedUnconfirmed. Retry persistence does not duplicate
Create. If the worker disappears after a claim, the receipt says Unknown and
requires inspection of the reopened catalog; it does not invent rejection or
success. Large catalog Arc retirement uses the existing worker handoff.

Local validation:

- The complete library test filter passed 108 tests, including the 12 existing
  pure forest groups and 11 new catalog/worker groups. Actual WAV source bytes
  and fingerprint identity stayed unchanged across collection operations.
- New coverage includes schemas 1–5 and malformed/future schema rejection,
  analysis/preparation preservation, nested overlapping order/restart, source
  refresh/replacement/missing/verified relocation, transactional imports,
  terminal-slot saturation, stale revisions, early/late cancellation, quick
  protection cycles, existing close waits, actual pre/post-rename faults, and
  an actual committed save whose worker loses its terminal result.
- Commands used the private local Cargo target: cargo test --locked --offline
  library -- --test-threads=4, and cargo build --locked --offline.
- An independent source review checked the claim, rollback, unknown outcome and
  import transaction paths without finding a blocker.

Full assembled-suite, native manager/controller workflow, performance gate,
generated license inventory and release-package verification remain for the
later complete UI integration. No hardware or issue-closeability claim is made.
