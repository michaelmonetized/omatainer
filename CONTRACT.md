# Omatainer control contract

Every labeled control must change audio or project state the way the label says.
Handlers that only flip a light, a string, or `return y` are bugs.

These clauses are enforced by `cargo test` (`engine::tests::contract_*`):

| Clause | Test |
| --- | --- |
| C1 Pitch lock | `contract_pitch_lock_preserves_pitch` |
| C2 Match | `contract_match_follows_favored_bpm` |
| C3 Banks | `contract_banks_sixteen_distinct` |
| C4 Held pads / octave | `contract_held_pads_and_octave` |
| C5 Compose | `contract_pad_writes_empty_clip` |
| C6 Spread / balance | `contract_spread_and_balance_are_stereo` |
| C6 EQ 3/5/8 | `contract_eq_bands_differ` |
| C6 Arp | `contract_arp_steps_chord` |
| C7 Builtin crate | `contract_builtin_reload` |

## C1. Pitch lock (L)

- **Off:** pitch fader changes tempo and pitch together (resample).
- **On:** pitch fader changes tempo only. Played pitch stays that of the file
  (OLA grains at 1×, playhead advances at the fader rate).
- Toggling L is audible at a non-center fader, not just a LED.

## C2. Match (⇄)

- Favored deck = xfader ≤ 0.5 → A, else B.
- Unfavored deck’s playback rate is set so its **effective BPM** equals the
  favored deck’s **pitched** BPM (`file_bpm * pitch_rate`), not session BPM.
- If both decks are playing, only the unfavored playhead is phase-aligned.
  Favored time does not jump.
- Works whenever the favored deck has audio; the unfavored deck can be stopped
  (rate is armed for when it plays).

## C3. Sample banks (Kit / Perc / Hits)

- Three banks, **16 samples each**.
- Switching banks changes which buffer pad N triggers.
- Samples in a bank are not all the same buffer.

## C4. Held pads + instrument + octave

- Pointer **down** = note/sample on, **up** = off. No click-on-then-off.
- `samples`: pads fire the current bank (one-shots may ring after release).
- `analog` / `keys` / `pad`: pads hold their distinct named synth voices until
  release. Drums are available through the sample banks; there is no drums synth
  choice until a distinct implementation exists.
- Changing the instrument affects new pad presses. Held voices keep their
  original sound, envelope and mixer destination until their matching release.
- Octave ^/v transposes held instrument notes by 12 semitones, and sample
  playback rate by `2^(oct-3)`.

## C5. Compose (shift-click cell)

- Shift-click a sequencer cell explicitly arms it as the compose target. The
  sampler also has Arm selected cell and Disarm compose controls, and displays
  the renderer-confirmed armed destination or disarmed state.
- If empty, the armed target becomes a MIDI clip. Browsing and playback/scene
  launches cannot change that destination; only another explicit arm can.
- Pads monitor and write into that destination even when not recording. Held
  pad voices and capture releases retain their original destination.
- Disarm finalizes current pad captures without cutting held monitor voices.
  Stop (including transport toggle-off) disarms composition. Restarting cannot
  re-arm it. Disarmed pads do not edit clips unless Record is deliberately on.
- Plain selection only browses; it neither creates a clip nor arms composition.

## C6. FX: spread, balance, arp, EQ 3/5/8

- **Spread:** stereo width (Haas). L and R differ when spread ≠ noon.
- **Balance:** left/right mix.
- **Arp:** on a MIDI clip, overlapping notes are stepped as 16ths instead of
  held as a chord.
- **EQ3 / EQ5 / EQ8:** different crossover counts (3 / 5 / 8 bands).

## C7. Builtin crate

- “Drums (session)” / “Harmony (session)” load that stem onto the chosen deck
  (→A / →B / double-click), including when it is already on a deck.

## Clip stop and voice tails

- Stopping or replacing a clip releases that clip's synth notes, including its
  arpeggiator. This also applies to scene/transport stop, single-shot completion,
  and replacing the note list of a playing clip.
- Synth voices finish their envelope release instead of being cut off. Finite
  drum one-shots and effect tails may finish naturally; no stopped clip may keep
  a synth voice held indefinitely.
- Live notes are separate from clip notes, including at the same pitch. Clip
  note-offs and stops preserve live notes and unaffected tracks; live note-offs
  preserve clip notes.

## Mute and solo

- Mute and exclusion by solo silence a track's output. Its note scheduling,
  synth envelopes, one-shot sample positions and FX histories keep advancing.
- A muted drum hit finishes naturally in silence; unmuting never resumes a
  frozen hit. Notes whose release occurred while muted cannot remain held.
- Unmuting restores the sound at the current musical position, including any
  still-current natural effect tail. It does not replay elapsed material.

## Quantized clip launches

- A queued clip starts on the next selected beat grid while transport runs;
  launching from stopped transport starts immediately. Every clip in one scene
  launch shares the same scheduled start.
- Before that start, the incoming clip emits no events. The sequencer shows it
  as queued with zero progress. Existing release envelopes, finite drum hits,
  effect tails, and unrelated live input retain the stop/tail policy above.
- The first event fires once, and a one-shot gets its complete clip length from
  that scheduled start. Loops and retriggers use the same sample boundary rule.
- The transport beat stored during rendering is the end of the current sample
  interval: an event exactly at that endpoint belongs to the following sample.
  A tiny beat tolerance absorbs clock accumulation error at exact grids.

## Library scanning

- Startup presents the built-in crate immediately. Filesystem discovery,
  metadata reconciliation, and sorting run on one background worker.
- Scan shows entry/file progress and an explicit completion or failure state.
  Cancel keeps the existing crate; another scan starts only after the current
  worker has acknowledged cancellation. Filesystem calls already in progress
  finish before cancellation can be observed.
- A completed crate replaces the old one atomically. Selection follows the
  same typed media source, valid cached metadata survives unchanged files, and
  play-history edits made during a scan remain visible. A scan error preserves
  the existing crate and selection rather than exposing a partial result.

## Pad monitoring and recorded playback

- Each pad press captures one destination track. The selected sampler instrument
  or sample bank supplies one source voice, which passes through that track's
  stereo EQ/FX, gain, pan, mute, and solo gate. Selection changes do not move a
  held voice or its release/one-shot tail to another track.
- Recording/composition writes the visible note immediately while the physical
  input supplies monitoring. That new note does not create a second clip voice
  in its initial pass or while the physical gate remains held, even after Record
  is switched off.
- After release, a captured note joins a later loop at its next eligible onset;
  a one-shot capture plays on a subsequent launch. Stored pitch, velocity, start,
  and duration remain unchanged. Other clip notes, including the same pitch,
  continue playing normally. Explicit note-list replacement or relaunch starts
  normal playback of the resulting clip.

## Scene FX ownership and routing

- Each scene owns its device settings and processor histories. Opening another
  scene panel changes only the edit target, never the audio route.
- A track enters its clip's scene bus when that clip actually starts. Pending
  launches keep the previous bus; stopped tracks retain their last bus for
  release envelopes, live input and track FX tails. New sessions use scene 1.
- Added scenes can play on different tracks at once. Each scene processes its
  own stereo sum, and those outputs are combined before decks and master FX.
- Starting another clip moves that track's complete output to the new scene,
  including any remaining track-level release tail. The old scene FX history
  stays on its original bus and continues receiving zero if no track remains.
  It decays according to its own feedback settings; changing panels/scenes
  never copies or clears it. Device bypass fades to dry, then freezes that slot's
  history according to the FX bypass policy below.

## Local control connection limits

- A request may contain at most 4096 bytes before its newline. Idle reads expire
  after 500 ms, and each complete line has a 2-second total read budget.
- At most eight clients are handled concurrently; excess connections receive a
  bounded `server_busy` rejection where possible. An ordinary connection closes
  after 32 requests. Responses are at most 8192 bytes before their newline and
  have a 200 ms total write budget.
- Diagnostics do not echo request bodies. Status metadata is capped, with
  `state_truncated` indicating omitted text or device entries. If a snapshot is
  unavailable after 50 ms, an accepted command still receives its receipt;
  a status query receives a temporary-unavailability rejection.
- Violations and disconnects release the client slot. Server shutdown interrupts
  and joins client workers before removing its owned endpoint.

## Media load replacement and cancellation

- Each deck's new file selection receives a unique increasing request identity.
  Unload and built-in replacement invalidate older pending, active, completed
  and already-queued file results. Obsolete errors cannot replace newer status.
- One decoder worker has at most one active job and one pending/result slot per
  deck. Pending work coalesces to the latest selection; decks dispatch fairly.
- Token validity is checked at decoder boundaries, UI publication and renderer
  application. Cancellation does not interrupt a filesystem syscall in progress;
  it takes effect at the next safe boundary without blocking UI teardown.

## Controller library loads

- Controller load buttons capture the GUI-published typed selection and target
  deck when the MIDI dispatch worker admits the request. Browsing or rescanning
  afterward does not substitute another source or move the GUI selection.
- A separate bounded queue holds 16 library requests; the GUI dispatches at most
  eight per frame through the same built-in/file loading path as crate buttons.
  Raw MIDI callbacks and the audio renderer never decode media or resolve paths.
- Empty selections, unavailable GUI receivers and full request queues report
  visible failures. An uncaptured legacy renderer request fails explicitly;
  it cannot infer a later GUI selection. File loading uses the existing pending,
  result and diagnostic status, with request-token cancellation rules above.

## Crate browsing

- Crate rows use a fixed-height virtual viewport. Only visible rows and egui's
  bounded overscan are painted or formatted; unchanged visible cells are reused.
- Filtered indices follow the scan worker's sorted crate and rebuild only when
  the query or immutable library metadata changes. Controller selection uses
  the same index cache. Play-history changes refresh only visible history cells.
- Selection follows its typed media source across filtering and publication;
  the scroll anchor follows its source across reordered scan results. If a
  selected source disappears, selection clamps to a valid neighboring row.
- Clicking a row focuses the crate. Up/Down, Page Up/Down and Home/End move and
  reveal selection while that list has focus. Search-field arrows edit text.

## Clip gain

- Supported MIDI, arpeggiated and drum clips capture clip gain independently at
  each note/hit onset, before track EQ/FX and track gain. Zero silences that
  clip source; unity keeps its original level. Live input and pads use unity
  clip gain and remain subject to their ordinary track controls.
- Gain edits apply to new notes/hits. Held voices, finite drum hits, release
  envelopes and existing downstream effect tails retain their captured level,
  including after a different clip launches. Track gain still scales the whole
  track output.
- Alt-click an occupied clip to edit its gain from zero to 1.5, with explicit
  zero/unity controls. Snapshots and serialized clips expose the same value;
  nonfinite edits are rejected. Audio-clip playback remains a separate feature.

## Deck filter sweep

- Center (`0.47` through `0.53`) is transparent. Leftward motion monotonically
  increases low-pass attenuation; rightward motion independently increases
  high-pass attenuation. Each channel has independent filter history.
- The active response meets bypass continuously, with a bounded 5 ms control
  slew for full-range jumps. Returning to center clears old branch history.

## Drum velocity

- Live MIDI and MIDI-clip drum hits capture normalized velocity independently
  from clip gain. Overlapping hits retain both values until their one-shots end.
- Velocity-zero note-on follows the release path and never creates or steals a
  drum voice. Existing finite drum hits continue under the usual release policy.
- Arpeggiated drum steps use the largest velocity among active, visible notes
  of the selected pitch. Membership boundaries, edits and loops refresh that
  value; a zero-velocity step is silent. Synth arp velocity is unchanged.

## Neutral FX and bypass

- Empty racks, noon Spread/Balance, flat EQ, and the audio side of Arp pass each
  channel through exactly. Common dry/wet processing returns exact dry samples
  at zero mix; enabled processor histories continue advancing at zero mix.
- A slot's initial enabled/disabled state takes effect immediately. Subsequent
  bypass changes crossfade processed and dry audio linearly for 5 ms at the
  output sample rate. Reversing a change continues from the current fade level.
- Processor history advances during the audible fade, freezes once fully dry,
  and resumes during the enable fade. Flat EQ keeps history warm; neutral Spread
  never activates a delay tap. Sample-rate changes rebuild history and restart
  at the slot's configured enabled/disabled state.

## Time-effect slot mix

- Delay, Reverb and Chorus produce fully wet processor output. The slot applies
  one linear dry/wet interpolation: dry × (1 − mix) + wet × mix.
- Zero mix emits dry input while processor history keeps advancing; full mix
  emits only the time-effect signal. Intermediate mix changes do not scale the
  stored feedback history or apply the wet coefficient a second time.

## Metronome timing

- Each transport beat starts a 20 ms click with a 1 ms attack and a decay to
  zero. Beats divisible by four use 1200 Hz at amplitude 0.20; other beats use
  800 Hz at amplitude 0.12, before existing master/cue processing.
- Beat scheduling uses half-open sample intervals. Oscillator phase and envelope
  belong to the click voice, so callback block sizes cannot change the sound.
- Disabling the metronome or stopping clears its voice immediately. Enabling or
  resuming between beats waits for the next boundary; starting on a boundary
  starts that beat's click. A sample-rate change clears and rebuilds the voice.

## Mixer control gains

- Crossfader and track pan/gain coefficients are cached at audio-block boundaries.
  Unchanged controls perform no per-sample powers or square roots. The original
  crossfader law remains `A=(1-x)^(1+2.5c)`, `B=x^(1+2.5c)` for position `x` and
  curve `c`; the existing square-root pan law is unchanged at steady settings.
- After the first rendered frame, crossfader, curve, track gain and pan changes
  ramp from the currently audible gain pair to the exact new pair over 5 ms
  (rounded to the nearest sample). A reversal starts from the current pair.
  These are linear ramps in gain space; steady curve endpoints remain exact.
- Muted/solo-excluded tracks still advance ramps. The stopped sample-rate reset
  establishes current target gains directly at the next callback. Initial
  controls also start at their exact target without an unnecessary fade-in.

## Spread and Balance slot mix

- Every Spread and Balance instance processes its own position in the serial
  stereo chain. Its mix applies once: dry × (1 − mix) + full effect × mix.
  Zero mix is exact stereo identity; full mix keeps the existing effect law.
- Spread above noon delays the right channel; below noon it blends toward the
  delayed stereo midpoint. Noon is exact identity and does not advance a delay
  tap. Balance keeps the existing square-root attenuation of the opposite side.
- Nonneutral enabled Spread history advances even at zero mix, so increasing
  mix exposes its current history. Per-slot bypass retains the 5 ms fade/freeze
  policy; each duplicate instance owns its history and mix independently.

## Three master FX controls

- Each of the three legacy master FX Wet controls addresses its own supported
  stereo slot. Initial types are Echo, Reverb and Filter, all at zero wet.
- The third control (slot index 2) initially blends a two-pole 1 kHz low-pass:
  minimum is dry audio, maximum is the filtered signal on both channels.
  Its existing NS7FX channel-1 CC `0x32` mapping has those same endpoints.
- Select cycles Echo → Reverb → Filter in that slot. Snapshot type and wet
  values describe the actual processor and mix, including the third slot.
- Every factory FX binding must resolve to a supported slot and observable
  renderer state; adding a controller label cannot silently create an inert FX.

## Persistent status followers

- `ctl follow` opens one read-only `follow` subscription. Its initial request
  retains the 4096-byte, 500 ms idle and 2-second total read limits. It then
  receives at most four current-state frames per second; no updates are queued
  and later bytes on that connection cannot submit commands.
- A subscription consumes one of the existing eight tracked client slots. Its
  lifetime is deliberately independent of the ordinary 32-request cap. Each
  frame retains the 8192-byte and 200 ms write bounds; a failed write retires the
  worker. Shutdown closes sockets and joins workers, including subscriptions.
- Follow frames carry the subscription ID and null `accepted`/`command_status`:
  they describe published state, never command admission or execution. Snapshot
  contention reports temporary unavailability without replacing the connection.
- The server compares only bounded display metadata and scalar telemetry. It
  reuses the encoded frame while those fields are unchanged; tracks, clips and
  waveform data are neither copied nor serialized for status. Changed metrics
  still produce a fresh bounded frame.
- The CLI has a 2-second initial request write and per-frame read budget,
  nonblocking connection attempts, and reconnect delays of 250 ms, 500 ms, 1 s,
  2 s, then at most 4 s.
  A valid frame resets backoff. Closed output exits successfully; process exit
  closes the socket. A reconnect never retries a musical control command.
## Shell scene arguments

- `Service.scene(n)` and public shell IPC `omatainer.scene` use one-based scene
  numbers 1 through 8, matching `omatainer ctl scene <n>`. The CLI alone converts
  them to the protocol's zero-based `n` and engine scene indexes 0 through 7.
- Every queued shell scene command captures its own separate CLI argument.
  Its receipt and final result retain that argument, even when several different
  scenes are waiting. Command acceptance still does not mean audio application.
- Invalid scene inputs return an explicit rejected result without queuing or
  starting a control process. Fractions, nonnumeric strings, booleans, missing
  values and out-of-range numbers never collapse to scene 1.
## Crate duration

- Files start with an explicitly unknown duration. Once a decoded file becomes
  the current deck load, the crate records its playable frame count divided by
  its own sample rate, independently of mono/stereo channel count or output rate.
- Duration belongs to the source path and matching file fingerprint. Unchanged
  rescans preserve it; replaced or changed files return to unknown until a valid
  result for those bytes becomes current. Rejected, cancelled and stale loads
  cannot update another row or supply an old file's duration.
- The existing metadata worker merges duration and publishes the crate atomically.
  Selection follows its source and visible cells refresh. Unknown displays as
  `unknown`; a measured zero is distinct and displays as `0:00`. Display rounds
  down to whole seconds while the cached value retains fractional seconds.
