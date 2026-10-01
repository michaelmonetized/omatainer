# Issue 49: source-owned clip gain

Clip gain is captured at note/hit onset. Synth voices retain a separate gain
applied after their individual voice filter; drum hits retain `clip_gain` beside
the sample index and playhead. These levels enter track EQ/FX before track gain.
Live synth voices and drum hits use unity clip gain. Clip replacement and gain
edits therefore do not rescale old held/releasing voices, drum one-shots or
already-generated effect tails.

`ClipGain { track, scene, value }` accepts finite values clamped to 0..1.5, with
scene/track bounds checking and target-specific adjacent-command coalescing.
`ClipSnap.gain` exposes the value, and existing Clip serialization preserves it.
Alt-clicking an occupied clip opens an editor with zero/unity controls and text
stating that the level applies to new notes and hits. Failed admission retains
the previous editor value and reports the existing persistent submission error.

Six new groups verify:

- Numerical zero/half/unity clip contribution with unrelated live input for
  bass, keys, pad, arpeggiated synth, drums and arpeggiated drums. Across 12,000
  frames, zero matches live-only and half matches live plus half the independent
  clip contribution within 2e-6.
- Same-pitch live and clip ownership remain independent.
- Mid-note gain edits and same/different-pitch replacement with a zero-gain clip preserve older held
  voices, release tails and drum one-shots; subsequent onsets capture edits.
- Independent track gain still scales the complete track, including live input.
- Snapshot/serialization values, bounds/nonfinite handling and FIFO target
  identity under coalescing.
- Actual egui Alt-click editor, rendered policy text, zero/unity edits through
  CommandPort to the renderer and asynchronous snapshot, and saturated-queue
  rejection without an optimistic gain change.

Scope: current clip playback supports MIDI synth/arp/drum sources. Audio-clip
playback is unimplemented separately and is not introduced here. Drum velocity
remains issue 52, and the per-frame sample-bank Arc clones remain issue 51.
These are local headless UI and numerical renderer checks, not physical QA.

Peer review found and reproduced the same-pitch release reuse edge before the
fix: a replacement onset overwrote the releasing voice's gain. Clip same-gate
reuse now requires a held envelope; released voices retain their own level when
a free voice is available. Physical input retrigger identity is unchanged. A
full finite voice pool still uses the existing quietest-voice stealing policy.

Final validation after the same-pitch fix: **215 tests passed**, `cargo build`
passed, new modules pass `rustfmt --check`, and `git diff --check` passes.
