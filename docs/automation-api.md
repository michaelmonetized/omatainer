# Automation API v1

Open **Setup → Automation** to inspect the actual local socket, read native state,
rename the selected track, or schedule and cancel its gain. Each mutation returns
a job; **Refresh API job** shows the renderer's applied/rejected/cancelled result.
The command line uses the same typed protocol:

```sh
omatainer ctl api '{"op":"discover"}'
omatainer ctl api '{"op":"state"}'
omatainer ctl api '{"op":"subscribe","page":{"axis":"scene","limit":8}}'
```

`discover` is the authoritative command/type/error catalogue. Unknown fields and
operations reject. State pages contain at most 16 summaries (default 8); follow
`next_offset` until it is null. Names are abbreviated to 32 UTF-8 bytes and expose
`name_truncated`. Identifiers always refer to the original native object, even
after display order changes. Deleted objects and replaced projects reject.

## Local transport and replies

The Linux desktop creates a Unix stream socket in its private runtime directory.
The socket uses mode 0600 and checks each connection's effective UID with
`SO_PEERCRED`; another effective user cannot control this service. The Automation
panel shows the actual path. [Linux Unix socket documentation](https://man7.org/linux/man-pages/man7/unix.7.html)
describes these permission and credential mechanisms.

Each request is one UTF-8 JSON line, at most 4096 bytes, with this envelope:

```json
{"op":"api","version":1,"id":"client-1","request":{"op":"state"}}
```

The optional correlation ID is a bounded string. Replies are at most 8192 bytes:

```json
{"ok":true,"version":1,"id":"client-1","result":{}}
{"ok":false,"version":1,"id":"client-1","error_code":"conflict","error":"State changed"}
```

The result above is illustrative; its fields depend on the requested operation.
Unsupported versions return `unsupported_version`. Legacy IPC operations remain
available independently. CLI errors exit unsuccessfully; direct socket clients
receive the structured error. At most eight clients are admitted. Snapshot reads
and socket writes have deadlines. A subscription sends an initial state then a
fresh page every 250 ms, with `event:"state"`; slow/disconnected readers close
without a growing event queue. Reconnect and subscribe again to recover current
state. This stream is state polling, not a lossless change log.

## Commands, edits and completion

State returns `expected` with a 32-character hexadecimal namespace and generation
and revision as decimal strings. Each `objects` entry contains a `target` with
that namespace, axis (`track` or `scene`), and a nonzero 16-character hexadecimal
object ID. Keep counters and IDs as strings to avoid numeric precision loss.

Supported actions are play, stop, scene launch, track gain/pan, crossfader, and
master gain. Gain ranges are 0–1.5; pan/crossfader ranges are 0–1, with pan centered
at 0.5. For example, build a command from a fresh state response:

```sh
state=$(omatainer ctl api '{"op":"state"}')
request=$(printf '%s' "$state" | jq -c '{op:"command",namespace:.result.expected.namespace,action:{op:"track_gain",target:.result.objects[0].target,value:0.75}}')
omatainer ctl api "$request"
```

`command`, `schedule`, and `edit` return `accepted:true`, a job ID, and
`status:"pending"`. Acceptance means admission; poll `{"op":"job","id":"…"}`
for `pending`, `applied`, `rejected`, or `cancelled`. Rejection exposes
`error_code:"not_applied"`. Up to 128 jobs survive client reconnect, but not app
restart; pending jobs are never evicted. A full pending table returns
`job_capacity`. Completed jobs can expire (`job_expired`).

`edit` accepts one atomic, undoable native rename, color, or move. Send `expected`
and `target` from the state page, plus `action` such as
`{"op":"rename","name":"Bass"}`, `{"op":"color","color":[20,80,160]}` (or null),
or `{"op":"move","position":0}`. Names have a 1024-byte UTF-8 limit. The renderer
rechecks namespace, generation, revision, sample rate and native transaction
receipt before applying. Concurrent edits prepared from the same revision cannot
both commit. Refresh state after a conflict or rejected job. Native Undo restores
the committed edit.

## Musical scheduling and safety

`schedule` takes the current namespace, a finite absolute quarter-note `beat`, and
one supported action. Transport must be playing; the beat must be in the future
and no more than 16384 beats ahead. The renderer keeps at most 64 scheduled
actions and dispatches at the musical sample boundary. Equal beats retain admission
order. Scene launches still obey native launch quantization. Tempo-map changes
alter wall-clock timing while preserving the requested musical beat.

`{"op":"cancel","id":"…"}` cancels a pending job before the renderer claims its
commit. Claimed/completed jobs return `cancel_conflict`. Stop, project replacement,
safety changes, or deleting the original target invalidate queued actions. Network
I/O and JSON parsing run outside the audio callback; callback scheduling has fixed
storage. `safe_stop` uses the reserved native safety mailbox. `emergency_silence`
requires `confirm:true`. Performance protection retains its native command rules.

## Optional loopback OSC

OSC is disabled by default. Enable **Preferences → Enable loopback OSC**, choose a
port (0 chooses an available port; explicit ports must be 1024–65535), then Preview
and Apply. Only 127.0.0.1 UDP binds. The Automation panel shows the actual bound
port and can reveal its 128-bit random token. The token is never saved in
preferences and rotates on restart or disable/re-enable. A bind failure preserves
the existing listener and displays the failure; cancel or failed preference saves
preserve the current listener.

Send one OSC message addressed to `/omatainer/v1` with type tags `,sb`: the token
as an ASCII OSC string, followed by the full UTF-8 API envelope as a blob. Replies
use `/omatainer/v1/reply`, type tags `,b`, and a JSON reply blob. Strings and blobs
use four-byte alignment and blobs use a big-endian length, as specified in the
[OSC 1.0 specification](https://opensoundcontrol.stanford.edu/spec-1_0.html).
Wrong tokens, malformed packets, oversized requests, bundles and subscriptions
reject. Use Unix subscriptions for state streams and API musical beats for timed
actions. UDP delivery is not guaranteed; inspect job completion over either
transport. No LAN listener or external network access is enabled.
