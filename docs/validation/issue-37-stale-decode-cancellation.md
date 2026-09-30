# Issue 37: bounded decoding with per-deck request identities

Each new filesystem selection obtains a monotonically increasing per-deck token. A single decoder worker owns at most one active job, one replaceable pending request per deck and one completion per deck. New selections replace pending work instead of accumulating an unbounded serial queue. Round-robin dispatch prevents one deck's pending stream from starving the other.

The worker passes token validity into the real decoder's cancellation predicate at safe probe/packet/decode/analysis boundaries. It checks validity again before publishing. GUI polling discards obsolete completions, and the `DeckDecoded` command rechecks the token inside the renderer, covering results already queued before a newer selection. An unload, built-in replacement or direct GUI media replacement invalidates the same token through the shared submission path. Obsolete failures do not overwrite the status of a newer load.

GUI buttons, crate double-clicks and dropped files use the bounded loader. Typed decode diagnostics and visible warnings remain intact. The queue acknowledgment remains distinct from actual media application. A missing decoder backend reports failure; an unchanged selection does not implicitly retry a failed load.

Cancellation cannot interrupt an OS filesystem call already in progress. Worker-owned resources remain alive until that call returns and the next safe boundary is reached; UI teardown signals cancellation without waiting on storage. Pending/result count is bounded, but this does not impose a file-size or decoded-audio byte limit. No timed physical disk or controller claim is made.

Validation: `cargo test --locked` passes 178 tests and `cargo build --locked` passes on the issue-25 base plus issue-34/35/36 prerequisites. Eight new groups cover:

- Delayed old success/error versus newer selection and unload.
- One thousand superseded requests, fixed pending/result slots and independent decks.
- Cancellation during a controlled worker job and nonblocking teardown.
- A stale completion already in the real engine queue versus newer selection/unload.
- Sixteen simultaneous barrier-started producers with unique ordered identities, latest-only work and invalid-deck isolation.
- Actual App selections A then B with real WAV decoding and delayed completion control.
- A built-in replacement and actual egui shift-click platter unload while old file work is active.
- An actual dropped-file frame that invalidates a previously queued completion before rendering.

The existing loader, corrupt-media/warning and scan-history tests now wait for the actual asynchronous worker fixture. Those fixtures control decoder completion timing but use the real App, generation loader, CommandPort, renderer and WAV decoder; there is no invented slow-file measurement.
