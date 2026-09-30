# Issue #12: bounded source transitions for pitch lock

Discontinuous positions and source-mode changes use `DeckRt::transition_to`.
It resets the current grain to the new position and places the previous window
half a grain earlier, so its first full-weight sample is at the target instead
of 512 output frames ahead. It also clears both channel EQ/filter histories.

For a jump, the final deck output blends linearly from the last emitted stereo
frame to the new source over `ceil(output_sample_rate * 0.002)` frames: 89 frames
at 44,100 Hz and 96 at 48,000 Hz. The first frame equals the preceding output;
the final frame contains only the new source. Playback and grain timing keep
advancing throughout. Another jump restarts this bounded envelope from the
current output; old grains are never replayed.

Covered transitions include seek, cue recall, hotcue, play/pause, keylock toggle,
media replacement/unload, matching the other deck, loop edits/wraps, file-end and
negative-position corrections, touch/release, and output sample-rate changes.
Grain reads wrap inside the active loop, including the interpolation endpoint
at its final frame. Continuous scratch jogging updates the source immediately
without repeatedly starting a fade; while touched, the deck follows the hand
directly and resumes pitch-lock grains from its release position.

Local verification:

```sh
cargo test --offline --locked --bin omatainer -- --test-threads=1
cargo build --offline --locked
```

The production build and 26-test suite pass, including nine transition regressions. Distinct
signed stereo regions verify the exact envelope and absence of stale source
after it, with mixed 44,100/48,000 Hz input/output rates and a 32,000 Hz media
replacement. A narrow 128-frame marker detects accidental half-grain skips;
short-loop fixtures detect out-of-loop interpolation and EOF seam leakage.
Tests also cover matching, scratch/release, cleared filter histories, and
unchanged playhead advance. Existing deck EQ/filter reference tests remain
focused on processing after the source transition, and the pitch-lock contract
now uses the real keylock command to initialize its grains.

This is deterministic source/output validation without audio hardware or a
listening claim. Physical controller response and perceived transition quality
remain part of the final producer/DJ hardware QA.
