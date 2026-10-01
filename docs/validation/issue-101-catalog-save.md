# Catalog save identity during sampler integration

The sampler content-proof work exposed the catalog's older backup/save pattern.
A deterministic fault-injection regression reproduced four failures: an external
in-place write or path replacement, either during backup or immediately before
commit, was overwritten and its identity accepted by a later retry. All four
cases failed on the original implementation; the red log is retained as
`issue-101-catalog-store-red.log`.

The catalog now binds reads to the descriptor whose bytes were parsed. Backup
preparation retains the original descriptor and compares inode, length, mtime,
mode and owner before accepting only link-related ctime changes. It checks the
primary and owned temporary again before rename, binds postcommit identity to
the written descriptor, and preserves unrelated temporary replacements during
cleanup. Postcommit diagnostics say that a replacement was committed. The
writer explicitly unlocks before dropping its file, so a retained fork/dup
handle cannot extend an already finished writer's lifetime.

These checks detect changes at the tested boundaries. Advisory locking still
requires cooperating writers; this is not an OS compare-and-swap against an
uncooperative writer racing the final check and rename. No format migration,
lock retry or relaxation of concurrent-writer exclusion is involved.

The first green library run passed 68 groups with four test threads in 1.15
seconds, including all four overwrite cases, retained-lock-description ownership,
unknown temporary preservation, and the existing actual killed-writer checkpoints.
Final checks additionally cover postcommit external changes and diagnostics;
those results are recorded after the final combined run.
