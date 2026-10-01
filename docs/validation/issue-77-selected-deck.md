# Issue 77: selected deck controls crate loading

The crate now exposes Deck A/B load-target selectors, and a renderer-confirmed
outline identifies the active deck. Pressing a deck's panel or waveform follows
the same selection policy. Double-click and F capture the accepted target;
explicit A/B load buttons keep their direct destinations. Keyboard transport
shortcuts retain their existing deck-specific behavior.

An accepted GUI selection carries a scalar request revision through the existing
bounded command queue. The renderer publishes that revision alongside selected_deck
in the same snapshot frame. Pending load targeting survives stale snapshots and
rapid A→B→A requests; a rejected selection cannot change it. After acknowledgment,
later renderer selections take precedence normally. The command and capture add no
heap ownership or callback allocation.

Pointer selection uses actual press positions, clipping and the top interactive
layer. Per-frame event indices preserve order even though deck A widgets are drawn
first. A quick press/release in one frame, release over another deck, hover or an
occluding foreground dialog cannot silently retarget the load.

Eight real egui/CommandPort/renderer groups verify A/B panel selection followed by
crate double-click before a snapshot acknowledgment, explicit selectors/F/direct
loads, preserved unrelated deck audio, queued versus confirmed UI state, waveform
and EQ actions, bounded-queue rejection, same-frame pointer ordering and foreground
occlusion. The revision fixture measures zero allocations/frees while applying the
selection command. Test fixtures use headless devices and built-in samples; they
make no physical-hardware or installed-desktop claim.

Local preparation results: all 340 tests and production build passed. All eight
new selection groups also passed separately; `git diff --check` passed. Peer
review identified the covered-last-press ordering case, now covered by a real
partial-overlay regression. Physical controller QA remains separate.
