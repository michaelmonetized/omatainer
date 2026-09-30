# Issue 32: private per-user runtime endpoints

Runtime discovery now returns a validated path or an actionable error before
the GUI acquires single-instance ownership or starts audio. The GUI captures
that path once for the ownership guard and IPC server; CLI requests use the
same discovery function.

An explicit `XDG_RUNTIME_DIR` must be absolute, owned by the effective UID, and
mode `0700`. Every path component must be a real directory rather than a
symlink, including aliases ending in `/.`; parent traversal is rejected.
Ancestors must belong to root or the effective user, and writable shared
ancestors require the sticky bit. Invalid explicit configuration fails closed.
The ownership/mode requirement follows the
[XDG Base Directory specification](https://specifications.freedesktop.org/basedir/latest/).

When XDG is absent, discovery validates the temporary-directory ancestry and
creates `omatainer-<effective-UID>` there with mode `0700`, then validates it.
Existing fallback occupants are never repaired, chmodded, chowned or removed.
Identity comes from `geteuid`, not `UID` or `USER` environment variables. A
warning identifies the fallback once per process. The directory remains across
exits so the sibling instance-lock inode can remain stable. There is no lookup
or cleanup of the former shared `/tmp/omatainer.sock` pathname.

An existing endpoint must be a real socket belonging to the effective user.
Issue 31's locked stale cleanup retains its repeated type/owner/inode check.
Issue 29's normal/error cleanup guard now also records and rechecks the
effective owner, in addition to socket type and device/inode, before unlinking.
Trusted ancestry and private final-directory permissions prevent a different
UID from replacing these paths between checks; same-UID and root actors are
within this trust boundary.

## Validation

- `cargo test --offline`: 141 tests pass, including eight runtime-path tests,
  a cleanup-owner regression, and all issue 31 process/startup tests.
- `cargo build --offline`: passes locally.
- `python3 scripts/check-runtime-isolation.py target/debug/omatainer`: passes
  all five groups through the real CLI. Child environments unset XDG, override
  TMPDIR, spoof UID/USER, and use umask `000`; the fallback remains EUID-owned
  `0700`. Real status requests reach private fallback and valid-XDG listeners.
  Invalid modes, symlink-dot aliases, relative paths, socket-path regular files
  and symlinks, and non-sticky writable temp parents fail without mutation.
- `python3 scripts/check-ipc-cli.py target/debug/omatainer`: all seven existing
  accepted/rejected/malformed protocol cases pass.
- `git diff --check`: passes.

Rust fixtures cover concurrent fallback creation, wrong expected UID, unsafe
directory modes/ancestors, endpoint collisions, and preservation of their
inodes/contents. Owner mismatch is injected by changing the expected identity
or guard metadata, without privileged chown or process credential changes.
No actual cross-UID interference is claimed tested. All sockets are isolated
fixtures; no installed application, real shared endpoint, audio device, MIDI
controller or desktop configuration was touched.
