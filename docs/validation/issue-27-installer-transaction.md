# Issue 27: transactional Omarchy installation

The existing shell entrypoint delegates to `scripts/install-transaction.py`.
Installation builds the executable before changing any application or desktop
file, snapshots all affected files and modes into a private journal, then stages
the complete prospective integration. Existing plugin extras are included in
staged validation and remain untouched in the installed plugin.

Required validation covers executable format/copy checks, Lua syntax, hook shell
syntax, desktop entry validation, plugin validation, JSON/JSONC, SVG XML, and
current Hyprland configuration errors. Staged files and backups are verified
again immediately before publication. Missing required tools or invalid config
fail before application/configuration mutation; symlink targets and plugin
symlinks are rejected rather than followed or overwritten.

Every target mutation has a durable journal entry before it happens. Files publish
by same-filesystem rename; the issue 26 helper publishes the executable and its
`.previous` version, and both participate in the surrounding config rollback.
On publication, required reload or installed-config validation failure, rollback
restores previous file contents/modes and removes newly created integration
directories, then reloads the restored desktop when a reload was attempted.
Failures report the command/path and its error. Optional desktop/icon cache
refreshes run after a successful transaction; absent commands are skipped and
command failures produce warnings without undoing the valid installation.

A process lock serializes install/recovery operations. An interrupted or failed
recovery journal blocks another install until recovered. Successful journals and
old files remain under `~/.local/state/omatainer/installations`; the installer
prints the exact path. Recovery verifies the journal's user root and backup
checksums and preserves unexpectedly changed target files rather than overwriting
them. If restoration or desktop reload fails, the journal/backups remain and the
error supplies a recovery command:

```sh
scripts/install-omarchy.sh --recover /path/to/install-transaction/journal.json
```

The transaction state directory and its stable lock file are installation
metadata, separate from application/configuration targets. Staging and all
targets must share a filesystem; a different mounted target is rejected before
publication. Existing file ownership must match the installing user. Running
Omatainer sessions retain the old executable inode and are not terminated.

## Validation

```sh
python3 scripts/test-install-transaction.py
python3 scripts/test-install-executable.py
bash -n scripts/install-omarchy.sh
```

Twelve transaction tests and eight executable-publication tests pass locally.
The transaction harness uses compiled old/new ELF fixtures, real configuration
files, and an explicit temporary `--user-root`; it never changes `HOME`. Journal
storage is also temporary and intentionally outside the compared user tree.
Cargo and desktop integration commands are isolated stubs. Lua, Bash and desktop
file syntax checks inspect only staged fixture files.

The harness observes 26 target mutation boundaries (12 publication operations and
14 new directories) in a fresh installation and injects an exception after each.
Every resulting user tree exactly matches the prior file bytes, modes, directory
set and unrelated data. Additional cases cover existing current/previous
executables, failed atomic config rename, build/validator/reload/config-error
failure, invalid JSON/Lua, symlink rejection, corrupt staging/backups, concurrent
install rejection, wrong-root recovery, recovery after a failed rollback,
idempotent repeated installation, commented-out integration lines, retained
plugin extras and the real shell entrypoint. Required reload failures are retried
only after files are restored; optional cache failures remain visible warnings.

No user-installed executable, desktop file, plugin or configuration was modified,
and no live desktop reload or physical controller QA was performed.
