# Issue 26: atomic executable publication

The installer now copies the Cargo-built ELF into a unique file in the destination directory, sets mode 0755, flushes it, and compares its size and SHA-256 against the build output before publication. Replacement uses a same-filesystem rename. The old executable inode is retained as `omatainer.previous` with its prior mode and ownership; symlink destinations and files owned by another user are rejected before replacement.

Running instances continue executing their original inode. The installer does not terminate or restart a session: users close and relaunch when ready to load the new version. This avoids interrupting a live set. To roll back just the executable, run the same helper with `omatainer.previous` as source and `omatainer` as destination; the displaced current version becomes the new backup. Desktop/config rollback is separately scoped to #27.

Verification uses compiled temporary C executables in a temporary directory, without touching the installed app or user configuration:

```
python3 scripts/test-install-executable.py
bash -n scripts/install-omarchy.sh
```

Eight tests prove old-process identity via `/proc/PID/exe`, successful new launches, executable backup and rollback, correct mode/ownership, complete old-or-new bytes during 24 replacements, staging failure cleanup, publication failure, rollback after a post-rename sync failure, and rejection of invalid inputs/symlink destinations. Failed rollback attempts preserve both the current executable and the preexisting `.previous` bytes, mode and inode through backup publication, directory-sync and executable-publication failures. A same-inode backup rename also leaves no temporary links.

Both original inodes remain linked privately until publication succeeds. If the filesystem also rejects recovery, the helper attempts both restorations and reports the retained original executable paths instead of deleting the only recovery copies. This failure path is exercised with injected errors in temporary fixtures. All eight tests pass locally. No live desktop installation or restart was performed.
