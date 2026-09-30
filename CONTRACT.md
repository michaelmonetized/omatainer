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
