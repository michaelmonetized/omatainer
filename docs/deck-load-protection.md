# Deck load protection

Use **Lock playing deck A/B** in the safety toolbar. Each lock protects a playing,
touched or audibly fading deck in Studio as well as performance mode. Normal
mouse, keyboard, dropped-file, MIDI and IPC loads/ejects share the same guard.
A quiet paused deck remains available. Locks stay active through project/output
changes in this app session; restarting starts unlocked. Performance mode also
protects live decks whose explicit lock is off.

Select the target deck and a library track, then choose **Review load override…**.
The window identifies the captured target, current track and selected replacement.
**Keep current deck audio** cancels without starting a decoder or replacing audio.
Confirm starts preparation while the old track stays loaded and continues playing.
Decode failure, supersession or a changed source, lock choice or safety state
refuses application. A ready replacement starts stopped; press Play deliberately.
The renderer consumes each review at its media boundary; an old review cannot
replace later media, target a different deck or be reused after applying.

**Review eject…** captures the exact current source. Confirm queues the eject;
its status changes to ejected only after the renderer applies it. Cancelling
preserves current audio. Pending/loaded/refused load receipts and current deck
metadata remain distinct from queue acceptance.

`omatainer ctl deck-load A` loads the captured GUI library selection on deck A.
`omatainer ctl deck-unload B` ejects deck B. The native socket operations are
`deckLoad`/`deckUnload` with zero-based `deck` 0 or 1. Invalid targets and extra
fields are refused before command admission. These requests obey live locks and
cannot supply a reviewed override; use the explicit app decision for one.
