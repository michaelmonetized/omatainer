# Version close and empty-folder follow-up

Follow-up to PR #489, stacked above PR #490. A native application close event
cancels named-version and project-import work, refuses exit while either worker
or import acknowledgment is outstanding, and exposes the settled result before
a later close. Already completed publication remains truthfully reported.

First snapshot initializes a nonexistent or verified empty real directory by
publishing a complete sibling stage. Empty-directory publication rechecks its
identity and contents; the kernel refuses replacement of a nonempty directory,
file or symlink. Existing version stores reopen normally. Foreign contents,
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
