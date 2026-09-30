# Issue #7: independent deck channel histories

Deck EQ and channel filters now keep separate left/right state. EQ gain, cut,
and solo controls update both channels; changing the engine sample rate resets
both histories and updates both EQ coefficient sets while preserving controls.

Local verification:

```sh
cargo test --offline --locked --bin omatainer -- --test-threads=1
```

All 17 tests passed (13 existing tests and four deck stereo regressions).

The regressions exercise the actual `render_deck` path on both decks at 44,100
and 48,000 Hz. Left-only and right-only impulses and logarithmic sine sweeps
remain exactly zero in the silent channel for EQ, low-pass, high-pass, and
combined EQ/filter processing. Every rendered sample matches independently
instantiated mono reference processors within `1e-6`. Independent stereo sweeps,
duplicated stereo sweeps, and equivalent mono files also match those references;
duplicated stereo and mono-file outputs are identical. Control tests cover
gain changes, cuts, gain changes while cut or soloed, solo restore, master gain
cut, UI snapshots, and 48,000 → 44,100 → 48,000 Hz sample-rate transitions.

These tests run without audio hardware and stop at the deck output. Master
delay/reverb channel timing belongs to #13, and filter-control mapping belongs
to #50. Physical-controller and output-device QA remains for the final QA run.
