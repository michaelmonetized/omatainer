# Issue 93 — applied theme reload receipts

The CLI now sends a distinct typed `reload-theme` request. The existing IPC worker
passes a ticket to a bounded GUI endpoint owned only by producers. The GUI starts
one forced resource read and acknowledges it after the following actual egui
frame installs fonts and zoom. This is an applied theme receipt, not an audio
queue acceptance or status snapshot.

The forced reader revalidates unchanged font bytes and treats colors, shell size
and selected font as one transaction. Failed bundles leave the previous usable
style and fonts in place, including after later automatic polls of the same
invalid resources. Corrected resources can recover normally. Follow-theme off,
explicit text size and scale keep the saved preference policy authoritative.

Validation uses private temporary resources, Unix endpoints and the production
App frame/renderer path; no live desktop, IPC endpoint, hardware or user files are
changed. New cases cover:

- Eight-ticket admission, unavailable/dropped GUI, FIFO identities, timeout and
  cancellation before application versus an explicitly unknown outcome afterward.
- Forced same-identity font re-read, atomic invalid colors/shell/font behavior,
  automatic recovery and retained font identities after failure. The watcher
  republishes valid cached resources once even if the forced ticket expired, so
  cancellation cannot leave the automatic theme permanently stale.
- Real IPC correlation and actual next-frame installed font bytes/text styles,
  with shell/font-only changes and an unchanged colors modification timestamp.
- No audio command admissions from reload, strict schema rejection, saved
  appearance overrides changed between installation and completion, and GUI
  controls continuing while the resolver is held.
- Private server shutdown while a resource read is held: handlers stop promptly,
  the endpoint disappears, and the late cancelled result is not applied.
- An opt-in freshly built native CLI launches against the private actual server
  and App. A held font resolution exceeds the normal 800 ms status budget, then
  completes successfully; a subsequent invalid shell size returns nonzero and
  preserves the installed style.

Resource limits: eight queued tickets, one active forced job/result, the existing
one latest automatic result and eight tracked IPC handlers; 4096-byte requests,
8192-byte replies, existing finite writes, 3-second server receipt deadline and
4-second reload-only CLI read deadline. Fontconfig has its existing 500 ms limit.
An unresponsive kernel file read may retain the single detached worker until the
OS returns; GUI shutdown and request completion do not wait for it. These tests
prove the native API and egui installation path, not physical display rendering.

Commands (private Cargo target reused):

```sh
cargo test theme -- --test-threads=1
cargo test -- --test-threads=1
cargo build
OMATAINER_TEST_BINARY=/absolute/path/to/target/debug/omatainer \
  cargo test native_cli_reload_waits -- --ignored --test-threads=1
```

Final local results: 490 tests passed, six opt-in tests ignored by the ordinary
suite; production build passed. The new native CLI opt-in test was then run
explicitly and passed. The final suite includes the added cancelled-ticket
watcher recovery regression and both real 3-second IPC timeout boundaries.

Assembled stack validation: 642 Rust tests pass (9 opt-in fixtures ignored),
a fresh production build passes, and the native reload test passes explicitly
against that executable. The six native follow groups, seven native CLI cases
and native package validation also pass.
