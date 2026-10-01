# Issue 92 — typed local control schemas

Every shipped wire opcode now deserializes into a typed operation before command
admission, including read-only follow requests. Deck and scene targets have
bounded unsigned integer types and are converted to u8 only after validating
0–1 or 0–7 respectively. Each no-argument operation is an empty struct variant:
ordinary Serde unit variants ignore extra fields, so those are deliberately not
used here. Unknown fields are rejected for every operation.

The shared correlation parser retains its existing optional/null, integer and
bounded-string contract. The schema removes only that validated `id` before
parsing the operation. CLI one-based scene conversion and existing wire opcode
spelling are unchanged. The complete opcode/field/range table is in CONTRACT.md.

## Verification

31 IPC-focused Rust tests pass (one existing opt-in follow soak ignored), as does
`cargo build --offline`. The seven ordinary native CLI protocol cases and six
strict native follow groups pass against that built executable.

New actual Unix-stream/server/renderer tests cover missing, null, boolean,
string, array/object, negative, fractional, exponent, boundary overflow, 255,
256, 257 and u64::MAX targets for both deck operations and scene launches. Each
response preserves correlation, identifies the invalid field and rejects command
admission. Processing the real renderer afterward leaves transport, deck
positions/cues, selection, clip content and launched scenes unchanged. Valid
boundaries reach precisely the requested deck and every one of the eight scenes.
All no-argument operations reject extra fields, including follow before it can
switch the connection into streaming mode. Existing scene rejection tests retain
their field/range diagnostics and pass unchanged.

Peer review found no blocking issue. Fixtures use private sockets and headless
renderers, not audio devices, controllers or a live application connection.
Final assembled checks run when this issue reaches its ordered stack position.
