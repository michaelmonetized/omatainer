# Offline-operation qualification

This document starts with the reviewed launcher boundary. Full application and
performance results will be added after the final local workflow runs; these
launcher tests alone do not qualify the application.

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

Application and performance execution results are pending this revision's
qualification run. The already tested launcher alone is not the acceptance
claim.
