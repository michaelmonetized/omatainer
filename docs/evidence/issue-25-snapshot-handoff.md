# Issue #25: bounded snapshot handoff and off-callback retirement

Periodic publication no longer acquires the public snapshot mutex or constructs
a public `Snapshot` on the audio callback. A fixed pool of **two reusable frames**
crosses bounded free/ready queues to a snapshot worker. Audio tries to obtain a
free frame, captures scalar values and prepared text/FX data, and submits it
without waiting. When both frames are in use, the update is skipped.

The public `Arc<Mutex<Snapshot>>` interface remains available to GUI and IPC.
Only the worker waits for that mutex, builds strings/vectors/waveform snapshots,
replaces the public value, and destroys the old value. The constructor produces
a complete initial snapshot before callback ownership begins. Later values are
asynchronous; queue acknowledgments remain separate from renderer state.

## Ownership and capacity policy

- Track/clip/deck shape is fixed. Text and selected-rack buffers grow only on
  the worker. A capture that needs larger capacity submits growth requirements
  and skips that state; subsequent captures retry without truncating metadata.
- Callback metadata copy work is bounded by prepared capacities and the current
  selected rack size. This is a bounded handoff/ownership guarantee, not a
  universal callback deadline for arbitrarily large project metadata.
- Waveform capture retains at most two `Arc<Sample>` references per frame. It
  does not copy peaks or sample data. The worker consumes and clears every media
  reference before returning the frame to audio, so reuse cannot drop the last
  reference to old media on the callback.
- Callback string updates retain their existing capacity. Shrinking UI vectors
  and destroying replaced names/peak arrays happen on the worker. Recycled
  storage retains its high-water capacity; the pool remains two frames.
- A full send retains the frame and retries later. A disconnected worker retains
  the failed payload and disables further publication. Neither error path drops
  variable payloads on audio. Pool/publisher teardown occurs with engine teardown,
  outside rendering, rather than on an error inside `publish`.
- MIDI device names maintained by the non-audio side survive publication.
  Selected track and per-scene FX controls, pending clips, progress, meters and
  submission statistics retain their existing snapshot meanings.

Project/control mutation allocation remains separate work. UI-side waveform
cloning is now off audio; issue #59 can still reduce the frequency/volume of
metadata work without being required for reader-independent rendering.

## Local validation

`cargo test -- --nocapture`: **137 tests pass** on the stack through #21 with
#23/#24 included as prerequisites. Existing snapshot assertions now wait for
worker completion in test-only helpers; production publication never waits.
Four new regressions cover the real ownership boundaries:

- Twelve actual 512-frame `RtEngine::process` blocks are positioned to cross
  periodic publication, with free frames available. Each acquires a frame and
  measures **zero allocations, zero frees and zero requested bytes** on audio.
- A reader holds the public mutex for at least 80 ms and until explicitly
  released. Sixty-four renderer blocks complete while the reader is still held,
  exhaust exactly two frames, and skip subsequent updates with zero allocation
  or free activity. A machine-relative timeout detects dependence on that held
  lock; it is not a device timing target. One local run measured a 24.1 ms
  baseline and 26.3 ms with the reader held for 80.1 ms. Publication catches up
  to the current beat after release.
- Large metadata includes a 320 KiB track name, 128 KiB clip name, long deck/bank
  names, 262,144 waveform peak buckets, a 4 MiB source buffer and 512 selected
  scene FX slots. Capacity misses and subsequent successful publications allocate
  and free nothing on audio, preserve complete metadata, and later replace it
  with shorter values. Weak-reference checks prove the old source and public
  waveform storage retire after the worker processes queued frames, including
  when the engine has already removed its source reference.
- Forced full and disconnected queues retain large frame/media payloads with
  zero callback allocation/free activity, retry a full queue successfully, and
  preserve disconnected payload ownership until teardown.

The extracted production callback's delayed GUI/IPC test also continues to
render and catches up through the asynchronous worker. `cargo build` and
`git diff --check` pass. No physical stream underrun or hardware latency claim
is made.
