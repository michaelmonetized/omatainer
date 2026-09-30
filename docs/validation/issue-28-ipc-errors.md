# Issue 28: truthful, correlated IPC results

Malformed JSON, missing/non-string/unsupported operations, invalid deck indices and invalid request IDs return `ok:false`, `accepted:false`, `command_status:"rejected"`, `error_code` and a readable `error`. Requests cannot silently fall back to deck A. Unsupported operation diagnostics truncate the displayed operation to 64 characters.

Requests may include `id` as an integer or a string of at most 128 UTF-8 bytes. Every response echoes a valid ID, including rejections; malformed JSON and invalid IDs return null. Legacy clients omitting an ID retain ordered line-by-line replies with null IDs. Query replies have null acceptance; command replies distinguish accepted/coalesced submission from execution and may carry a snapshot predating that command.

The CLI supplies a unique process/sequence ID and validates the reply's ID and boolean success field. Server rejection, malformed/empty replies, missing/wrong-type success and mismatched IDs propagate through `main` as nonzero exits. `ctl follow` keeps its observer behavior: it emits structured error events and continues polling.

Validation:

- `cargo test --locked`: 120 passed on the issue-20 stack, including three new private-socket tests for malformed/missing/unknown operations, field validation, unchanged state/empty queue, request correlation, split requests, truncated EOF, valid control requests and rejected CLI exchanges.
- `cargo build --locked` passed.
- `python3 scripts/check-ipc-cli.py target/debug/omatainer`: actual CLI returned 0 for accepted submission and 1 for all six failure fixtures. The harness uses private temporary runtime directories and never launches the GUI/audio engine or connects to the live application socket.

Request-size, deadline and connection-concurrency limits remain separately scoped to #30. Request IDs correlate responses; they do not imply audio-thread application acknowledgment.
