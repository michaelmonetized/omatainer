# Issue 34: built-in crate source routing

The crate now stores `LibSource::Builtin(BuiltinStem)` separately from
`LibSource::File(PathBuf)`. Drums and Harmony map explicitly to the two generated
session stems. No path-component or substring test decides which loader runs.
Both arrows and double-click still call `App::load_sel`.

The original audit's `load_sel` observation is now an assertion suite in
`src/ui/tests.rs`, compiled with the application. It uses the actual `App`,
bounded `CommandPort`, snapshot worker, decoder and `RtEngine`; it does not
replace application source or substitute an engine implementation. A test-only
constructor leaves external audio/MIDI connections unopened. Production startup
still requires and retains its real audio stream.

Coverage:

- Both built-ins, both decks, and repeated loads: exactly one `LoadBuiltin`
  command, no decoder job, correct stem buffer/title, stopped playhead and cue
  reset to zero. The other deck remains unchanged.
- Real egui pointer events on both arrow buttons and row double-clicks select
  the intended built-in/deck. A single row click only selects.
- A real PCM WAV whose filename begins `builtin:` still enters the decoder
  queue. The production decoder reads it; completion emits `DeckAudio` with the
  decoded sample and loads the requested deck.
- Full/disconnected command queues report rejection without decoder fallback.
  A disconnected decoder reports failure; an empty filtered selection emits
  nothing.

Run `cargo test ui::tests` for the five focused regressions, or `cargo test` for
the complete suite. `cargo build` checks normal production construction.
The local focused run passed 5/5 and the full run passed 150/150 tests.

The status text distinguishes queue acceptance (`queued`) from engine
application. The tests inspect renderer state after applying the exact emitted
command to prove successful loading and reset. Native window presentation,
physical audio output, and controller hardware remain part of final QA.
