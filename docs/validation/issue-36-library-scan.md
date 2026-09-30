# Issue 36: cancellable background library scanning

Startup initializes the two built-in sources and submits filesystem discovery
to one persistent worker. The Scan button uses the same path. Traversal,
filesystem metadata reads, reconciliation of existing valid metadata, and full
crate sorting happen on that worker. The UI polls a bounded result channel and
fixed progress counters, displays explicit scanning/cancelling/complete/cancelled/failed states,
and exposes a Cancel button. Scan is disabled until the current job has actually
finished, preventing repeated clicks from accumulating workers or jobs.

The visible crate is an immutable `Arc<Vec<LibItem>>`. A worker takes a cheap
baseline reference, merges existing title/artist/BPM/key/length/history by typed
source when the file size and modification time remain unchanged, and returns a
complete sorted candidate. A bounded acknowledgment transfers replaced or
discarded crate ownership back to the worker, keeping full crate destruction
off the publication path. Last-play changes made by the UI during traversal live
in a small source-keyed overlay and cannot be overwritten by an older baseline.

At publication, selection is remapped from the currently selected typed source,
including an active text filter. If that item disappeared, selection is clamped
to a remaining visible row (or zero for an empty result). Existing UI filtering
and selection remapping still traverse the visible list; this change does not
claim constant-time UI rendering or solve crate-row virtualization. Errors and
cancellation retain the old crate, metadata, and selection. A cancellation that
arrives after traversal completed but before publication discards that result.

Cancellation is cooperative around traversal/metadata work and sorting. A
filesystem syscall already blocked in the OS cannot be interrupted portably;
the UI remains responsive and does not spawn another worker behind it. Closing
the app signals cancellation and does not synchronously join a blocked scan.
The worker owns its data and exits when the blocked operation returns. This is
not a promise of a physical disk latency bound.

Eight new tests use private temporary directories and synthetic file entries:

- Thirty-two full production `App::update_frame` egui frames render while a
  traversal hook is deliberately held. A real Q key event submits DeckPlay,
  which the real renderer applies and renders for every iteration. The old
  crate stays visible throughout; the full result appears after release.
- A completed merge preserves the selected filtered source, valid cached
  metadata, an in-flight load's new history timestamp, and sort order; repeated
  roots do not duplicate entries.
- The rendered Cancel button cancels a slow scan, overlapping requests are
  rejected, the old crate survives, and a later scan succeeds.
- Cancelling an already-ready result prevents stale publication.
- A filesystem error after partial discovery leaves the original crate and
  selection intact; a corrected scan recovers.
- Unchanged files retain cached analysis; changed size/mtime invalidates it.
- Only supported regular files enter the crate; missing optional roots are an
  empty success, and removal of a selected file remains safe.
- Dropping the scanner does not wait for a held filesystem hook.

Validation on assembled issue 25 plus the issue-34 prerequisite: all 158 local
tests passed. The 13 UI tests include the existing built-in arrow/double-click
and real-file decode routing cases. `cargo build` passed. The heartbeat fixture
uses real egui logic, command admission, and rendering without opening audio or
MIDI hardware; it proves independence from the blocked scan, not desktop FPS or
physical disk throughput. No user Music directories were scanned by tests.
