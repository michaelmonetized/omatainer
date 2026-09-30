# Issue 41 — MIDI realtime and channel framing

`handle_msg` now extracts status-only realtime messages before channel data
access and before MIDI learn. `FA` reaches the existing Play handler, `FC`
reaches the reserved Stop path, and `F8` reaches `MidiClockInput::receive_tick`.
The hook counts accepted ticks and records the last input source; snapshots and
status JSON expose these as `midi_clock.ticks` and `midi_clock.last_source`.
Clock reception does not change BPM, transport phase or play state. Tempo/phase
synchronization remains separate work.

The MIDI Association's [MIDI messages overview](https://midi.org/about-midi-part-3midi-messages)
specifies that realtime messages can appear between the bytes of another
message. The fixed three-byte local framer therefore emits realtime immediately
while retaining the incomplete channel frame in that supplied callback packet.
A new non-realtime status discards an incomplete frame and starts fresh. Only
actual seven-bit data bytes complete a channel frame; there is no zero velocity
or centered pitch fallback for missing bytes.

Complete Note, CC, pitch bend and poly-pressure frames retain the existing
binding/learn path. Complete two-byte Program Change and Channel Pressure are
currently unmapped and ignored, including in learn, rather than padded with an
invented third byte. Unsupported system-common/SysEx content and unsupported
realtime statuses (Continue, Active Sensing, Reset and reserved values) have no
new action; supported realtime embedded among them still reaches its handler.
The packet boundary remains explicit: truncated channel frames and orphan data
do not persist into later callbacks or invent running status. This layer is a
consumer of the complete packets supplied by the MIDI backend, not a serial
stream reassembler.

Issue 39's native callback still only copies bounded packet bytes. Parsing runs
on its worker; the new framer uses fixed storage. Its 256-byte packet limit and
queue overflow/source-reset policy remain unchanged. Overflow may discard clock
ticks and records dropped/reset counters; oversized packets are rejected before
parsing. The clock count is an accepted-command observation, not a lossless
hardware timing measurement. Stop continues using its reserved engine admission
lane, and issue 39's interleaved-stop overflow regression remains passing.

Seven new tests replace the original one-byte negative audit with positive
command and renderer assertions. They cover all insertion boundaries for every
realtime status, truncated channel families on every channel, malformed status
resynchronization, unsupported system bytes, learning, exact tick/source
publication through the snapshot worker, and the real input worker to renderer
path. A clock command processed on the renderer performs no allocations or
frees. Tests use actual queues and engine state without opening MIDI/audio
devices; physical transport/clock hardware QA remains pending.

All 174 tests and the production build passed. Validation uses the agent's
private reused Cargo target:

```sh
CARGO_TARGET_DIR=../issue-38/target cargo test realtime -- --nocapture
CARGO_TARGET_DIR=../issue-38/target cargo test
CARGO_TARGET_DIR=../issue-38/target cargo build
git diff --check
```
