# Issue #97 — batched recovery journal

Recovery writes independent, private recovery copies of named and untitled projects. It never overwrites an explicit `.omat` save. Restart offers verified inactive sessions; Preview reports content and problems without replacing the open project. Restore uses the existing Save / Discard / Cancel and revision-checked installation, starts stopped, and leaves an unsaved untitled copy.

## Durability and scheduling

This is a **batched full edit-state journal**, not a synchronous per-command journal. Dirty state becomes eligible about every two seconds. The separate checkpoint interval defaults to 30 seconds and compacts journal segments; it does not set the dirty-record cadence. Capture contention, delayed rendering, worker backlog and filesystem I/O can increase the loss window. The UI shows the confirmed durable captured-state age, revision and sequence, with the later commit acknowledgment labeled separately; it warns when newer state is pending, and never advances that timestamp on a failed write or committed directory-sync warning. A backward wall clock reports an unavailable age.

The dedicated worker captures through the existing bounded project exchange, then serializes, hashes, writes, fsyncs and retires media off the GUI and audio threads. GUI updates coalesce into one latest snapshot; explicit requests have one pending slot and results two slots. The worker lowers its own Linux scheduling priority when permitted. No scheduling or physical audio deadline guarantee is inferred from that request. Explicit project operations cancel automatic capture and use a bounded worker-only canceled-reply fence; they do not steal another active project transaction.

Automatic journal capture, durable rotation and intentional-exit retirement remain essential in performance mode. Full discovery hashes sidecars and is optional guarded work, as are Preview and explicit deletion. Protected startup only checks root metadata and reports that copies may exist; it does not report an unverified empty inventory. Protection cancels outstanding inspections and invalidates already displayed verified lists/previews, including a quick protect/unprotect transition. A fresh Refresh in Studio is required.

## Storage contract

The default directory is `$XDG_STATE_HOME/omatainer/recovery`, or `$HOME/.local/state/omatainer/recovery` when the first path is absent or relative. Directories are private `0700`, records are `0600`, and symlink/untrusted-owner inputs fail closed. Nonblocking session locks exclude live instances; a root mutation lock serializes quota reservation, publication and cleanup across processes.

Each record has versioned framing, document epoch, sequence, length, previous hash and SHA-256. Media sidecars contain native project-codec bundles with embedded PCM; persistent identity includes content and shape, and restart re-verifies the sidecar digest. An in-session `Arc` identity is only a reuse cache. A malformed or incomplete final record stops replay with a bounded report. Older verified generations remain recoverable when a newer generation or its assets are damaged. Missing original media paths are informational if verified embedded PCM remains available; missing required sidecars cannot silently remove audio from a recovery.

Preferences version 5 migrates versions 1–4, retaining startup performance protection and existing audio choices. Saved profiles expose:

- Checkpoint interval: 5–3600 seconds, default 30.
- Retained generations per session: 2–20, default 3.
- Global recovery storage cap: 64 MiB–16 GiB, default 2 GiB.

Quota includes media, records, checkpoints, and staging across sessions. Accounting is logical unique-inode bytes, not allocated filesystem blocks; the UI qualifies last-write accounting and warnings rather than presenting an unmeasured value as zero. Storage exhaustion refuses a new commit and preserves prior recovery. It never deletes another session or the only useful copy to make room. Retirement records intent durably before entering a resumable cleanup namespace; unknown/malformed markers are preserved and reported.

Limits are 8 MiB JSON per record, 64 MiB / 256 records per segment, 128 sessions, 100,000 filesystem entries, and 32 diagnostic rows of at most 2048 UTF-8 bytes. Discovery and retained-generation pruning share a 16 GiB hashed-byte ceiling per pass. Recovery checks the native aggregate 1 GiB PCM / 8 MiB metadata limits before allocating the next sidecar. File verification checks cancellation every 64 KiB; a blocking OS syscall is not claimed interruptible.

## Validation

All fixtures run locally on Linux aarch64 with private temporary roots; none changes the user's live recovery, project, device or desktop configuration.

- Storage fixtures exercise actual child-process `SIGKILL` at journal-append, media-publication and checkpoint boundaries; the parent reopens and validates the prior confirmed durable record. Further fixtures cover ENOSPC, cancellation before and after commit, directory-sync warnings, malformed/reordered/oversized tails, missing/corrupt assets, immutable-sidecar mutation, permissions/symlinks, locks, quota, bounded replay, aggregate decode/hash budgets, rotation and resumed retirement cleanup.
- Real `App::update_frame`/AccessKit and renderer fixtures Save As, persist preferences, restart, Preview, cancel, Restore through unsaved decisions, and verify byte-for-byte preservation of the original explicit file. Tests cover edits during preparation, canceled automatic captures before New/Open/Restore, canceled exit after committed retirement, late committed deletion, missing provenance versus required sidecars, and Save As provenance changes without a creative edit.
- Dynamic status, reports and candidate content use isolated UI identity scopes. A controlled regression retains the previously exposed native Preview/Restore node, changes the real worker to a paused automatic append without redrawing, then activates that original node. This reproduced lost actions before the fix and now exercises actual preview and project installation.
- Full-disk injection runs through the real background worker and visible UI status. It retains the previous durable age; removing the scoped injected fault allows a new confirmed durable record.
- Repeated automatic two-second capture and actual disk writes run while the production `OutputCallback` renders. Every warmed callback is checked for zero allocations and frees, and output is compared sample-for-sample with an identical reference renderer. Callback wall observations are diagnostic, not hardware deadline or XRUN evidence.
- Performance-mode fixtures prove essential journaling continues while optional discovery/inspection/restore is refused or canceled. Preview publication and later invalidation are exercised separately.

Process-kill and injected filesystem failures are not physical power-loss qualification. No physical audio, controller, Orca or human usability testing is claimed.

### Final combined results

Base: final performance-mode layer `acceca5`. All Cargo work used the private local `/home/michael/Projects/omatainer-work/issue-35/target`.

- `cargo test -- --test-threads=4`: **744 passed, 0 failed, 13 opt-in fixtures ignored**. The ordinary storage parent explicitly launches and kills the guarded crash-child fixture; its ignored marker does not omit that crash test.
- `cargo build`: passed. Existing unused/dead-code warnings remain; no warning-free claim.
- `cargo test ui::atspi_tests --no-run`, then `python3 scripts/check-accessibility.py --test-binary …`: passed against the fresh binary, with 235 native nodes and 696 real App frames. This preserves native value/action, sample/synth gates, project Save/reopen/Undo, preferences, help lesson and performance-mode workflows; the new recovery-specific flows are the actual egui/renderer fixtures described above, not a claimed additional native screen-reader exercise.
- `python3 scripts/license-manifest.py check --binary …`: passed for the production binary and updated source-bound notices, including SHA-256 dependencies.
- Generated offline manual consistency and `git diff --check`: passed.

The previously intermittent missing-provenance workflow also passed five consecutive isolated repetitions after the UI identity correction.

Local final evidence logs: `/tmp/issue97-full-id-fix.log`, `/tmp/issue97-build-id-fix.log`, `/tmp/issue97-native-id-fix.log`, `/tmp/issue97-license-id-fix.log`, `/tmp/issue97-missing-provenance-fixed-repeat.log`. The pre-fix deterministic failure is `/tmp/issue97-action-race.log`.
