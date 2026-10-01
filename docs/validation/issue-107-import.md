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

## Qualification checkpoints

The first assembled source passed all 1,036 ordinary tests with 25 opt-ins
ignored in 117.06 seconds (`issue-107-final-full-v1.log`). All three actual
block-filesystem opt-ins passed again in 0.45 seconds
(`issue-107-final-local-block-v1.log`).

A subsequent admission regression reproduced a pending preference rescan lost
when protection began before admission (`issue-107-deferral-red.log`). Scans now
retain the pending request through protection and Close instead of clearing it.
The corrected source must pass the complete checks below.

The first release checkpoint (f44c134) completed the native preflight and fixed
workloads but failed one unchanged wall-time maximum: live-DJ repetition 3 had
maximum 5.621517 ms versus 5.333332 ms, with p99 0.219954 ms. Its failed JSON,
raw samples and log remain as `issue-107-checkpoint-performance-v1.*`; the source
was subsequently corrected for scan deferral. This run is not qualification.
No policy ceilings, audio goldens or scheduling priorities were changed.

## Final qualification

The corrected source f37fa87 passed 1,037 ordinary tests with 25 opt-ins ignored
in 110.22 seconds (`issue-107-final-full-v2.log`). The three real block-filesystem
opt-ins passed again in 0.49 seconds (`issue-107-final-local-block-v2.log`).
The pending-scan regression now uses the actual private persistent catalog writer
and confirms the root bookmark after returning from protection to Studio.

The first corrected native attempt reached the existing 70-second consumer
bound near the final recovery controls. Its incomplete report and failure log
remain as `issue-107-checkpoint-performance-v2.*`. No timeout was extended.

The unchanged source-bound release gate passed all eight groups × three runs
from 2026-10-01 19:00:13.097321 to 19:02:53.368395 UTC, with CPU affinity [6],
nice 0 and SCHED_OTHER. Native validation passed 158 actions, 244 visited nodes
and 568 App frames under the original bounds. This is a private D-Bus App/renderer
adapter, without a desktop window, physical controllers or Orca certification.

| Mode | Callback wall p99 / maximum, ms | Renderer CPU p99, ms |
| --- | --- | --- |
| producer | 0.807206 / 4.415862 | 0.676083 |
| composer | 0.912748 / 4.564988 | 0.692582 |
| live DJ | 0.214333 / 1.736245 | 0.157917 |
| hybrid | 1.200789 / 5.059446 | 0.908417 |

Audio/state goldens passed, with zero measured callback allocations, frees and
rejected commands. The local host was an M1 Pro, aarch64 Linux 7.1.13-3-2-ARCH,
with ten logical CPUs and load average 14.14 before / 13.64 after the workload.
No user processes, scheduling priorities or policy limits were changed. The
recorded affinity is a qualification condition, not proof that arbitrary busy
host scheduling meets every deadline.

Release SHA-256: `ea123b88d3eb69fe7ad80bbcdb829ce497749998ac0dd21c9aaaf5053a13efb9`.
Release-test SHA-256: `24fd89455db9ff353ec42840a58e2b26dd3a7f017b40cdd9614dd0a5ef6d96f6`.

The first active-history attempt on CPU 6 failed an unchanged maximum-wall check
at 44.1 kHz / 256 frames / 1.50×: p99 1.272331 ms, maximum 18.164307 ms versus
11.609976 ms. Its log and host binding remain as
`issue-107-checkpoint-active-history.log` and `issue-107-checkpoint-timing-host.json`.
A subsequent three-second read-only CPU observation found all performance cores
about 60–63% busy; it does not establish the cause of the individual outlier.
The next fixed matrix uses CPU 7, capacity 1024, with unchanged source, limits,
nice level and scheduler. It also failed at 48 kHz / 256 frames / 0.84×: p99
1.831332 ms, maximum 11.304995 ms versus 10.666666 ms. Its log and host binding
remain as `issue-107-checkpoint-core7-active-history.log` and
`issue-107-checkpoint-core7-timing-host.json`.

The complete additional keylock matrix ran all 18 configurations × three
repetitions on CPU 7. Audio/state goldens, repeated audio hashes and zero
heap/rejected-command checks passed; seven callback-wall maxima exceeded their
unchanged ceilings. At 96 kHz / 128 frames, maxima ranged up to 3.833419 ms
versus 2.666666 ms; one 96 kHz / 256-frame case reached 6.688588 ms versus
5.333332 ms. This is a failed timing gate, retained in
`target/keylock-quality/issue-107-final-show-v1/{verified-showload.json,raw.json,execution.log}`
and `issue-107-final-keylock-show.log`.

The four-source persistence-worker probe also failed its third repetition's
wall maximum: p99 0.731083 ms, maximum 2.850085 ms versus 2.666666 ms at
96 kHz / 128 frames (`issue-107-final-worker-history.log`). A third active-history
attempt failed at 96 kHz / 128 frames / 0.50×: p99 0.789334 ms, maximum
3.556753 ms versus 2.666666 ms (`issue-107-final-active-history.log`). Neither
aborted matrix is represented as a complete pass. Host bindings are retained in
`issue-107-final-timing-host.json`; aggregate results in
`issue-107-final-post-results.json`. OBS was observed consuming about six CPUs;
that observation does not prove the cause of any particular callback outlier.

Independent verification of the passed eight-group release record succeeded.
The immutable `issue-107-final-package` binds committed source f37fa87, exact
executable/license records and the passed standard workload receipt; independent
package verification passed. Seven CLI protocol groups, six follow groups, five
runtime-isolation groups and real headless safe startup passed against the same
final executable. Logs: `issue-107-final-{gate-check,package,package-verify,cli,follow,runtime,safe-start}.log`.
The final source-bound standard evidence is retained as
`issue-107-final-performance.{json,raw.json,log}` and `issue-107-final-native.json`.

The functional import change and standard release gate are reviewable. Additional
history/show timing qualification remains outstanding: repeat those unchanged
probes in the coordinated quiet host window required by
[the show workload contract](issue-100-showload.md). Their failed maxima cannot
be treated as passes or replaced by CPU p99 figures. No final performance,
physical-driver, listening or zero-XRUN acceptance is claimed.

## Remaining user QA

Use actual removable music with nested folders: import, play/cue both decks,
unplug, reconnect at a different mountpoint, remove/re-add roots and restart.
Confirm stable track/crate identities, explicit offline versus mounted missing
status, preserved cue preparation after verified same bytes and uninterrupted
other-deck output. Physical controller operation, driver delivery, listening,
latency and XRUN results remain separate from these software tests.
