# Issue 90: desktop validation and truthful installer results

The transactional installer from issue 27 already rejected nonzero `hyprctl
reload`/`configerrors` statuses and nonempty `configerrors` stdout. A private shell
fixture reproduced a remaining false success: `configerrors` wrote an error only
to stderr and exited zero, after which installation returned zero and printed
`Installed Omatainer`.

All three desktop-validation phases now share one check: current desktop before
publication, installed desktop after reload, and restored desktop after rollback
or explicit recovery. A nonzero status or non-whitespace diagnostic on either
`configerrors` stream fails validation. A reload failure still fails on its exit
status. Diagnostics retain the phase, complete command, exit status and both
output streams, so an stderr message cannot hide the actual stdout error list.
Configuration failures include a repair/recheck/retry instruction. Reload's normal
informational output is not interpreted as a configuration error; its subsequent
`configerrors` validation must still pass.

Required validation failure returns nonzero. Installation restores the prior
files and reloads/validates them before reporting completed rollback. If the
restored desktop is still invalid, its journal and backups remain, the result is
nonzero, and the error includes a shell-quoted recovery command that also works
for user-root paths containing spaces. Explicit recovery likewise reports success
only after the restored desktop validates. No install/recovery success message
is emitted while a required validator is still running.

Binary/config publication, per-install license records, release receipts,
idempotence, optional cache-refresh policy and prior-file verification retain the
existing transaction semantics. The license manifest was refreshed through its
existing update tool; the only manifest change is the reviewed installer source
hash.

## Private Linux verification

The tests run the actual shell entrypoint and Python transaction code with real
old/new fixture files and ELF binaries under temporary user roots. A private
`hyprctl` executable supplies controlled statuses and stdout/stderr, records exact
call order and can pause the final check. No `HOME` substitution, real Hyprland
reload or real desktop configuration mutation occurs.

Six new groups cover:

- Configuration errors on stdout, stderr and both streams, each with zero and
  nonzero exit statuses; all fail and restore the exact prior tree.
- Reload failure retaining both diagnostic streams, plus nonzero configuration
  status with no diagnostic text.
- Existing desktop stderr errors failing before any publication or reload.
- Failed installed validation followed by failed restored validation, retained
  recovery state, another failed recovery, then successful recovery; prior
  executable versions, configuration, license records and receipt are preserved.
- A paused final validator with unbuffered output: no success line and no
  committed journal while paused; clean whitespace-only validation permits success
  afterward. Informational reload output is accepted.

Validation passed: all 20 transaction groups (including the six new groups),
eight executable publication/rollback groups, seven license/package groups,
`license-manifest.py check`, shell syntax validation and `git diff --check`. These are unit/integration fixtures for installer behavior;
they do not claim live-desktop, physical power-loss or controller QA.
