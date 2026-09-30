# Issue #8: exclusive audio callback ownership

The CPAL callback owns `RtEngine` by value inside `OutputCallback`. GUI and IPC
handles only submit to a bounded command channel or read published snapshots.
The renderer mutex and its all-zero-buffer fallback no longer exist. The unused
GUI-facing `Engine::notes` renderer accessor was removed.

`CommandPort::send` reports accepted, full, or disconnected synchronously without
waiting for audio. Acceptance means queued, not applied. GUI rejections remain
visible until dismissed; load receipts only report submission. IPC replies use
`accepted: true` / `command_status: "accepted"` for accepted commands and
`ok: false` / `command_status: "rejected"` with an error for failures. State
fields come from the latest snapshot and can lag the accepted command. Queries
have null submission fields. The CLI follower and status commands use the same
JSON status payload.

## Local verification

Validated with no audio device or desktop connection on 2026-09-30:

- `cargo test -- --nocapture`: 25 tests passed on the isolated #2 + #5 base.
- `cargo build`: production callback and all GUI/IPC callers compile.
- `python scripts/check-scene-inputs.py`: CLI, IPC and engine boundary checks
  passed in an isolated `panic=abort` process. Invalid scene messages enqueue
  nothing; valid scene acknowledgments are checked before renderer consumption.
- `git diff --check` passed.

The continuous-audio regression invokes the exact generic `OutputCallback::render`
method used by CPAL. A GUI worker holds the snapshot reader lock while submitting
a gain command. An actual Unix-stream IPC handler submits Play and waits to read
the same snapshot. During that deliberate delay, audio completes 48 blocks of
512 frames, crossing four normal snapshot publication boundaries. Every block
contains audible signal, the sample position advances exactly 512 frames each
time, and the two submitted commands are applied by the renderer. A later normal
publication catches up after the reader releases its lock.

The test releases the reader only after audio completion (or a generous
two-second failure timeout). This tests independence from a slow reader, not an
audio-device latency or underrun guarantee. Separate tests exercise f32, i16 and
u16 output conversion and accepted/full/disconnected outcomes through both the
shared producer handle and the actual IPC handler, including retry after full.

## Related issue boundaries

Snapshot publication now tries the snapshot mutex once, before building any
metadata, and skips that publication if a reader holds it. This minimal #25
prerequisite is necessary to keep the extracted callback independent of delayed
GUI/IPC readers. Successful publication still allocates and retires snapshot
metadata on the callback; #25 and #59 own the complete bounded publication,
retirement and immutable-waveform work.

Full queues are reported explicitly. Guaranteed delivery of critical release
events under overload remains #11; this change does not claim accepted commands
are already executed or rejected commands are automatically retried. Physical
controller behavior and real audio-device latency remain hardware QA work.
