# Music imports and removable libraries: work in progress

Issue #107 is not yet complete or qualified for a PR. This layer starts above
final #106, cebfb53. Physical external-drive unplug/reconnect remains untested.

## Implemented checkpoints

A worker-only Linux location module reads bounded `/proc/self/mountinfo` and the
installed libudev block inventory, preserving filesystem UUID separately from
namespace/mount/device access guards. It parses kernel path escapes, handles
bind/subvolume roots, resolves a persistent volume-relative identity at a new
mountpoint, and refuses duplicate UUIDs, ambiguous overmounts, changed namespaces,
foreign nested mounts and symlink traversal. Offline, unavailable mounted views
and missing files are separate outcomes. The resolver is not yet connected to
all media workers; no product removable-load claim is made at this checkpoint.

Primary interfaces reviewed:
[Linux mountinfo documentation](https://www.kernel.org/doc/html/latest/filesystems/proc.html),
[libudev header](https://github.com/systemd/systemd/blob/main/src/libudev/libudev.h).
Nine resolver tests pass, including read-only discovery on this real host;
remount, duplicate UUID and bind-mount scenarios use explicit software snapshots.

The existing scanner now accepts explicit file/folder import jobs that merge
rather than discard prior rows. Canonical overlapping inputs are deduplicated.
Readable regular files are checked through a non-following, nonblocking open;
unsupported extensions, unavailable inputs, permissions/entry errors, symlinks,
invalid serializable paths and traversal limits expose bounded reasons.
Limits are 64 inputs, 64 folder levels, one million visited entries and 100,000
library rows. At most 32 path/detail samples are retained (1024/256 bytes each).
A boundary marker does not claim an exact count of unvisited descendants.

Fourteen scanner tests pass, covering deeper-than-six traversal, explicit files,
overlap, bounded skipped samples, live reason counts, cancellation, performance
protection and actual GUI/renderer progress during a held filesystem worker.
The Library dialog accepts actual typed input, imports music through that worker,
shows progress and skip details, and opens saved root preferences. A captured
button ID is retired when the path text changes. Automatic watching is pending.

Nine library-store/UI groups pass, including actual AccessKit focus and typed
text, stale-action refusal, overlapping file/folder input, real catalog saving
and unchanged source fingerprint. Every rendered callback in that import UI flow
was bit-for-bit equal to a separate playing reference renderer and included
nonzero audio. It does not prove physical device delivery, latency or zero XRUNs.

Logs under the local work evidence directory: `issue-107-location-v3.log`,
`issue-107-import-v3.log`, `issue-107-import-ui-v1.log`. The first scanner compile
failed two test-only PathBuf moves; these were corrected before passed checks.

The assembled foundation passes all 1,018 ordinary tests with 22 opt-in tests
ignored in 111.94 seconds (`issue-107-foundation-full-v1.log`). The generated
manual and source/license inventory were refreshed before that build. This is
a functional checkpoint, not final release/show or hardware qualification.

## Required before publishing

- Persistent volume bookmarks for user-managed watched roots, off-GUI coalesced
  change detection and optional-work deferral during performance protection.
- Stable catalog adoption of pre-existing file records without duplicate IDs;
  same-byte qualification across changed remount fingerprints.
- Actual typed-source resolution through deck decode/receipts, analysis,
  sampler preparation/residency, cue qualification, relocation and history.
- Visible offline versus missing-on-mounted-volume status without deleting saved
  tracks on incomplete/unreadable scans or root removal.
- Restart/watch/reconnect/playback workflow checks, full ordinary regression,
  final source-bound release/native/show/package checks and explicit hardware
  evidence limits. No issue closure or #107 PR is claimed yet.
