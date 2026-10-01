# Issue 74: documented shortcuts for existing actions

One binding table now supplies keyboard dispatch, the visible help rows and a
checked README table. Scene digits 1..8 launch renderer scenes 0..7. W/O toggle
the existing deck A/B tempo-sync controls; F uses the current filtered crate
selection and renderer-confirmed selected deck. Brackets choose crossfader A/B
endpoints through the existing mixer gain ramp. The endpoint behavior is explicit
in both help and README, without depending on a stale GUI fader snapshot.

Existing transport, play/cue, help, MIDI-window and effect-close bindings are in
the same table. Pointer-control instructions are kept separately and no longer
misdescribe deck Q as a global quantize shortcut or A..G as keyboard piano notes.
The unsupported Tab view-navigation claim is identified as separate feature work.
No desktop configuration or system-wide binding is changed.

The existing post-widget focus guard remains authoritative, including the frame
that a text editor closes, dialogs/popups and lost window focus. Unmodified keys
require all modifier flags to be clear. Modified chords retain egui's platform
Ctrl/Command alias handling and explicitly reject an extra mac_cmd flag. This
avoids egui's matches_exact(NONE) treating mac_cmd alone as unmodified. Repeats
are ignored after egui derives them from held-key state.

Validation: all 332 local Rust tests and production build pass. Four new groups
check unique bindings and exact README/help rows, send real egui events through
the full App for all eight scenes/sync/crossfader targets, verify filtered F loads
the correct deck and source, and reject every extra modifier combination/repeats.
The previous text-focus regression now types every newly bound character into
the real search field with zero global actions. Independent source review found
no blocker. This is headless application evidence, not physical keyboard QA.
