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
- `drums` / `analog` / `keys` / `pad`: pads hold synth voices until release.
- Octave ^/v transposes held instrument notes by 12 semitones, and sample
  playback rate by `2^(oct-3)`.

## C5. Compose (shift-click cell)

- Shift-click a sequencer cell selects it as the compose target.
- If empty, it becomes a MIDI clip.
- Pads write notes into that cell even when not recording.

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
  never copies or clears it. Device bypass retains the existing freeze policy.

## Local control connection limits

- A request may contain at most 4096 bytes before its newline. Idle reads expire
  after 500 ms, and each complete line has a 2-second total read budget.
- At most eight clients are handled concurrently; excess connections receive a
  bounded `server_busy` rejection where possible. A connection closes after 32
  requests. Responses are at most 8192 bytes before their newline and have a
  200 ms total write budget.
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
