# Issue 83: renderer-owned undo transactions

Creative commands acquire their inverse at the point where the renderer applies
them. GUI, MIDI and IPC therefore enter the same history. The journal stores typed
values for global controls, track mixer controls, clip gain/content, deck media
and preparation, and track/scene effect slots and controls. It swaps only the
objects an entry changes; Undo does not install another whole project graph.
Current clip names, lengths, gain, notes and media references stay together in a
clip inverse. Deck media inverses retain the exact immutable PCM, preparation and
load receipt identity. A seek groups the position and changed preparation in one
entry. Recording can group all 64 track/scene cells in one take.

A renderer transaction is atomic with respect to its supported targets. Storage,
target validity and worker capacity are checked before any mutation. Every replay
validates all of its patches before applying the first one. Names include the
known deck, track, clip or FX slot; no history-label strings are built in audio.
Continuous gesture IDs group only adjacent edits to the same parameter target.
An intervening command splits a gesture, and effect targets remain distinct even
at the maximum rack size. Incoming command coalescing also requires matching
parameter targets and gesture IDs.

Transport, advancing playheads, momentary note/touch gates and MIDI clock ticks
are not journaled. Persistent navigation participates in the save checkpoint but
is not a creative Undo entry. Warp, automation and plug-in authoring are not
implemented in the current model, so this change does not claim history for those
future features. Process-local history is not serialized in the native file.
Successful New/Open clears it; canceled or conflicted replacement leaves it
intact. Save retains it. GUI gestures, shortcuts and coherent save checkpoints are
detailed in [the GUI evidence](issue-83-history-ui.md).

## Playing and held-input behavior

Mixer and effect replay preserves transport and the existing smoothing paths.
Replacing notes releases only clip-owned voices for the affected clip, rebuilds
that clip's schedule, and preserves unrelated live input owners. Media replay
pauses and transitions the affected deck through the existing discontinuity
handler; the other deck, tracks and live notes continue. A restored media receipt
retains its existing playback evidence rather than inventing a play on Undo.
Stale decoder requests cannot restore a receipt merely because history retains it.

Recording reserves an independent writable buffer before changing a note list.
The original list never aliases the live recording buffer. A held note retains
its original input identity and target. Undo during a hold finalizes its actual
elapsed duration into redo, cancels that capture's future writes, and leaves its
monitoring gate owned until physical release. An audio clip remains monitor-only.
If recording storage is unavailable, monitoring and release still work, the clip
is unchanged, and the history window reports the persistent capacity reason.

Active capture inverses cannot be evicted by later controls. Later duration
changes update the checkpoint chain through both applied and redo entries; saving
mid-hold and then undoing an unrelated mixer edit cannot incorrectly report clean
content. A stopped-device sample-rate preparation updates retained processors
before replay. If their new storage would exceed the budget, it preserves only
the applied inverse patches needed by current held captures, retires the other
history, starts a new checkpoint epoch and reports that history was trimmed.
Current clip data and later physical-release duration remain intact. This is an
explicit history-truncation policy for successful rate preparation, not a claim
that the rate change was rejected.

## Ownership and bounds

The timeline has at most 256 transactions, with at most 64 typed patches each.
Clips support 8192 notes for history/recording, metadata text is capped at 4096
bytes, and FX racks are capped at 128 slots. Oldest whole transactions are evicted
when necessary; an active capture instead rejects an edit that would require its
original inverse to disappear.

Live retained history has a 256 MiB payload budget. PCM accounting uses a
preallocated registry keyed by immutable sample identity, including both original
and replacement media reservations. Replaying a patch cannot silently change
registry membership. Repeated same-source loads share the live PCM charge. The
last retired history reservation carries the PCM retirement charge; FIFO worker
processing ensures earlier retired references have gone first. Payload capacity,
strings, peaks and sample/peak Arc headers are included. Shared peak buffers may
be conservatively charged more than once across different samples.

Fixed timeline, registry, 64 prepared note/name buffers and retirement/emergency
slots are separate from the live payload budget. The tested build reports
71,307,104 accounted fixed bytes; allocator/channel bookkeeping and other engine
storage are outside that figure. Outstanding retirement bytes are also shown
separately.
The 1024-slot retirement queue reserves room for already admitted commands, and
creative admission closes when its byte/slot threshold is reached. That byte
threshold is a backpressure threshold, not a claim that already accepted payloads
vanish immediately. Disconnection retains the bounded remainder and rejects new
creative work instead of freeing its last owned buffers in audio.

Incoming owned command data has an independent 256 MiB admission cap. A receive
batch releases its byte credit only after collection, so producer refills cannot
expand one batch to 32 times that cap. At most another 256 MiB can be queued while
the current batch is processed. These are separate from live history, worker
retirement and fixed storage; none is presented as total application memory.
Notes and shared PCM are charged conservatively per incoming reference. Reserved
note/touch releases and Stop remain admissible when creative/payload capacity is
exhausted.

The worker drops evicted notes, media and processors and replenishes recording
buffers. Coalesced gesture boxes and rejected owned commands follow the same
retirement path. Stale/invalid/canceled decode wrappers, failed application claims,
ignored learn strings and invalid note targets also retain their final payloads
until worker retirement. The renderer only performs bounded atomic/queue
operations and a nonblocking history-view publication attempt. This evidence
covers journal capture/replay and owned retirement; it does not assert that every
preexisting forward DSP-construction command is allocation-free.

## Local verification

Final private Linux validation: `cargo test` passed 529 tests with six explicit
ignored probes; `cargo build` passed. The ignored undo benchmark was run separately.

The real renderer/admission fixtures cover undo/redo while playing and holding
notes, full 64-cell takes, actual held lengths, save checkpoints across a later
mixer edit, exact sample/receipt restoration, MIDI and Unix-socket IPC edits,
continuous gesture interleaving, capacity eviction, redo-branch retirement,
same-media deduplication, note/media rejection, all owned media-request early
exits, worker saturation/disconnection, effect target collisions, maximum rack
admission, sample-rate changes and successful/canceled/conflicted project install.
Thread-local allocator measurements assert zero allocations and frees for the
covered replay, grouped-command, rejection and release paths. Last-owned sample
fixtures observe the weak reference expire on the worker after callback return.

The isolated local benchmark replays a populated 256-entry timeline 51,200 times
and verifies zero callback-thread allocations/frees. The final local run took
487.909435 ms, or 9.53 microseconds per operation; concurrent local work can affect
this timing. Its result is a renderer operation timing, not output latency, physical-controller latency or an XRUN
measurement. Run it explicitly with:

```sh
cargo test engine::undo::tests::dense_history_replay_benchmark -- --ignored --nocapture
```

Physical controller and producer/composer/live-DJ QA remains the user's final
hardware pass. The automated evidence uses actual local Rust/egui/renderer/IPC
paths and private files, without deployment or claims about unconnected devices.
