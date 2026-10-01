# Issue 88 — MIDI input preference application

This is the MIDI management prerequisite for the full Preferences issue. The
companion Preferences change owns durable profiles, Preview/Cancel, startup
integration and the GUI. This component does not save preferences itself.

`MidiHub::start_with_policy` accepts All, Selected exact port names, or Disabled;
`start` preserves the default All behavior. Selected names are case-sensitive,
unique, and bounded to 64 names of 1–1024 bytes without NUL. If several actual
ports have the same selected name, each remains an independently owned input.
Missing selected devices never broaden the policy to All. The existing exclusion
of MIDI Through ports remains in place. This is an input policy; it does not
change output-port selection or add hotplug polling.

`configure_inputs` validates and publishes a monotonic request generation without
waiting for discovery, connection work, teardown or queue capacity. One bounded
wake slot coalesces requests; the manager reads the latest immutable policy after
an OS operation returns. `policy_status` exposes separate requested/applied
policies and generations, the latest successful discovery preview (at most 256
unique selectable names, with an explicit truncation flag), missing selected
names and a bounded error. The preview cap does not restrict exact matching of
an explicitly selected name. Reads clone one immutable Arc without allocation.

The manager retains permitted connections and source IDs. Excluded connections
close their callback before joining their input worker, which sends source-owned
releases through the existing CommandPort. Applied acknowledges completed
management and queued releases; the audio renderer consumes those releases in
its normal bounded FIFO. Keyboard and mouse commands remain available throughout.

A connecting input is atomically gated until the backend returns. Activation,
new policy publication and shutdown share one short lock, so an excluded or
closed connection cannot become newly enabled after that decision. The only
production activation operation inside the lock is an atomic enable store;
backend calls, joins, status publication and raw/audio callbacks never acquire
that lock. Raw MIDI callbacks gain one atomic enabled check and otherwise keep
the existing fixed-size handoff and overflow behavior.

An OS discovery/connect call cannot be forcibly cancelled. While it is blocked,
a newer request stays visibly pending and the GUI/renderer continue. Manager
shutdown returns without joining that call; the worker retains ownership and
finishes teardown when it returns. No hardware deadline or device compatibility
claim is made from these fixtures.

Validation uses a controllable backend through the production manager, raw input
sink, input worker, CommandPort and real renderer:

- Disabled startup and case-sensitive selection, explicit missing names, and
  permanent keyboard/mouse availability.
- Two same-pitch held voices on separate sources; removing one source preserves
  the other connection and voice, and Disabled releases the remaining source.
- Blocked connection plus 129 preference edits: raw input remains inert, controls
  and audio blocks progress, and only the latest policy is acknowledged.
- Eight simultaneous callers produce 256 unique ordered generations, then one
  latest discovery/connection pass without a queued-job backlog.
- 263 discovered names exercise the bounded preview; a selected name outside
  that preview still connects and is not falsely reported missing.
- Disabled closes/releases an existing source before a held discovery fails;
  the error is bounded and the input policy remains applied.
- Connection failure and retry retain the selected filter and generation receipt.
- Invalid requests and unavailable manager responses, strict serialized shape,
  allocation-free raw gated-input calls and immutable status reads.
- A deterministic pause at the activation decision proves that policy/shutdown
  cannot publish between the allowed check and enabled store.

Commands use the private existing Cargo target:

- `cargo test engine::midi -- --test-threads=1`: 57 passed.
- `cargo test`: 464 passed, 5 opt-in tests ignored.
- `cargo build`: passed.

No installed binary, desktop configuration, audio device, MIDI device or live IPC
endpoint was changed by these tests. The parent stack reruns combined Preferences
and application acceptance after applying this prerequisite.
