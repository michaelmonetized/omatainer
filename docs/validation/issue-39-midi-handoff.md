# Issue 39: bounded MIDI callback handoff

Each MIDI input callback now submits fixed events to a private 256-entry SPSC
ring. It copies at most 256 bytes and performs bounded atomic operations; it
does not parse the controller map, lock command admission/log/learn/shift,
format strings, allocate, sleep, notify a worker or wait for capacity. A worker
per connected input performs the existing `handle_msg` dispatch. Idle workers
poll at 1 ms; thread scheduling is not a hard real-time deadline guarantee.

Adjacent absolute CC updates to the same mapped parameter may replace the
latest pending value when the ring is full. Relative/jog controls, notes,
transport and different targets remain ordered barriers. Every event has a
64-bit sequence and overflow epoch. The worker merges the FIFO and pending CC
by sequence, retaining whichever head is newer for the next step. It takes the
CC mailbox before probing the FIFO, so a producer racing an empty FIFO probe
cannot make a new CC overtake newly queued notes or jog events.

The pending CC mailbox contains three preallocated fixed cells. Unique writer
and reader handles own one cell each; an atomic index owns the third. A swap or
single CAS transfers ownership before accessing a cell. There is no retry loop
or fallback lock. The small `UnsafeCell` implementation is confined to
`midi/handoff/latest.rs`, with the ownership invariant documented at its `Sync`
implementation. Epoch, sequence and the complete payload travel together.

When ordered traffic cannot fit, the callback increments its input's epoch and
records the overload. The worker discards stale queued events before accepting
the newer epoch, clears that input's shift state and releases only that input's
accepted live-note and scratch-touch gates through issue 11's reserved releases.
Release is still deliverable when ordinary engine capacity is full. Other
controllers' equal-pitch notes, held touches and shifts remain active; GUI
touches are an independent owner too. Each deck's fixed touch-owner table has
the same 256-entry bound as total admission gates, so every admitted touch fits.

An overflowed mapped Stop or realtime Stop has a separate atomic safety latch
and reaches reserved transport-stop admission. Oversized callback messages are
rejected without scanning/copying their entire payload; their conservative
policy resets that source and requests a transport stop, since a stop anywhere
inside the discarded message cannot be excluded. Disconnect/shutdown retires
the source's gates, discards pending onsets and preserves queued stops.

Input received/queued/handled, CC replacements, discarded events, source resets,
oversized messages and disconnection counters are atomically observable and
displayed in the MIDI window. This is distinct from engine command admission:
its existing accepted/rejected diagnostics still apply after worker dispatch.
The callback never writes a log message to report saturation.

## Validation

- `cargo test --offline`: 179 tests pass on issue 35 plus issue 38 prerequisites.
- `cargo build --offline`: passes locally.
- Nine handoff tests cover actual callback code, command admission and renderer
  state. A native worker is deliberately blocked by held log/learn/admission
  locks; 20,000 callback invocations fill the input ring and still finish while
  all locks remain held. The measured local run took 0.765 ms total, with zero
  allocations and frees. The assertion uses a broad 500 ms completion bound,
  not a claim of per-event hardware latency.
- Saturation with an already-full engine queue retires one source's notes,
  scratch touch and shift while preserving another input's identical notes and
  touch, plus a GUI touch. Stale onsets never replay; a newer epoch resumes.
- Mapped/realtime/interleaved stop overflow, absolute-CC replacement/barriers,
  relative jog ordering, oversized/disconnected messages and shutdown safety
  stops are covered. More than 200 simultaneously admitted touches on one deck
  retain all owners and release without renderer allocations.
- A controlled interleaving pauses after an empty FIFO probe, then publishes
  notes/jog/CC from the callback; subsequent dispatch preserves their order.
  A concurrent 100,000-update mailbox stress check verifies coherent full
  payload/epoch/sequence and strictly increasing received sequence numbers.
- `git diff --check`: passes.

The MIDI parser's existing message-length behavior is unchanged here; issue 41
owns complete one-byte/interleaved realtime parsing. Issue 42 owns relative jog
encoding. No physical controller, installed application or audio/MIDI device
was used in this validation; the user's controller QA remains outstanding.
