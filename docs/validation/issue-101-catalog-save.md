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
The normalized sampler-core run on commit `0bafa84485d70983f24ef65040adca373ec44c69`
passed 851 ordinary tests with 15 opt-in tests ignored in 51.15 seconds, including
all six in-place/replacement cases at checkpoints 2, 3 and 4. This covers
postcommit diagnostics as well as the recovery and sampler ownership changes.
The retained log is `issue-101-normalized-full.log`; final editor-union and
release qualification are recorded separately.
