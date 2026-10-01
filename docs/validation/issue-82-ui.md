# Issue 82 — native project GUI and worker workflow

This layer connects the engine project handoff and native file codec to the
shipped egui application. It introduces Project → New, Open, Recent, Save,
Save As and Save Copy, plus unsaved decisions on replacement and window close.
The dialogs are application path-entry dialogs, not an OS file picker. Relative
paths resolve against the application working directory on the worker.

## Persistent state and visible behavior

`Document` wraps the strict engine `State`, strict `UiState`, and factory mapping
schema 1. The engine owns its supported timing, clip/note, track, mixer/effect,
sampler and deck controls/media references; the GUI adds crate filter, selected
typed source, scroll offset, key-help/MIDI/diagnostics panel visibility, and verified deck
history identities. Save validates the entire document, not just engine state.
Unknown mapping schema or invalid view fields fail before a destination changes.
The current build has fixed factory mappings, not an editable mapping document.
Future persistent UI fields must extend this wrapper deliberately.

New creates the engine's real empty session; the default demo is not reused.
Open/New start stopped. The completion message describes Space to resume remembered
session clips and deck play buttons to resume saved positions. The title distinguishes
an untitled state not yet saved to a file, a saved file, and unsaved edits. Saved
views wait for `Snapshot.project_revision >= Applied.revision` before controls
resume, so a lagging deck-selection snapshot cannot route F to the old deck.

Save and As update current path and captured clean baseline. Copy leaves both
unchanged. As/Copy use no-overwrite publication unless the user checks Replace an
existing file. Save to the established path explicitly uses atomic replacement.
Edits made while a captured version is being written remain dirty; a following
New/Open/Close must obtain a fresh unsaved decision. The Open path dialog itself
retains the authorization baseline, so edits made while entering a path cannot
be silently discarded based on an older decision.

Verified deck identities are matched using receipt pointer identity against the
actual engine capture. A replacement admitted before capture cannot inherit the
old GUI watch. File identities require the captured filesystem fingerprint;
embedded sample path strings alone are insufficient. Opening attaches those
identities to new applied receipts, while unrelated crate history stays intact.

## Ownership, concurrency and failure boundaries

One named worker has a bounded one-job queue and four-result queue. It performs
all project/recent file operations, engine capture/install waits, decoding the
native format, and DSP preparation. A prepared graph stays on the worker while
it sends a small Ready event and waits for the GUI's scalar expected revision.
The GUI checks its current view/load-intent baseline, drains prior bounded GUI
controller requests, invalidates old media generations, and then authorizes the
engine handoff. A controller request arriving after that check makes the engine
install fail with Conflict; the old graph and accepted request are preserved.
Large discarded/prepared/retired graph ownership stays off the GUI/audio threads.
Closing the app sets cancellation and drops channels without joining file I/O.

The underlying codec distinguishes precommit errors from a committed file whose
parent-directory sync failed. The latter remains saved and reports a durability
warning. All primary errors are visible. Auxiliary Recent failures are separate
warnings and cannot turn a committed project into a failed save. Recent history
stores at most 12 absolute paths, reads only bounded regular files with no-follow
and nonblocking open, and writes an exclusive temporary followed by atomic rename.
A FIFO, symlink or unrelated temporary occupant is never repaired or removed.
If an existing cache is malformed, newer or unreadable, its bytes are preserved:
that worker retains the read warning and uses only an in-memory Recent list for
the session, while primary project operations remain available.

Window Close is cancelled until the workflow authorizes it. Clean close captures
prior commands, then obtains a `CloseGuard` sealing creative admission through the
terminal GUI Close event. New edits or queued controller work reopen the unsaved
decision. Explicit Discard obtains a seal without a revision constraint. If audio
stops progressing while its receiver is still owned, the precommit handshake can
fail; clean close then offers Save/Discard/Cancel, while explicit Discard may exit
without claiming a saved state. A failed project worker likewise cannot trap an
explicit discard. Widget input runs before terminal result polling, so Cancel
observed in that frame prevents a pending close. A cancelled guard is dropped
with an atomic release request, not a GUI-side join or engine lock.

Cancellation remains cooperative: in-flight OS calls cannot be interrupted here.
A commit that already happened wins a later cancellation; the GUI honors its
Saved/Applied result. The engine/codec documents cover admission, file atomicity,
state bounds, fresh-process reload and canonical rendering in their own layers.

## Actual GUI and private worker evidence

Nineteen tests run real `App::update_frame` with egui pointer/key/window events,
the production project worker, private paths and the headless audio renderer.
They exercise:

- Actual Project menus and path entry for As/Copy/New/Open; native notes/view and
  built-in identity roundtrip; stopped state; Current/Copy path and dirty semantics.
- Save/cancel/destructive continuations paused after a real acknowledged capture;
  GUI frames continue, old files survive cancellation and newer edits stay dirty.
- Changes during Open preparation and its path dialog, including a new media-load
  intent; malformed, missing and unsupported-schema files preserve the current
  graph/view/path and paint a real error message.
- Persistent Recent across a second App and actual Recent-menu reopen; auxiliary
  cache failure does not alter primary success; real FIFO/symlink inputs terminate
  promptly and retain their occupants. Malformed/newer regular caches also remain
  byte-for-byte intact after a successful project save and in-memory Recent update.
- Worker result disconnection releases the busy UI, cancels uncommitted work and
  keeps the current session; explicit discard can still close.
- An invalid 4097-byte view filter fails before replacing an existing good file.
  An admitted replacement captured after the GUI watch list was taken cannot
  acquire the old deck's history identity.
- Actual viewport Close, Save/Discard/Cancel, commands accepted before capture,
  and an edit admitted between capture and seal. Clean close never drops them;
  current-frame Cancel leaves no held close guard.
- A renderer with its command receiver still owned but no callbacks, using a
  per-instance test-only 25 ms wait limit in the real engine exchange. Failed clean
  close does not exit; direct Discard and Discard after a timed-out capture do.
- Actual old/new published snapshots replayed to control display lag: F is blocked
  while the applied revision is missing and targets the restored deck afterward.
- An accepted controller Load deliberately inserted after GUI Ready checks and
  before renderer install: install conflicts, old creative state survives, and the
  originally captured load is subsequently delivered.

All tests use local Linux private fixtures. This proves the production egui/worker
control flow and headless renderer integration, not compositor dialogs, audio
hardware behavior, or the user's future physical-controller QA. The final local
full suite on the assembled issue-81 base passes **488 tests**, with five explicitly ignored
harness/benchmark entries (subprocess probes are exercised by parent tests).
`cargo build`, new-module `rustfmt --check`, and `git diff --check` pass. Two peer
reviews covered worker/path identity, close/cancellation and snapshot gating; their
findings were fixed and regression-tested before this final run.

## Later stack integration

Issue 84 adds its independent deck-time settings to the view wrapper after this
project PR. Issue 85 adds global DJ-library persistence: its close flush must either
accept the already-held project seal or admit its read-only LibraryFence through
that seal, and a combined close regression must verify it cannot wait forever.
Global library persistence remains separate from project Discard authorization.
