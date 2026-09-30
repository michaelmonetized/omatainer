# Issue 29: explicit IPC startup and endpoint ownership

Application startup now binds the control socket and starts its accept worker synchronously, returning an error before opening the GUI if either fails. The startup error includes the socket path and a permissions/service context. An owned server guard stops and joins the accept worker when the application exits or GUI startup fails.

No startup path unconditionally removes an existing endpoint. A successful bind records its filesystem identity; failed worker startup and normal shutdown remove only the socket with that identity. Bind conflicts leave live listeners and unrelated files intact. If the pathname has been replaced, shutdown leaves the replacement alone. The single-instance probe treats successful connection as evidence of a listener without requiring a prompt status reply, and failed connections no longer authorize unlinking. Robust cooperative ownership and stale recovery are still #31; private runtime directory policy remains #32.

Four new tests use private temporary directories: live-socket and regular-file conflicts, injected permission/thread-start failure, successful correlated status exchange and teardown, replacement-inode preservation, and an unresponsive listener probe. Local `cargo test --locked`: 124 passed on the issue-28 prerequisite checkout. `cargo build --locked` passed. Independent source review found no blocker.

The tests do not bind the user runtime socket or open an audio device. Client request/thread limits remain #30; this layer owns listener startup and shutdown.
