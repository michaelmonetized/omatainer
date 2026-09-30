# Issue #5: bounded command dequeue

Each `RtEngine::process` call receives at most 32 commands, including commands
subsequently coalesced. Concurrent producers cannot extend this loop. A fixed
stack array stores the batch; any remaining commands stay in the existing FIFO.

Only adjacent absolute assignments to the same parameter and target coalesce.
Transport, note gates, pad gates, scratch deltas, target selection and relative
controls retain every event in order. Coalescing never crosses one of these
events or a callback boundary. Receipt of a note-off in the following batch
therefore cannot overtake its note-on.

The shared snapshot and `ctl status` response expose `commands`: cumulative
received/applied/coalesced counts, last-block counts, observed backlog and high
water depth, and budget-exhaustion count. Backlog is an observation at dequeue
time; concurrent producers may change it immediately afterward.

Validation:

- `cargo test --locked`: existing audio/control contracts plus parameter/event
  ordering, a completely full queue drained across eight callbacks, a note-off
  deferred across a callback boundary, and a sustained concurrent producer.
- `cargo test --locked control_tests -- --nocapture`: prints the maximum
  observed duration of the 128-frame renderer stress blocks. Assertions check
  the dequeue bound, rendered frame progression, finite output, advancing deck
  position, accounting for every accepted event, and eventual release.

This bounds dequeue/application count, not the worst-case running time of an
individual command handler. Existing instrument construction, clip edits,
snapshot publication and DSP still need their own real-time safety work. This
change does not claim device latency or xrun qualification. GUI queue-full
admission and MIDI callback blocking are separately tracked by #11 and #39;
accepted queued musical events are never discarded by this consumer.
