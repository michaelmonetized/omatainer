# Issue 98: named, colored, persistent hot cues

Each of eight hot cues retains its source-frame position, a validated name of up
to 64 UTF-8 bytes, and an optional opaque RGB color. Empty names use the slot
number; empty colors follow the theme. The deck pads, visible waveform markers
and cue list use the same renderer-published values. Text fields are drafts;
Apply creates one undoable edit after validating the current media receipt and
set cue slot. The editor closes when that media is replaced. Queued Set, Jump,
Delete and style edits cannot apply to replacement media. All owned receipt
rejections retire on the existing worker rather than freeing on the callback.

The cue list scrolls in small windows and provides an explicit Close action.
Keyboard text editing retains the global-shortcut focus guard. Full names,
positions in source seconds and RGB values remain in accessible descriptions
when compact pad/waveform labels are shortened.

Library schema 3 stores styles with existing preparation and defaults older
schema 1/2 files to unnamed theme-colored cues. Native projects include the same
metadata. The list distinguishes session-only preparation, saved library cues,
pending saves and content not yet qualified for a future move.

Relocation changes the catalog association, never filesystem placement. The
user moves/copies the file and enters its absolute new path. Worker SHA-256
verification requires exact bytes, regular no-follow files, stable file identity
before/after reading, and an 8 GiB per-file verification bound. A prior digest
allows a missing original; otherwise the original must remain available. Names
alone never transfer preparation. A successful transaction preserves TrackId,
records the old path/fingerprint, and lets still-loaded/history-pinned old
receipts update the same verified track. Different content later occupying an
old path receives a distinct identity. The last-played navigation anchor follows
only the same verified bytes at the current location, including after filtering;
replacement bytes cannot inherit it. Conflicts, changed bytes and unavailable
unverified originals leave the old association intact.

The selected row follows that stable identity if it is still selected when the
new crate publishes. A user's intervening selection is preserved. The result
of a relocation is separately identity-qualified and retained across stale
worker results and later essential saves, so cancellation/rejection cannot turn
into a generic successful-save message. A stale request to an already-associated
destination still reports its explicit verification rejection.

Performance protection refuses new relocations, cooperatively cancels queued
verification and guards its catalog commit. Content hashes are optional work;
cue edits, preparation/history persistence and final close flush remain
essential. The worker defers automatic qualification for captured tracks and
retries after returning to Studio. This optional backlog retains at most 256
latest source identities and never starts a whole imported-library hash sweep.
Unqualified tracks retain all cue data; keep the original file available until
qualification or explicitly verify it during relocation. A committed catalog
remains truthful while protected row publication follows the existing deferred
worker policy. Every coalesced optional component rechecks its own cancellation
generation under the shared commit guard. Protected cue style edits and new cue points remain available;
cue deletion is rejected centrally at producer and renderer boundaries.

## Local evidence

Real App/egui/AccessKit tests drive all eight Set, name, color and Apply controls,
then verify exact renderer/preparation values, waveform and pad descriptions,
durable restart and restored fields. Separate cases cover malformed drafts,
queued stale Apply/Set/Delete, small-window last-row focus and shortcut isolation.
Real WAV decode and catalog workers exercise source-file move/reload, stale
publication and reordering, manual selection preservation, cancellation during
verification, and retained essential captures. Protected cue edits save before
qualification and qualify automatically after Studio resumes.

The private Linux AT-SPI fixture additionally opens the actual list through the
production Actions menu, sets slots 1/8, checks native text semantics and focus,
applies, deletes only slot 1 and closes the list. Arbitrary name/color typing is
verified by actual egui keyboard events; it is not claimed as native AT-SPI text
injection. All fixtures use temporary files/private buses. No installed desktop,
physical controller or audio-device qualification is claimed.

Normalized on the final issue-96 baseline (`acceca5`), the complete local ordinary
suite passed: **725 tests, 12 existing opt-in tests ignored**, with
`cargo test --locked -- --test-threads=1` (132.87 seconds). The five real cue GUI
groups also passed independently before the complete run. The private AT-SPI
run visited 237 native nodes and executed 100 actions across 879 actual App frames,
including the added cue workflow and existing project/help/performance flows.
`cargo build --locked`, the source/component license inventory check, and exact
embedded manifest/notices comparison against the built executable also passed.
Independent read-only peer review found no remaining blocker after the explicit
relocation-result fix.
The controlled release timing gate is run separately on the final assembled
stack; these correctness runs make no new audio deadline claim.
