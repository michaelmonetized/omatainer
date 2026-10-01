# Music imports and watched removable libraries

Issue #107 stacks above final #106, cebfb53. This layer adds explicit files and
folders, saved watched roots, UUID-based removable sources and guarded resolution
through deck loading, analysis, sampler preparation, cue qualification, relocation
and history. Final release qualification is recorded below when completed.
Physical external-drive unplug/reconnect remains reserved for user QA.

## Product behavior and ownership

The Library dialog accepts one absolute path per line and merges readable music.
The existing filesystem worker performs traversal, progress, cancellation and
bounded skipped-reason publication. Limits are 64 inputs, 64 folder levels, one
million visits and 100,000 rows. The first 32 skipped path/detail samples are
bounded to 1024/256 bytes. Descendant symlinks are not followed. Overlapping
canonical paths and logical volume paths do not duplicate rows.

The same worker owns one nonblocking inotify descriptor, at most 4,096 directory
watches, two-second idle mount checks and a full-scan hint every 30 idle seconds.
Hints coalesce; real scans wait for Studio, current catalog persistence and Close.
Filesystem calls can delay completion. Notifications are not a completeness proof;
the fallback catches missed events, additional directories and unsupported watches.
Large summaries and enrollment batches retire on workers.

Catalog schema 7 retains root bindings by active profile and configured path,
with strict migration from schemas 1–6. The sole metadata writer saves enrollment
and bookmarks atomically with the optional scan. Offline bindings survive; root
removal prunes only that bookmark. Tracks, named-crate membership, preparation
and source files are retained. Imported catalogs never activate foreign roots.
Startup waits for the existing catalog before scanning; essential metadata survives
cancellation of optional scan work.

Previous File records enroll under UUID/relative paths while keeping TrackId and
old aliases. Changed fingerprints create fresh preparation. Matching preparation
and history can return only after actual measured content-hash qualification;
UUID, pathname and title are not content proofs. Missing status comes from guarded
inspection of the exact path on a visible mounted volume, never an absent entry
in an incomplete traversal. The selected row displays the last scan observation;
o scan deletes tracks. Offline, not-visible, missing, unreadable, ambiguous and
changed remain distinct.

## Linux resolution and media workers

Worker-only discovery reads bounded mountinfo and installed libudev block data.
Filesystem UUID is separate from current namespace, mount ID/root/point and
filesystem/block-device guards. Duplicate UUIDs, ambiguous overmounts, foreign
nested mounts and symlinks are refused. A known enrolled UUID remains resolvable
when bus/removable classification later changes. Unsupported UUID formats on
unrelated blocks do not disable local files.

Real-host checks exposed Btrfs anonymous/subvolume device numbers: mountinfo and
file stat can legitimately differ. Btrfs roots now require the read-only FS_INFO
filesystem UUID to match block discovery; another anonymous device is accepted
only with that same verified UUID. No mounts, formatting or privileges are used.
Primary interfaces reviewed: [mountinfo](https://www.kernel.org/doc/html/latest/filesystems/proc.html),
[libudev](https://github.com/systemd/systemd/blob/main/src/libudev/libudev.h),
[Btrfs FS_INFO implementation](https://github.com/torvalds/linux/blob/master/fs/btrfs/ioctl.c)
and the installed Linux btrfs.h argument layout.

Typed source identity survives separate worker I/O resolution. Production loads
open a regular, non-following, nonblocking descriptor; removable loads hash and
decode that same descriptor, then recheck path fingerprint and mount identity.
The content-proof limit is 8 GiB. Analysis and sampler preparation use equivalent
same-descriptor guards. Offline, swapped and changed sources cannot publish PCM
or preparation. Content proofs persist through the existing metadata owner.

## Functional evidence

The assembled component run passed all selected resolver, loader, analysis,
sampler, catalog, scanner, metadata/store, preferences, cue and history groups:
`issue-107-assembled-components-v1.log`. The root-save/cancellation regression also
passed (`issue-107-root-save-v1.log`). Actual inotify events from nested WAV files
coalesce, defer under protection and produce a real subsequent scan; catalog JSON
writes do not trigger a self-rescan loop.

Actual AccessKit focus, typed path text and Library controls exercise import,
stale-button retirement, File-to-volume enrollment, saving/restart, offline and
reconnect snapshots, cue reload and root removal. Every output callback in that
flow equals a separate playing reference bit-for-bit and includes nonzero audio.
The ordinary portable version uses explicit software mount snapshots over real
files; it does not represent a physical remount.

Three opt-in tests passed on this host's actual block filesystem in 0.53 seconds
(`issue-107-local-block-workers-v5.log`): GUI enrollment/playback, analysis and
sampler preparation/verified relocation. Only initial removable classification
is injected; UUID discovery, current mount/Btrfs guards, production resolution,
descriptor decoding and hashing use actual host interfaces. The simulated offline
case remains software evidence. These checks do not prove USB detection or unplug.

Earlier failed development runs remain retained. They exposed strict legacy
fixture fields, root-bind trailing separators, generic proof-error text, and
known-UUID/Btrfs access mismatches. Corrected positive-count tests passed; an
accidental zero-test exact filter is not counted as evidence.

## Final qualification

Pending full ordinary regression and unchanged source-bound local release,
native, show and package checks. This checkpoint makes no completed PR claim.

## Remaining user QA

Use actual removable music with nested folders: import, play/cue both decks,
unplug, reconnect at a different mountpoint, remove/re-add roots and restart.
Confirm stable track/crate identities, explicit offline versus mounted missing
status, preserved cue preparation after verified same bytes and uninterrupted
other-deck output. Physical controller operation, driver delivery, listening,
latency and XRUN results remain separate from these software tests.
