# Issue #11: bounded control admission and reserved releases

GUI, IPC and MIDI share `CommandPort`. Each submission reports `Accepted`,
`Coalesced`, or an explicit error (`Full`, `StopPending`, `Disconnected`). The
snapshot and IPC expose separate producer accepted/coalesced/rejected counts
and the last failure. These differ from the existing consumer dequeue/apply
counts. GUI submission errors and background producer errors appear in the
actionable, dismissible error window introduced with issue #8.

## One FIFO and reserved capacity

The production channel still contains at most 256 commands; the callback still
receives at most 32 per block. No priority queue can overtake an accepted onset.

- Every first accepted MIDI note-on, sampler-pad down, or deck-touch down reserves
  a future FIFO entry for the matching release. A same-key retrigger keeps that
  reservation. A release converts its reserved credit into an actual FIFO entry.
- Gate keys currently distinguish MIDI channel/note, sampler pad, and deck. A
  fixed array of 256 active-key entries bounds storage and lookup work without
  requiring a bitmap of all possible future device/source identifiers.
- A redundant release coalesces only when its key has no outstanding accepted
  onset. An accepted off→on→off sequence always enqueues both releases.
- Ordinary admission always withholds nine additional entries: one global Stop
  and eight StopTrack lanes. Before admitting an ordinary command, the sum of
  current queue depth, outstanding gate-release reservations, these nine entries,
  and the command's own cost must fit. A new gate costs two entries; other ordinary
  commands cost one. This is deliberately conservative while Stops are queued.
- Each Stop lane has at most one pending event. Repeated Stops can coalesce only
  while commands capable of restarting that target are rejected as `StopPending`.
  This includes scene launches, AddScene, FireClip, and TogglePlay's automatic
  scene launch. A fixed atomic callback acknowledgment reopens start admission.

As long as the callback continues running, an accepted release has at most 255
FIFO predecessors and is consumed within eight subsequent command batches.
New traffic cannot overtake it. This bounds delivery in blocks, not device wall
time or the running time of unrelated existing command handlers. Disconnection
is an explicit failure, including for otherwise redundant releases.

## Thread ownership

The producer-only mutex covers fixed reservation bookkeeping and `try_send`.
No producer waits for queue capacity or constructs instruments/media under it.
Rejected payloads are dropped after the mutex is released. The callback never
acquires the producer mutex: Stop completion is a fixed atomic store, and
submission statistics use atomic loads. There is no new renderer lock, extra
priority queue, background spill queue, or emergency reset of unrelated decks.

Multiple producers can briefly contend for the admission mutex. MIDI callback
lock avoidance remains issue #39; moving all producers through the same port is
required here so raw MIDI sends cannot consume another source's release credit.
Device-specific note ownership/source identity remains issue #16. The port does
not replace the engine's existing target/voice ownership policy.

## Validation

`cargo test --locked` covers:

- Exactly 256 queued events, with note/pad/touch onsets before their reserved
  releases and all nine Stop lanes; releases finish within eight 32-command
  batches, real voices leave held stages, and an unrelated live deck continues.
- 123 distinct accepted note-on keys before any consumer runs, followed by every
  matching release and all dedicated Stops without queue-full failure.
- Repeated release and off→on→off cases; pending Stop rejection of each restart
  path until the callback acknowledges execution.
- Four concurrent producers running 400 gate lifecycles while ordinary traffic
  saturates admission; all accepted events are accounted for and no voice stays
  held after their releases.
- The callback completes while another thread deliberately holds the producer
  mutex. The two-second harness timeout detects lock dependence only.
- IPC accepted, coalesced, full, disconnected and query responses, including
  truthful separation between acceptance and executed snapshot state.

No physical controller, hardware latency, or listening qualification is claimed.
