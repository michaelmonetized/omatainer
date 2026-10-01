# Issue #84: deck time modes and runout warnings

Each deck has a time menu in the existing footer below its platter. It chooses
Elapsed (source playhead time) or Remaining (estimated wall time to file end),
and a warning lead from 0 to 300 seconds. The default is Remaining and 30 seconds;
0 disables the warning. The settings are independent per deck and represented by
a strict, optional `DeckTimeSettings` field in the native project view. Save/Open
round-trip both decks independently; New restores defaults. Earlier native
documents without this field use those defaults. Out-of-range warning leads or
unknown setting fields fail validation and preserve the previous good file.

Remaining time uses the captured source frame count, playhead, source sample
rate, and actual rate after sync/smoothing/scratch processing. The rate is source
seconds per output second, so differing media/output sample rates are not applied
twice. Elapsed is source time rather than accumulated listening time. Tooltips
explain this distinction and that the remaining value assumes the current rate.
The display floors to tenths, carries minutes/hours correctly, and presents an
unknown value for absent, invalid or nonforward estimates.

A playing, forward-moving deck enters RUNOUT at or below the configured lead.
Both a text label and a red platter outline/readout identify it; no flashing or
color-only signal is required. Paused, unloaded, scratching, stationary and
reverse states never alert. A finite loop longer than one source frame suppresses
runout only when its full range is inside the media and the captured playhead is
inside that range. A playing valid loop shows LOOP. Its remaining readout still
estimates file end, and its tooltip explicitly says future repeats are ignored.
Invalid or out-of-range loop flags cannot suppress a real warning. Replacement
and unload continue to use the renderer's existing loop-reset policy.

Validation is local and synthetic; no hardware qualification is claimed:

- Fixed playhead cases cover short clips, long tracks, fractional boundaries,
  hour formatting, exact warning thresholds, rate changes, disabled alerts,
  pause/scratch/reverse/stationary states, invalid media and invalid loops.
- Actual renderer → bounded snapshot → GUI cases cover 44.1 kHz media at 48 kHz
  output, 48 kHz media at 96 kHz output, and 96 kHz media at 44.1 kHz output. Sync
  intentionally disagrees with the pitch fader, and several block sizes observe
  smoothing. Further checks render a 125-second track at all pitch ranges, pause,
  loop across the endpoint, scratch backward, replace, unload and reach EOF.
- Real egui pointer events operate both deck menus and edit the numeric lead.
  They verify independent settings and that no play/cue/seek command is emitted.
  Paint output verifies warning text and red outline, disabled warnings, LOOP and
  PAUSED states. The actual full App at 1440×900 verifies both time controls and
  every popup field remain on screen; Escape does not leak into performance
  commands. The settings popup stays open during field interaction.
- Settings serialization preserves each deck's mode/lead and supports defaults.
  The readout defensively bounds an excessive lead; native project validation
  rejects unsupported values and fields. Actual Save/New/Open round-trips both
  deck settings, and a failed save preserves the good destination. Existing
  periodic snapshot allocation checks cover the added scalar capture fields.

Original isolated layer: eight regression groups passed; its full suite passed 401
checks with three existing opt-in benchmark/native-soak tests ignored. The
production `cargo build --offline`, targeted rustfmt checks and `git diff --check`
also passed. The ignored tests are the unrelated dense-polyphony timing probe,
130-second native follow soak and offscreen Quickshell scene fixture.

On the final #83 stack, all **538 ordinary Rust tests passed**, with six explicit
opt-in entries ignored, and the production build passed. This includes native
project preference round-tripping and the selected-deck regression checks.
