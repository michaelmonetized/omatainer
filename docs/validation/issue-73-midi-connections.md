# Issue 73 — truthful MIDI connections and retry

## Behavior

Discovery, per-port connection attempts, explicit retry/rescan, and connection
teardown now belong to one management worker. The GUI submits through one
bounded request slot; repeated submissions while work is active report
`AlreadyRunning`. No retry creates another management thread. Successful ports
stay open; a port's backend identity distinguishes devices with identical names.
A changed port name is treated as a new device/map, retiring the prior entry.

The shared `Snapshot.midi` field reports discovered, connecting, connected,
failed with a reason, and disconnected. All discoveries are published before
attempting the first connection, so waiting ports remain explicitly discovered.
Failed attempts retain their diagnostic even when their input worker finishes.
Retries close and join the previous attempt on the management worker before
resetting its state. GUI, IPC and shell continue to consume the same status.

Keyboard and mouse have a permanent connected entry because those controls
remain available when hardware succeeds, fails, or is still being checked.
Ctrl+M exposes failure reasons and **Retry / rescan MIDI**. It displays checking
state during a slow backend operation and identifies an unavailable manager.

The existing raw MIDI callback still only pushes to the bounded input handoff.
Output-port probing also runs on the management worker. On GUI teardown, a
blocked OS call is not forcibly interrupted: the detached worker retains all
connection ownership and performs teardown after that call returns. There is
no automatic hotplug polling or claim of physical device validation.

## Validation

Tests inject a backend into the production manager, without opening MIDI ports:

- All discovered/connecting/success/failure transitions; two identical display
  names with separate backend IDs and input owners; visible failure preservation
  and successful retry without duplicate rows or reopening healthy connections.
- Discovery failure, connect failure, and recovery with keyboard/mouse present.
- Explicit missing-port detection releases the original source's held notes;
  reappearance reconnects with the same displayed row and fresh input identity.
- Held backend connection attempt while control submissions and renderer blocks
  continue; manager drop returns before the backend is released, then worker
  teardown closes the synthetic callback and publishes disconnect.
- Actual egui MIDI window renders the failure reason, drives its retry button,
  draws 24 frames and applies master controls while discovery is held, preserves
  the Space transport shortcut, and renders the successful connection afterward.
- Existing MIDI callback allocation/lock, profile, ownership, overflow/reset,
  shared-status publication and real IPC tests remain in the full suite.

No connected controller, sound device, or live socket was used. UI fixture
liveness bounds are regression guards, not audio-deadline or hardware timing
measurements.

Commands and results:

- `cargo test`: 305 passed, zero failures.
- `cargo build`: passed.
- `python3 scripts/check-midi-status.py`: native shared snapshot, private IPC,
  and the real offscreen `Service.qml` agree on all five MIDI states.
- Eight simultaneous retry callers admit exactly one request and coalesce the
  other seven; the completed pass leaves no extra retry backlog.

These checks used the private existing Cargo target, without changing installed
binaries, desktop configuration, or the application's live endpoint.
