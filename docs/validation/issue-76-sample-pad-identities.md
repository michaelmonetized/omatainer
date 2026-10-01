# Issue 76: displayed pad numbers match dispatched identities

Sample pads now display 9–16 on the top row and 1–8 on the bottom row. Physical
identities remain top 8–15 / bottom 0–7, preserving the established piano geometry
and held-source ownership. `PadIdentity` supplies the one-based sample label,
piano label and MIDI pitch used by rendering/hover text and engine note selection.
Sample number N means zero-based bank slot N−1 in every bank.

The piano keeps bottom A/B/C/D/E/F/G/A and top A#/blank/C#/D#/blank/F#/G#/blank.
Those three gaps do not trigger notes. A press must be a new primary-pointer down
on an enabled cell and successfully enter command admission before it is held.
The matching release uses the original identity even if the current mode makes
that cell blank or the pointer is released elsewhere. Changing from a blank
piano cell to samples while its pointer is already down cannot create a gate.
Held synth voices retain issue 58's original kind and issue 16's release ownership.

Validation on integrated issue 64:

- Seven real egui pointer groups exercise all 16 sample labels in all three banks,
  asserting exact on/off commands and the actual `pad_banks[bank][index]` buffer
  selected by the renderer. Each hover paints the matching number and slot.
- Every physical cell is pressed/released in each of Analog, Keys and Pad modes.
  Tests independently specify the expected octave-three note numbers, check all
  active voice identities and matching release pairs, and verify all gap cells
  emit no gate. Half the releases deliberately occur outside the pressed cell.
- Held samples switch to instrument mode across every cell, including gaps;
  held synths switch to samples, change selected track and transpose octave,
  retaining kind/pitch ownership until the original release. Inert-gap presses
  remain inert when switched to samples. Full-queue rejection emits no orphan
  release and records no held UI gate.
- Same-frame release/press handoff, same-pad retrigger and complete quick taps
  retain exact paired identities, including a quick tap released outside the
  cell. Dragging in from elsewhere does not trigger it. Press-position hit tests
  honor the clipped interaction rectangle and top interactive layer: a real
  foreground area blocks new presses without swallowing an existing release.
- `cargo test --offline`: 357 pass, two opt-in shell/native CLI fixtures ignored
  by the default suite. Existing sampler audible-source, bank distinction,
  held-voice, octave, composition and MIDI-input ownership tests also pass.
- `cargo build --offline` and `git diff --check` pass.

Existing factory MIDI profiles have no sampler-pad action: Akai-family controls
currently route documented hotcues or ordinary live MIDI notes. This change does
not claim a new hardware sampler map or alter those identities. The sampler
command's existing zero-based `pad` value now agrees with the visible number and
tooltip. Physical controller and full producer/composer/DJ QA remain pending the
user's later hardware runs.
