# Issue 70: renderer-confirmed play history

The chosen event is the first valid source frame rendered with deck Play active
in a continuous playing episode, before gain/crossfader processing. Paused loads,
failed and cancelled requests, paused scratch audition, invalid/empty sources,
EOF without a source frame, and transition tails earn no update. Loop wraps are
one episode; an actually rendered pause followed by resumed playback earns a new
one. Muted routing and valid silent content count as source playback.

`Receipt` retains application state and an atomic timestamp. Its wall-clock anchor
is captured outside the callback; a monotonic elapsed time is read only at a play
onset. The renderer publishes the timestamp before retiring media. Independent GUI
watches read terminal state before the timestamp, so replacement, unload, status
Dismiss, and renderer disconnect cannot discard a final event. Steady frames do
not read the clock. Initial built-in decks also have watched receipts.

The GUI overlays history using typed `LibSource` plus issue 67's verified file
fingerprint (device, inode, length, modification and change times). This remains
separate from immutable crate storage, and lookup borrows the pathname during row
painting. Old file versions and renamed paths do not receive one another's
history; unverifiable decode identity earns no path-only credit. Removed entries
retain keyed in-session history, which can reattach only to the same pathname and
fingerprint. This is neither persisted history nor a move/content-hash tracker.

Validation on assembled issue 58 plus issue 67 prerequisite:

- `cargo test --offline`: all 347 tests pass. Eleven new play-history groups use
  real command admission/rendering, real WAV decoding and worker cancellation,
  exact queued-file cancellation, unavailable/full-queue built-in loads, filtered
  source attribution, first-frame/zero-frame behavior, pause/resume, loops, EOF,
  silent sources, paused scratch, version replacement, unload and disconnect.
- Actual egui status Dismiss is clicked before playback; the subsequent source
  event still reaches history. Actual crate text shows the captured source's
  timestamp after filtering changes without changing user selection.
- The existing asynchronous scan regression now actually applies and renders
  loaded media during the scan. Its newer timestamp survives publication. The
  viewport regression still formats only the changed visible history cell and
  performs no new filter pass.
- Thread-local allocation instrumentation measures zero allocations and frees in
  full renderer processing at the first onset, resume, and 100 continuous blocks
  crossing periodic publication. A disconnected steady GUI history poll likewise
  performs no allocations/frees.
- Existing load-cancellation/application race tests and all renderer, snapshot,
  ownership, viewport, and metadata suites pass unchanged.

Production `cargo build --offline` and `git diff --check` pass. No physical
controller, audio device, or producer performance QA is claimed; the user's final real-hardware mode runs remain separate.
