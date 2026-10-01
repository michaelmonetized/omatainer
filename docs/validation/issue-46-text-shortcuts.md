# Issue 46: keyboard ownership while editing

Global dispatch runs after the current frame's widgets. A focused egui
`TextEdit` owns all keys, including shortcuts, and keeps ownership through its
focus-ending frame. This protects Escape (egui clears focus before App runs),
Enter, Tab, and simultaneous typing/clicking. The rule uses the focused widget's
typed text-edit state, so non-text crate navigation retains its keyboard behavior.

Blocking dialogs call the shared `keyboard::block_for_dialog` guard before
rendering, including their first and dismissal frames. Previous-frame egui modal
state and open popups also block globals. Future text fields use the same generic
focus rule; future blocking dialogs must use the shared guard. No global shortcut
is an exception while editing or a blocking dialog is open.

Outside those contexts, ordinary shortcuts require no modifiers; help also uses
Shift+Slash and the MIDI window uses Ctrl+M. Each key event's own modifiers are
checked, rather than the frame's final modifier state. Repeats and unfocused app
input are ignored.

## Validation

Six real egui test groups render App frames and drive the actual search field or
dialog, then process the real command queue in the hardware-free engine fixture:

- Every bound printable character, spaces, slash/question mark and uppercase text
  appears in search without transport, deck, help or MIDI-window actions.
- All 32 modifier-bit combinations preserve search ownership; selection,
  cursor movement, deletion and paste still edit text normally.
- Escape, Enter and Tab cannot leak actions as focus ends.
- Unfocused bindings work with exact per-event modifiers; repeats are ignored.
- An admission-error dialog blocks shortcuts on both its opening and clicked
  dismissal frame, and shortcuts resume afterwards.
- A separate future-style TextEdit and actual egui Modal use the same guard.

These are synthetic egui/engine checks, not physical keyboard or desktop QA.

Local validation: `cargo test` passed all 235 tests, including these six groups;
`cargo build` and `git diff --check` passed. Peer source review found no blocker.
