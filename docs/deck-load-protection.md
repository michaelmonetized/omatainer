# Deck load protection

Use **Lock** beside each deck’s time display. Each lock protects a playing,
touched, previewing or audibly fading deck in Studio as well as performance mode. Normal
mouse, keyboard, dropped-file, MIDI and IPC loads/ejects share the same guard.
A quiet paused deck remains available. Locks stay active through project/output
changes in this app session; restarting starts unlocked. Performance mode also
protects live decks whose explicit lock is off.

Select a library track and load it to the locked deck, or drop a file on that deck.
The review identifies its current track and captured replacement. **Keep playing
track** cancels without starting preparation. Acknowledge **I intend to replace
this playing track**, then choose **Confirm deck replacement**. Preparation keeps
old audio loaded until the renderer applies the ready replacement. Failure,
cancellation or changed source, lock or safety generations preserve current media.
A replacement starts paused, with paused preview cleared; play it deliberately.
Each review is consumed once and cannot target another deck or later media.

Shift-click the platter or use its **Unload** accessibility action to review eject.
The same acknowledgment confirms the captured target. Queued unload changes to
unloaded only after the renderer applies it. Cancellation preserves current audio.
MIDI and IPC cannot confirm a review. Global performance protection requires a
quiet paused deck before replacement.

`omatainer ctl deck-load A` loads the captured GUI library selection on deck A.
`omatainer ctl deck-unload B` ejects deck B. The native socket operations are
`deckLoad`/`deckUnload` with zero-based `deck` 0 or 1. Invalid targets and extra
fields are refused before command admission. These requests obey live locks and
cannot supply a reviewed override; use the explicit app decision for one.
