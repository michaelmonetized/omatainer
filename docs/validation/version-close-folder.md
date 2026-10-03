# Version close and empty-folder follow-up

Follow-up to PR #489, stacked above PR #490. A native application close event
cancels named-version and project-import work, refuses exit while either worker
or import acknowledgment is outstanding, and exposes the settled result before
a later close. Already completed publication remains truthfully reported.

First snapshot initializes a nonexistent or verified empty real directory by
publishing a complete sibling stage. Empty-directory publication rechecks its identity and contents, atomically
exchanges it with the complete stage, then verifies the displaced inode before
accepting publication. Removal makes exchange fail; a swapped or newly nonempty
destination is restored. Unexpected changes during restoration retain displaced
contents at the reported recovery path. Existing version stores reopen normally. Foreign contents,
symlinks and cancellation remain refusals and preserve their input.

Local Linux aarch64 qualification: 21 focused native/model tests pass (nine
version, six import UI, four portable storage and two existing project-close
fixtures), plus eight license/package fixtures. Native close regression pauses
Snapshot, Branch and Prune independently, sends the actual viewport close event,
checks CancelClose and cancellation, resumes each worker and verifies unchanged
index/branch output. The import close regression covers reviewed and queued,
unclaimed imports. The ordinary native version workflow now starts from an
existing empty folder and still snapshots, compares, branches, restores and
prunes. Model fixtures also refuse foreign folders, symlinks and cancellation.

Debug test SHA: `e3ca9ee4b960370f976beabb1a5d9beeea8b20b2377212bc677da3f939639be1`.
Logs/receipts: `/home/michael/Projects/omatainer-work/version-close-focused.json`
and adjacent `version-close-{versions,import,portable,close,package}.log` files.
This focused layer does not claim a separate full release gate or physical OS
window/hardware qualification; the preceding full gate is retained in
`issue-124-project-versions.md`.

## Destination exchange regression

A subsequent race finding is resolved by checking the inode displaced by
RENAME_EXCHANGE. The deterministic fixture removes or swaps the destination after
its original check: a missing path remains absent, and swapped foreign contents
are restored exactly with no staged files published into them. Fourteen focused
tests pass on this revision (nine version and five portable storage), together
with eight license/package fixtures. The earlier close/import qualification
above remains retained. Updated debug test SHA: `fc31158d0f9f39ed3de219e89554ee969847bfcfc4865aa994c6b67510d752c6`.
Logs: `/home/michael/Projects/omatainer-work/version-folder-exchange-{tests,versions,portable,package}.log`.

## Shared close and safe-mode restart

Version/import cancellation and settling now guards the shared Close request,
its final project-close preparation and the safe-mode Restart normally action.
Restart intent is set only after the worker gate succeeds. Eighteen focused
tests pass (ten version, six native import and two existing close regressions),
plus eight package fixtures. The new test uses the actual safe audio owner,
pauses a snapshot, clicks Restart normally through the native accessibility
request, verifies cancellation without close preparation, and confirms no index
was published after the worker settles. Debug test SHA: `ae664bb7ae207911d0dfa782931c1521f3791f88af60a0a1f13f2d5394c6284a`.
Logs: `/home/michael/Projects/omatainer-work/version-restart-{tests,versions,import,close,package}.log`.
