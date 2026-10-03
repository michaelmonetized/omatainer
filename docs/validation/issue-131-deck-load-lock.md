# Playing deck load protection: issue 131

Each platter has a Lock playing deck checkbox. Mouse, keyboard, dropped-file, MIDI, IPC and queued renderer loads/ejects respect the same lock while the deck plays or is touched. Undo/Redo cannot replace that active locked media. Admission is checked again at the renderer, so locking a deck after a load is queued still protects it.

A native replacement review captures the current deck track and selected replacement. Cancel keeps playing. Explicit acknowledgement authorizes one receipt bound to the reviewed current media key. Decoding proceeds while the old track stays loaded and playing. Failed decoding preserves the old track. A successful replacement stops the target deck, and a stale approval cannot replace another track. Global Performance mode still requires pausing and releasing the deck.

The checkbox is a runtime safety setting; a new engine starts unlocked. Snapshots and CLI status expose the renderer-confirmed lock. `omatainer ctl deck-lock A on`, `deck-lock B off` and `deck-eject A` validate arguments before connecting. Typed IPC uses `deckLoadLock` and `deckEject`. Custom MIDI maps can bind DeckLoadLock; factory load producers obey locks without relying on GUI selection timing.

## Acceptance fixtures

- `engine::deck_load_lock_tests`: queued lock races, all media producer forms, one-use consent, stale loads/ejects, invalid raw eject target, typed IPC, mapped MIDI and guarded history. Measured callback work allocates/frees nothing.
- `ui::deck_load_lock::tests`: actual native confirmation/cancel buttons; stale/global-mode rejection; deliberate eject; real arrow-button, row keyboard, dropped-file and factory MIDI entry points; asynchronous approved file success/failure while old audio continues.
- `ipc_schema_tests::deck_protection_cli_validates_target_and_action_before_connecting`: CLI shape, target and operation validation.

Final-source qualification receipts are retained with the combined PR. Physical controller LEDs/screens and real USB input qualification are separate pending work; injected MIDI bytes do not prove hardware compatibility.
