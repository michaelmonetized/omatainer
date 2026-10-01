# Offline-operation qualification

The source-bound functional qualification below covers the implemented local
application workflows. Performance qualification uses a separate, coordinated
release run; launcher checks alone are not application or timing evidence.

`scripts/offline_guard.py` installs a filter only in its new child, before exec.
It retains the effective UID, enables no-new-privileges, denies creation of every
socket family except AF_UNIX, and denies io_uring setup so that interface cannot
create sockets outside the syscall rules. It closes inherited descriptors above
stdio and rejects socket stdio. The filter persists across exec and descendants.
The parent process and desktop network configuration are unchanged.

Every invocation verifies refused IPv4/IPv6 TCP/UDP socket creation, an actual
io_uring setup syscall returning EPERM, and working Unix socketpair/listener
traffic. Failure to install the filter, unexpected
negative-control behavior, an untrusted receipt directory or occupied receipt
fails before the requested executable runs. Receipts are fresh 0600 files in an
effective-UID-owned 0700 directory, bounded to 16 KiB. The supported guard hosts
are native Linux aarch64/x86_64 with libseccomp and procfs.

The child environment excludes inherited proxy, session/accessibility bus,
display, dynamic-loader, browser and credential variables. HOME retains its
original meaning. The application fixtures use explicit private startup paths;
XDG and fixture-specific Omatainer variables are retained. A private D-Bus tree
must be started inside the guard for native accessibility checks.

This is a direct network-capability restriction for audited app workflows, not
a malicious-code sandbox. AF_UNIX is intentionally allowed for IPC; deliberately
contacting an existing host proxy or receiving a network FD from it would exceed
this evidence. No existing browser, portal or user's session bus is used in the
qualified process tree. Explicit source-record browser links are separate
handoff requests; opening a remote browser page is not part of offline proof.

The launcher tests execute real filtered subprocesses and validate exec
inheritance, same UID, local IPC, inherited FD closure, failure preservation,
no-overwrite/symlink refusal, parent isolation and failure to install. They make
no Internet connection and change no route, firewall, network service, user
desktop setting or application data.

## Functional and timing orchestration

`check-offline.py functional --binary /absolute/omatainer --test-binary
/absolute/omatainer-tests --out /absolute/fresh/o103-run` uses prebuilt local
executables. Use a short private path beneath `/home/michael` on this host:
Unix-domain socket paths are finite. The destination must not exist; symlink
ancestors are refused, reports are create-new, process time/output and aggregate
evidence bytes are bounded. `TMPDIR` and all fixture XDG roots point inside it;
HOME remains unchanged.

The shipped safe CLI runs cold, including malformed preferences, unavailable
support storage and existing-instance refusal. Actual App/worker tests and both
private AT-SPI workflows run inside the inherited filter. A separate actual UI
fixture creates/assigns/trims/applies a sampler bank, loads the local WAV onto a
deck and saves `.omat`; the launcher removes only that fixture WAV. A fresh
process opens through the Project UI, compares PCM hashes, bank identity/range/
gain and notes, then plays the deck and auditions its embedded bank. This is
hardware-free renderer/UI evidence, not a compositor or physical audio claim.
Existing UI suites retain real failure/cancellation/refusal checks. Browser-link
tests inspect `PlatformOutput.commands` and never execute the handoff.

The separate `performance` action is opt-in and requires release executables.
It runs the mandatory native preflight, the original eight workload groups with
three repetitions and the two-keylocked-deck 18-group show load with three
repetitions. It calls the existing strict evaluators without changing workloads,
goldens, deadlines or jitter ceilings. Reports include every actual callback
wall/render-CPU deadline exceedance independently of policy pass. Use a
coordinated quiet host window; no backend XRUN, device latency, physical
controller or listening result is inferred.

Every document/performance receipt embeds the executable's retained license and
source manifest; it must equal the currently validated inventory. Production
and test executable digests, policy digests and source inventory are unchanged
across the run. A failed stage leaves a failure record and prior logs; it cannot
become a pass by reusing a previous output directory. Build dependencies and
source inventory regeneration occur outside the network-denied execution.

## Functional qualification, 2026-10-01

The complete network-denied functional run passed nine stages from
10:50:28.745571 to 10:53:17.727921 UTC. It used the normal debug-profile test and
production executables, not a substitute app or a claimed hardware backend.

- The shipped cold safe-startup CLI passed real startup/capture, malformed-file
  preservation, existing-instance refusal, and read-only diagnostic storage.
- Normal App create/load/trim/Apply/Save passed. A fresh process reopened after
  the fixture WAV was removed, matched deck/bank PCM hashes, bank identity,
  trim/gain and notes, then produced finite nonzero deck and audition output.
- A fresh real safe-mode owner opened that same embedded project, preserved the
  PCM/controls/notes and saved a copy with zero backend callbacks and no MIDI
  manager or output device.
- All 312 ordinary UI cases passed (8 separate opt-in cases ignored), including
  actual local loader/store/project cancellation and failure behavior. All 24
  support cases and 9 native project-codec cases also passed; their 2 and 1
  opt-in cases respectively remained explicitly ignored.
- The normal private AT-SPI workflow passed 123 actions, 242 initial nodes and
  1,109 App frames. The safe support workflow passed 9 actions, 245 nodes and 138
  frames. No existing desktop bus, window, browser or physical device was used.
- Every stage measured IPv4/IPv6 TCP/UDP and io_uring setup refusal plus working
  private Unix IPC. The application child independently checked socket refusal
  after exec. The six launcher and five orchestration safety tests also passed.

Retained local report:
`/home/michael/Projects/omatainer-work/o103-f1/report.json`.
Report SHA-256:
`91f87bd46efef50a864ceb20aa1f258f7938670a81445dc83977758d31021360`.
The source manifest is
`6f7dc45d8a349d4ac70d3fa72ee08131bf71d5ef3a3f9149394bc970c48245ee`;
production executable
`a2460a5aa5d478c9c6b1d75b77c0ef11f931d4e4b37d89be1c6e4cb58b58a149`;
test executable
`e917c4413cfb86787451cf37a10723c09566d134b01f0177815db8f88c7731bb`.
The report retains the full source inventory and each guard/output receipt;
source and executable hashes were unchanged after execution. Its bounded
artifacts total 25,154,827 bytes before the final report itself.

The full ordinary regression suite also passed 895 tests with 19 documented
opt-in cases ignored in 88.17 seconds; the production debug build passed.
These checks preceded the release timing phase.

## Release performance qualification

Pending the coordinated network-denied run against the exact packaged release
executables after the standard release gate. Functional success above makes no
performance-budget, zero-deadline-exceedance, physical XRUN or listening claim.
