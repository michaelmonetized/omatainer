# Issue #59 — share immutable waveform metadata

`Sample.peaks` now owns an `Arc<Vec<[f32; 3]>>`. File analysis and generated
media move their completed peak vector into that Arc once, before callback
ownership. Snapshot materialization shares that exact Arc; it never copies the
peak vector. The waveform widget continues to borrow the snapshot's data while
painting, with no new waveform ownership in the drawing loop.

The two reusable snapshot frames and initial publication use one shared empty
waveform. Unchanged loaded and unloaded snapshots therefore retain their
identity. Replacing media selects the new media's waveform; unloading selects
the stable empty waveform. Reloading the same immutable media object reuses its
original peaks, while newly decoded media owns a new identity even when its
peak values happen to be equal. Changes missed while publication is backlogged
still follow #25's latest available snapshot policy; this is not an event log.

The #25 ownership protocol is preserved. Audio temporarily shares `Sample`
references only after capture capacity checks pass. The worker takes/releases
those references and clones one waveform Arc per populated published deck.
A UI snapshot can retain old waveform data after the old audio buffer retires;
releasing that UI snapshot releases only its shared ownership. Failed or
backlogged frame submissions retain their payload on the publisher as before.

Allocation budget:

- Prepared callback capture/publication: zero allocations and zero frees;
  waveform length does not affect capture work.
- Off-callback materialization: zero peak-data allocations/copies. Existing
  variable metadata is cloned into the public snapshot on the snapshot worker;
  its cost scales with track/clip/deck names, racks and banks, not audio duration.
- The measured default-session fixture with one `same title` deck allocates
  29 metadata buffers totaling 3002 bytes per materialization, equally for 1,
  2048 and 1,048,576 peak buckets. This is a fixture budget, not a universal
  cap for arbitrary project metadata. Metadata growth and retirement remain
  worker responsibilities under #25.
- Media preparation performs the existing analysis-vector allocation plus one
  Arc header allocation; wrapping moves the Vec without copying its buffer.

Validation (local private Cargo target):

- `cargo test --offline`: **281 passed**.
- `cargo build --offline`: passed.
- `git diff --check`: passed.
- Actual periodic rendering/publication crosses twelve boundaries per state:
  empty, first playing media, different same-valued media, unload and reload.
  Pointer identity stays exact, and each callback measures zero allocations/frees.
- A held snapshot survives real DeckUnload, retains every peak, and releases
  that metadata when dropped without retaining the old sample's audio buffer.
- Worker instrumentation verifies allocation count/bytes are identical for
  tiny/normal/12 MiB waveforms and exactly one published waveform reference is
  added then released. Existing held-reader/full/disconnected ownership tests pass.
- The actual egui waveform widget paints identical line geometry/colors for
  shared metadata and the previous copied-vector representation at four positions.
  Existing decoder, playback and load-status regressions pass unchanged.

No physical audio/MIDI device qualification is claimed. This change neither
alters crate viewport caching nor the per-deck load receipt/status UI.
