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

Every invocation verifies refused IPv4/IPv6 TCP/UDP socket creation and working
Unix socketpair/listener traffic. Failure to install the filter, unexpected
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
