# Issue 65: shared MIDI device status

Input connection management writes the existing shared `Snapshot.midi` field.
The GUI no longer substitutes a separately reconstructed device list. Native
status requests and the shell therefore see the same source before the first
GUI read and after every asynchronous audio-state publication. The snapshot
worker already preserves this field under the same public mutex.

Labels distinguish connecting, connected, failed and disconnected inputs. A
device becomes connected only after its native connection opens successfully.
Backend/connection/worker startup failures remain visible; a failed input is no
longer presented as connected. Connection worker completion publishes disconnected
outside the raw callback. Identical device names keep independent entry identities.
Failure survives a later completion notification, and a late connect notification
cannot resurrect an already ended worker. The keyboard/mouse fallback remains
visible when no input connection succeeds.

Status handles retain a weak snapshot reference. Formatting and mutex acquisition
occur only in management/dispatch-worker code. Raw MIDI callbacks still copy
bounded bytes only; audio publication still uses the existing nonblocking frame
handoff. Holding the status snapshot can delay a management update, never audio.

This describes known connection lifecycle state. It does not add device discovery,
hotplug polling, MIDI output status or a claim that every physical unplug is
reported by the underlying MIDI backend. Those capabilities remain separate.

Validation:

- All 299 Rust tests and the production build pass on the preparation baseline.
- Three new groups compare GUI/shared state with real UnixStream status JSON
  before and after audio publication, exercise every lifecycle state and failure
  ordering, separate identically named inputs, and verify snapshot retirement.
- A real MIDI handoff worker reports disconnect after its callback owner closes.
  With the public snapshot held, 64 render blocks and raw input admission perform
  zero allocations/frees; management completion waits until the reader releases
  the snapshot, then its state survives later audio publication.
- `scripts/check-midi-status.py` obtains four native protocol replies from that
  actual headless engine fixture and feeds them to the real Service.qml under
  Quickshell. Its MIDI state exactly matches the GUI and native status arrays.
  Existing protocol truncation limits remain unchanged.

The shell fixture uses a private temporary directory and an offscreen process.
No live desktop service, application socket or physical controller is used.
