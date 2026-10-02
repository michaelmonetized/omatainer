# Issue 112: explicit MIDI device, port, channel and message routing

This layer adds profile-owned, independent track routing. It follows issue111
SMF interchange / PR467. Default profiles retain selected-track keyboard and
controller mappings; explicit routing overrides the generic note destination.
All routes use exact case-sensitive names and optional backend ids. Ambiguous
or missing outputs refuse rather than selecting another port.

## Behavior and boundaries

Preferences version6 migrates versions1–5 without an automatic write, preserves
all older settings, and rejects newer routing fields in an older schema. Each of
eight tracks may accept eight input port/channel-mask selections, monitor notes
internally, and send live thru and clip MIDI to an explicit output. Preserve
retains source channel;1–16 remaps channel messages. Input policy separately
admits controllers. Preview and Cancel change neither file nor live routing;
Save publishes a durable profile, then queues routing after input work settles.
Saved and applied generations, errors, retry and cancellation remain distinct.

Notes retain onset/release velocity, CC/bank/program, both pressures and bend
retain exact two/three-byte widths. Bank CC0/32 has a separate filter. SysEx is
opt-in, complete bounded F0…F7 with7-bit data,3–256 bytes and a complete extended
manufacturer id when prefixed0; it is opaque, with no vendor interpretation.
Realtime bytes may interleave complete callback messages. Fragmented/larger
SysEx and malformed channel messages refuse. No cross-callback running status
or fragmented reassembly is claimed. Existing realtime clock/transport remains.

Configured or active input devices/clients cannot also be output destinations,
including mapped-controller inputs. Backend ids resolve before open and active
sources recheck under publication admission. New inputs also check committed
outputs. Select controller inputs explicitly and deselect a bidirectional output
device from the global input policy. No packet-equality suppression is used:
legitimate fast repeated notes/controllers must pass. Unseen physical cable and
hardware thru loops cannot be inferred from packet equality; their verification
belongs to the user's hardware run.

Input callbacks only copy fixed data/read atomics. An input worker selects fixed
track commands with existing reserved release admission. Output discovery,
connect, send, reset, close and errors stay on one output worker. A2048-event
queue and256 renderer messages per audio block bound delivery. Explicit reset also stops current external clip streams until relaunch/edit/routing
apply, with a visible refused state; internal audio transport keeps running. On overflow,
older packets become stale before any subsequent send; an in-flight OS send
must finish before reset. A refused clip stream needs a new launch/edit/routing
apply. Callbacks allocate/free no heap. GUI close does not join blocked OS work.
Failure/teardown disables output admission; an exact explicit retry reconnects.

A route transaction releases registered input ownership and resets old outputs
before publishing a generation. Disconnect removes only that source's notes
and sustain; clip stop removes only clip ownership. Owners sharing a physical
port/channel/pitch combine into one gate: the first onset velocity lasts until
all owners release. Sustain combines active owners. Other channel controls
use received order when routes share an output channel. Route/device resets
send CC120/123/121, centered bend and zero channel pressure on all16 channels;
backend failures report unconfirmed reset rather than claiming hardware silence.

External clip MIDI uses source notes/channel lanes before internal arp/audio
mixing, retaining source timing/order/channel and both velocities. Internal
faders, mute, solo and arp affect internal sound. Loop boundary releases precede
new cycle onsets, final boundary lane events are retained, seek chases active
notes but not elapsed bank/program/controller changes, and recorded physical
first-pass notes are not duplicated by clip playback. Delivery is asynchronous
ASAP; audio-frame schedule proof is not calibrated hardware latency proof.

Preferences exposes native channel/type/port controls and Preview/Cancel/save.
The MIDI panel displays pending/applied generations, errors, per-track input,
routing, filtered/merged, sent, failed, overrun and clip-refused activity, plus cancel/reset.
Both windows scroll within the display height. Ordinary IPC status and follow
share a compact routing summary: enabled/generation, aggregate received/sent/
failed/overruns and a refused-track bitmask (bit0 is track1). Aggregate totals
saturate at u64::MAX. Detailed per-track counts stay in the native MIDI panel.
The unchanged8 KiB frame reserves telemetry space with56-byte MIDI display names
and232-byte deck display titles; truncation remains explicit. Native editor
names/titles are complete. Tests combine maximum routing counters and escaped
metadata/ids with the original maximum command counters and allocation caps. Sent means the backend accepted bytes.

## Software evidence

Focused routing v5:16 passed, one maintainer-only virtual-port probe ignored,
4.43 s. This includes real input handoff workers, guaranteed fixed-track
commands and actual renderer playback, two simultaneous controller fixtures,
all supported bytes, shared notes/sustain, disconnect, exact reconnect,
known feedback rejection, fast identical repeats, blocked open/cancel,
cancel after publication claim, recovery and bounded-output overflow. All128
note gates and source channel-lane messages cross the actual renderer at exact
independently calculated frames at48/96 kHz with the file conductor and no heap.
Pure playback additionally exercises maximum8192-note chase and loop ordering.

Native profile v1 passed actual App/egui/AccessKit controls, Preview/Cancel,
durable save, live receipt, preference load and startup reopen. Linux native
virtual v1 passed production Midir input/output managers on aarch64 ALSA with
only three task-created private ports, two separate source clients and one
receiver. All12 expected remapped message packets and route-reset bytes were
observed, and every private port was retired. No user hardware port was opened.
Receipts: `/home/michael/Projects/omatainer-work/issue-112-linux-virtual-v1.json`
and focused logs in the same work directory. Routing MIDI regression v6 passes132 tests (one opt-in ignored),28.89 s,
including recording first-pass suppression, stale renderer generation/epoch
admission, reset refusal/relaunch, unaffected-cell edits and both native UI
profile/failure workflows. Linux private virtual v2 and captured software trace
v2 recheck the current implementation. Full qualification is appended below.

Earlier routing v4 remains preserved:12 passed, two failed, one opt-in ignored.
The stress failure exposed repeated clip-clear admission into an already full
queue; current epoch checks and clear suppression corrected it. The disconnect
fixture used a nondeterministic per-track sent equality; the replacement waits
for all four aggregate wire packets and retains every ownership assertion.
Native regression v2 passed28, failed one, ignored five: its static accessibility
label assertion did not establish painted text. The fixture now inspects the
actual missing-port receipt painted by the MIDI panel on a720-line display.
The intermediate borrow-check failure in the recording fixture remains in
midi-regression-v3.log; no passing production qualification is claimed from it.

Reference requirements: [Ableton Live12 routing manual](https://www.ableton.com/en/live-manual/12/routing-and-i-o/).
The original MIT implementation bundles no vendor/reference-product code.

Inherited issue107 supplemental quiet-host wall maxima remain unresolved.
Physical multitimbral/controller compatibility, hardware loop behavior, listening,
Orca and audio-backend XRUN QA remain for the user's producer/composer/live-DJ
run with Pioneer DDJ-FX, Numark NS7 MarkII, APC40 mkII, MPD232 and MIDI keyboard.
No PR is merged and no issue is closed by this layer.

The first complete suite is retained as `issue-112-full-tests-v1.log`:1174
passed, two IPC-limit failures,26 opt-in ignored,229.48 s. Detailed routing
activity objects exceeded the unchanged8 KiB response limit with worst escaped
metadata. The compact summary and explicit display metadata reservation now
pass18 IPC server/follow tests (one opt-in ignored),11.01 s. Existing tests
additionally seed maximal routing counters/refused masks beside original
maximal command counters, escaped128-byte ids and oversized metadata, retaining
all original response, allocation, equality and zero-allocation cache assertions.
No response bound, performance policy or audio reference was widened.
Intermediate IPC v2 failures and native first-layout paint assertion failures
remain preserved. The native paint fixture waits for window layout to settle;
current missing-output/panel/reset and profile workflows pass in MIDI v6.

## Frozen local qualification

Qualified source: `702fa8bf7993c69aa0b753423e613ac600a72004`; routing feature
`4bede95d508395181ba4f24678af82a0e60508d1`. Linux aarch64, local controlled
release profile, debug assertions off, loader/incremental overrides unset.
Policy and original audio hashes are unchanged. The final gate ran on CPU6
from2026-10-02 11:09:05 UTC through11:16:59 UTC.

- Complete ordinary suite: **1176 passed,0 failed,26 opt-in ignored**,233.21 s.
- Standard gate: **eight workloads × three repeats passed**; callback allocations
  and frees zero. Original audio hashes: producer `26f84beea80f5ec5`, composer
  `e2f197b02633bd3d`, live DJ `d7711a2dc3b32a39`, hybrid `adc9540dab057fed`.
  All original state/input/crate/recording/project roundtrip checks passed.
- Native private Linux AT-SPI: **158 actions,246 visited nodes,555 frames**.
  Actual App/renderer plus native accessibility API; no window, Orca, desktop
  setting or physical hardware QA is claimed from this adapter.
- Frozen release test binary: private Linux virtual-port probev3 PASS,0.08 s;
  software two-source tracev3 PASS,0.07 s; native routing profile UI PASS,0.25 s;
  native missing-output receipt/retry/reset UI PASS,0.14 s. All12 expected wire
  packets plus route/teardown reset were captured (173 observed packets); every
  task-created port retired. No user hardware port was opened.
- License source inventory/check: **435** bound files with retained pinned notices.
  All8 license/package script fixtures pass. Frozen gate recheck, immutable
  package creation/verification,7 CLI groups,6 follow groups, runtime isolation
  and real headless safe startup pass on the actual packaged executable.

Release binary SHA-256:
`8abfe66b31f252fae0d7c0e08f039de44c75ff5bcc6c004d6626aa8b9e73c863`.
Release test binary SHA-256:
`6685b3904afafe52d74127861816c08c5288aaf24d27f5b3e88eeca4c160d583`.
Policy SHA-256:
`fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`.
Retained immutable artifact: `/home/michael/Projects/omatainer-work/issue-112-final-package`.
The source-bound summary and hashes are in `issue-112-final-qualification.json`;
raw timings and native action receipt are in `issue-112/target/performance.json`.
Captured final packets are `issue-112-linux-virtual-v3.json` and
`issue-112-routing-software-trace-v3.json` in the work directory.

No installation over the active application, merge or issue closure occurred.
The inherited107 supplemental wall-max failures and the user's final physical,
listening, Orca and backend XRUN QA remain explicitly unresolved.
