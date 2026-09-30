# Issue #10: steady-state FX allocation contract

The earlier arp layer removed the sample-loop slot-vector clone; the independent
FX-state layer keeps each processor beside its controls and borrows slots in
place. This layer adds durable allocation assertions for the full FX matrix and
the original eight-track defect fixture.

`cargo test --locked fx_allocation_tests` checks:

- Empty, all-enabled, all-disabled and alternating-enabled chains containing
  every implemented effect: zero allocations, frees or requested bytes over
  96,000 stereo frames each, after construction/warmup.
- Actual `RtEngine::process` over 1,024 stereo frames with eight stopped tracks:
  zero heap traffic for each of those configurations and for the original
  one-compressor-per-track fixture (originally 8,192 slot-vector allocations).
- Each of the 13 effects while bypassed: exact dry output, zero heap traffic,
  and sample-exact resumed output compared with an untouched frozen processor.
  This catches DSP state advancing silently during bypass.
- Queued slot addition, parameter/mix edits and toggles: no mutation before
  the render block consumes the commands, correct target and FIFO behavior,
  and zero heap traffic when applying edits/toggles to constructed processors.

The #9 primitive-based numerical references remain part of the full suite;
this regression layer changes no DSP equations. Slot addition can allocate
processor/vector storage at command application, outside the warm sample-loop
measurement. Parameter edits and toggles retain state; bypass freezes it, and
deletion/replacement discards only the removed processor's history. With #8,
GUI/IPC producers enqueue commands and only the callback mutates the chain.
No producer concurrently mutates or frees the callback's FX state.

These assertions are intentionally scoped to steady-state processing and to
edits of already constructed slots. They do not claim that constructing a new
processor, retiring an old one, rebuilding a snapshot, or resizing callback
buffers is real-time allocation-free. The short process fixture avoids periodic
snapshot publication, which remains tracked by #25/#59. Hardware xrun and
latency qualification remains separate.
