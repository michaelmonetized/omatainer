# Issue 30: bounded local control connections

Requests use fixed 4096-byte line storage and a fixed-size buffered reader. The
limit excludes the final newline. Each line has a 500 ms idle timeout and a
2-second total deadline, so sending a slow stream of bytes cannot retain a
worker indefinitely. The server handles at most eight clients, rejects excess
connections before parsing or submitting their commands, and closes each
connection after 32 requests. Finished workers are reaped; shutdown interrupts
all retained sockets and joins their workers before endpoint cleanup.

Writes have a 200 ms total budget, including partial writes. Responses are
capped at 8192 bytes before the newline. Diagnostics contain bounded text rather
than request bodies. Status responses select at most eight MIDI device names
(64 UTF-8 bytes each) and two deck titles (256 bytes each), setting
`state_truncated` when needed. This avoids cloning an arbitrarily large
snapshot. Snapshot acquisition waits at most 50 ms: a command already admitted
to the queue receives a truthful receipt without state; a query is rejected as
temporarily unavailable. The CLI also caps requests/responses and bounds reads
and writes, including malformed peers without a newline.

Ten new tests use socket pairs or private mode-0700 temporary directories only:

- Exactly 4096 request bytes succeed; 4097 bytes reject without mutation, free
  the worker, and permit a subsequent valid exchange.
- Idle and periodically dripping peers hit their respective deadlines.
- Eight retained clients plus forty concurrent excess clients never exceed
  eight handlers; all excess commands reject, and valid service resumes.
- Dropping the server interrupts eight partial requests with 30-second idle
  settings in under two seconds and joins every worker before path cleanup.
- Thirty-two requests close the connection and release its slot.
- A full socket output buffer forces a finite write failure.
- Handler allocation stays below 32 KiB for both 4097-byte and 1 MiB supplied
  prefixes: 5360 allocated bytes in 18 allocations for each, measured on the
  handler thread with the existing test allocator.
- Snapshot-lock contention preserves an accepted command receipt and reports a
  failed query; queries resume after the lock is released.
- More than 512 KiB of snapshot metadata, maximally escaped request IDs/text,
  and maximum command counters still produce a bounded response without a full
  snapshot clone (26,696 allocated bytes in 74 allocations, below 64 KiB).
- The CLI rejects an oversized unterminated response.

Local validation on the issue-29 prerequisite: `cargo test` passed all 134 tests;
`cargo build` passed. The 20 IPC-related tests include existing protocol IDs,
explicit rejection/receipt behavior, startup failure, and endpoint ownership
coverage. No test opens the user's runtime socket, GUI, or audio device.

These are ordinary OS-thread and socket deadlines, not hard real-time claims.
The already-existing producer admission mutex remains shared with other command
producers; its bookkeeping does not acquire an audio-renderer lock. Cooperative
instance ownership and runtime directory policy remain issues 31 and 32.
