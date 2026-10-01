# Issue 91 — strict status/follow request encoding

Issue 64 already replaced the original bare-text polling loop with a bounded,
persistent read-only `follow` subscription. This issue retains that protocol:
one `{op: "follow", id: ...}` JSON request starts a stream of status frames;
ordinary status uses `{op: "status", id: ...}`. Neither path sends bare `status`.
The two typed read operations now share the same encoder, correlation allocator,
JSON-object validation, newline framing and 4096-byte request limit. Existing
control exchanges use the same envelope encoder.

Client-side errors remain bounded state-unavailable JSON. `protocol_error`
identifies malformed JSON/UTF-8, wrong correlation, invalid state envelopes and
oversized frames. `not_running` identifies absent or refused socket connections.
`transport_error` identifies a dropped connection or other I/O failure. Valid
server status errors, including temporary snapshot unavailability, pass through
without forcing reconnection. Reconnection retains the existing backoff and
allocates a fresh request identity. It never replays musical control commands.

## Evidence

On Linux aarch64, 29 IPC-focused Rust tests pass, with the existing opt-in
long-running follow fixture ignored. `cargo build --offline` passes. The ordinary
seven-case native CLI protocol fixture also passes.

`python3 scripts/check-follow-protocol.py /path/to/omatainer` runs the actual
built CLI against private strict Unix socket servers in six groups:

- Ordinary status and persistent follow requests contain exactly the typed
  opcode and a unique string ID. Multiple returned states parse correctly.
- Snapshot-unavailable errors retain their server code and recover on the same
  connection.
- Malformed JSON, wrong IDs and oversized frames report protocol failures;
  the next connection sends a fresh valid request and receives actual state.
- EOF is a transport failure, while an absent socket is `not_running`.

All six groups pass. The fixtures use private runtime directories, bounded
socket/process deadlines and explicit process cleanup. They never open audio,
MIDI, a desktop window or the live application's socket. Peer review found no
blocking issue. Final assembled checks run when this layer reaches the stack.
