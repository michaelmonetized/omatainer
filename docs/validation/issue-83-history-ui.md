# Undo history: GUI and native project checkpoints

The Edit menu exposes Undo, Redo and History. Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y
come from the same binding table that drives dispatch, in-app help and the
README. Existing text/modal focus guards run first: text-entry Undo stays in
the field, and project decision dialogs cannot trigger a creative Undo behind
them. Auto-repeat never repeats a global Undo/Redo command.

The history window lists the renderer's applied and redo entries, the live
retained payload budget and outstanding worker retirement. Queue acceptance
is labeled queued; the list and persistent failure message reflect renderer
publication. A renderer rejection does not silently mutate the model. Dismissing
an automatically shown error does not change the saved history-panel visibility.

One primary pointer press through release receives one producer gesture ID.
Held arrow adjustments retain an ID while the same control keeps focus and the
key remains down. Focus loss ends grouping. Only numeric edits carry these IDs;
transport, physical note/touch gates and emergency stop keep their established
admission and release paths. The renderer groups only consecutive compatible
edits, so an intervening MIDI/IPC edit divides a drag into separate transactions.

History is process-local. Successful New/Open clears the old timeline and starts
a fresh epoch; it cannot undo into a prior project. The panel visibility is an
optional saved view field, allowing pre-history native documents to reopen.
An open clip-gain editor refreshes from the published model after the renderer's
revision is visible, rather than retaining an obsolete value after Undo.

## Save and replacement semantics

The engine's monotonic revision still protects Open/New/Close authorization.
Saved-state comparison uses the coherent content checkpoint captured alongside
the actual musical state: history epoch, content state and persistent changes
outside history, together with the captured UI view. Undo can therefore return
to saved content even though the admission revision has advanced. A stored
navigation change remains dirty when an unrelated mixer Undo leaves it intact.

Every later value within a grouped drag receives a newer content checkpoint.
A save paused after capture cannot mark those later values clean merely because
they share a history entry. The saved file remains the exact captured version.
Conversely, an edit followed by Undo while Open prepares still invalidates the
old replacement authorization, even if the content again compares clean.

## Local verification

The GUI fixtures run the actual App, egui key/pointer events, command admission,
renderer, project worker and private native files. They cover named menu/history
operation, all three shortcuts, search focus and unsaved-dialog isolation,
pointer drag grouping with an intervening external edit, queue rejection,
renderer note-limit rejection, save/undo/redo dirtiness, a paused mid-gesture save,
Open authorization after edit-and-undo, and history reset/panel roundtrip through
New/Open. Existing keyboard, sampler, controller, loader and project tests remain
part of the combined validation.

These are local Linux headless GUI/engine/file fixtures. Physical controller
performance and the user's final producer/composer/live-DJ runs remain separate.
