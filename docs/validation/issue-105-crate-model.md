# Issue 105 pure crate model preparation

This isolated preparation adds `src/library/crates.rs` and its tests only.
The shared Catalog declaration/schema, metadata worker, application modules and
UI are not registered or changed yet. Root 104 owns schema 5; the parent will add
the model as schema 6 after that integration settles. This commit alone does not
make issue 105 closeable.

`CrateForest<K>` uses the existing Catalog TrackId as K at integration, with a
known-track predicate. It contains ordered roots, ordered children and ordered
unique memberships; members can overlap across crates. Updates clone a bounded
candidate, apply, validate the whole graph and advance a checked local revision
before replacement. No-op updates keep their revision. Failed edits/imports
leave the original serialized state unchanged.

Limits are 4096 crates, depth 32, 256 UTF-8 name bytes, 100000 members per crate and
250000 total memberships. Validation checks reachability/cycles/single parents,
unique sibling names, known tracks and byte-bounded names without recursive tree
traversal. The later Catalog reader must retain its pre-deserialization 64 MiB
limit. The model has no file I/O or audio operations.

The actual source module compiled directly as a temporary standalone Cargo
library using the repository's pinned serde 1.0.229 and serde_json 1.0.151. It was
not copied into a substitute implementation. Twelve groups passed in 1.96 s:
ordered nesting/overlap/roundtrip, cross-crate moves and idempotent copy, subtree
moves/deletion, rejected cycles/anchors/stale edits, name/depth/node/member
bounds, revision no-ops/overflow, strict serialization, import conflicts and
stable identity independent of mocked catalog location/version ordering.

Command used:

```sh
CARGO_TARGET_DIR=/home/michael/Projects/omatainer-work/issue-35/target cargo test \
  --manifest-path /home/michael/Projects/omatainer-work/issue-105-model-harness/Cargo.toml --offline
```

The temporary manifest's `[lib] path` points directly at this module and has
only serde/serde_json dependencies. Evidence is `/tmp/issue105-model-tests.log`.
Agent 4's read-only transaction/import/bounds review found no blocker.

The external approved architecture and remaining real worker/disk/UI/native,
refresh/relocation/source-file-preservation and performance qualifications are
in `/home/michael/Projects/omatainer-work/issue-105-plan.md`. No restart, disk
persistence, callback timing, physical-controller or complete UI claim follows
from these model-only tests.
