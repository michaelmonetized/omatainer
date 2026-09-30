# Issue 31: preserve live IPC endpoints and serialize startup

Application startup now acquires a kernel-backed exclusive instance lock before
opening the audio/MIDI engine or constructing a GUI. The guard remains alive
until the IPC server and GUI have shut down. Simultaneous launches that find the
lock held focus the existing instance without starting another engine, including
the interval before the winning process has bound its socket.

The sibling `.lock` file is opened without following symlinks, with close-on-exec
and mode 0600. Its effective-user ownership, private permissions and regular-file
type are checked; the acquired descriptor must still match the named inode. The
lock pathname is retained across normal shutdown and process death so contenders
cannot acquire separate replacement inodes. Closing the file or process death
releases the operating-system lock.

Endpoint probing makes one nonblocking AF_UNIX connection attempt. It never
waits for a response or writes an application request. Connected peers and full
listen backlogs remain existing instances; missing/refused endpoints are distinct
states. Interrupted or in-progress probes conservatively preserve the endpoint.
Other errors are reported without deletion. A legacy listener that does not use
the instance lock is also preserved when connected or busy, regardless of reply
latency or content.

Only the exclusive instance owner may unlink a refused stale socket. Before
removal, the socket must belong to the effective user and still match the probed
device/inode; regular files, symlinks and changed paths are preserved. The IPC
server retains issue 29's independent owned-endpoint cleanup guard. Main resolves
the endpoint once and supplies the same path to ownership and server startup.
Runtime directory selection and UID-private fallback are scoped to issue 32.

## Validation

`cargo test instance::tests -- --nocapture`: eight tests pass (seven scenarios
plus the child-process fixture entrypoint). `cargo test`: 132 tests pass on the
isolated issue 29 base. `cargo build` passes locally.

- Eight concurrent child processes start behind one barrier; exactly one owns
  and binds the endpoint, seven observe the owner, and the winning inode remains
  reachable until its explicit release.
- A killed owner releases the kernel lock; its refused socket is recovered by
  the next owner without replacing the persistent lock inode.
- A listener that delays 350 ms and sends malformed content remains reachable;
  ownership detection does not wait for its reply or remove its pathname.
- A real listen backlog is filled with retained nonblocking connections; both
  probing and acquisition complete through a bounded test channel and preserve
  its inode.
- Held-lock startup without an endpoint, stale cleanup permission, regular
  files, socket/lock symlinks, private modes and invalid socket paths are covered.

All fixtures use private temporary directories and direct instance APIs. No
actual runtime socket, installed application, audio/MIDI device or desktop focus
command was touched. Existing UI focus behavior and hardware QA are unchanged
and are not claimed verified by these fixtures.
