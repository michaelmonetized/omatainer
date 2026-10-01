# Native projects: engine model and application boundary

The native `.omat` container stores all currently implemented editable musical
state. It does not serialize the UI snapshot: that projection omits notes,
nonvisible racks and actual cue positions. The independent strict project model
contains all eight tracks and scenes, complete clip names/kinds/bars/notes/gain
and audio attachments, track gain/pan/mute/solo/arm/instrument/tuning/EQ/routing,
all track and scene rack order/types/enabled/mix/parameters, three master FX,
master/cue/crossfader settings, sampler banks and their exact sample references,
instrument/octave, transport tempo/grid/metronome/position, deck attachments,
playheads/cues/hotcues/loop bounds/arming/pitch/range/sync/vinyl/keylock/filter/EQ,
and engine selection/view/rack edit target. The GUI document adds its own view
and library identity metadata; see the GUI validation companion.

The container embeds exact sample PCM and metadata and deduplicates shared
attachments by their immutable Sample identity. The saved source path is retained
as metadata; reopening does not reinterpret that pathname as proof of matching
library bytes. A saved file can open with the original media absent. Existing
factory mappings have an explicit compatibility revision in the GUI document.
There is no editable automation, warp, plugin-host or custom mapping system in
this revision. Those future feature layers must extend the typed project model
and round-trip tests when introduced. Unknown fields/versions are rejected;
there is no silently ignored placeholder for those future features.

Opening starts stopped. Saved deck playheads and clip resume targets remain in
the document. Play resumes the saved clips; Deck Play starts each paused deck.
DSP histories, scratch motion, physical held keys, metering, background jobs and
OS device connections are process state. A canonical fresh render starts with
new processor histories. Opening never reconstructs demo notes or automatically
emits saved held physical gates. New creates an empty eight-by-eight session with
available factory resources and unloaded decks.

## Ownership and concurrent edits

One bounded request/result handoff connects the project worker and renderer.
The first capture pass reports string/vector capacity requirements; the worker
reserves them. A subsequent callback copies a coherent block boundary using
preallocated storage and Arc increments. Serialization, deduplication, file I/O,
validation, DSP construction and retirement of the replaced graph run on the
worker. The audio stream, live command receiver and snapshot publisher retain
their ownership. A full reply queue retains its payload on audio until it can
be returned; it never drops a last media/processor owner there.

Install closes creative command admission using a single atomic closed bit and
producer lease count. Already admitted producers finish, and their queued
commands drain before the revision check. The callback never waits for a
producer mutex. New creative commands receive an explicit ProjectChanging
rejection. Physical note/pad/touch releases, zero-velocity note-ons and emergency
stops remain admitted, so cancellation cannot strand held input. Queued GUI
browse/load intents cause install to conflict and remain deliverable to the old
project. Cancellation wins before the application claim; a successful commit
wins a cancellation arriving after that claim.

A changed revision rejects replacement, preserving edits made while opening.
Note onsets, final durations and continuing held recording durations update that
revision. Navigation that is stored in the document also changes the revision;
transport ticks and ordinary physical performance gates do not. Repeated
assignments can conservatively leave the document dirty. Saves carry their
captured revision; continued recording cannot be marked clean by an earlier
save. The GUI waits for a published snapshot at least as new as Applied before
reenabling creative controls, avoiding stale deck-target/view actions.

Clean/Save-and-Close obtains a separate admission seal and rechecks the saved
revision, pending GUI intents and active recorded holds. A late edit conflicts
instead of slipping between a clean check and the window Close event. Explicit
Discard uses a distinct policy that can discard active recording. A lightweight
CloseGuard keeps admission sealed through the final Close; dropping it requests
reopening on the next callback without GUI blocking or large object destruction.

## Format bounds and evidence

State version 1 supports 8,192 notes per clip and 65,536 notes total, 128 slots per
rack, 16 sample banks and 4,096 bytes per editable name. Over-limit state receives
an explicit error rather than truncation. File limits independently allow 8 MiB
metadata, 256 distinct assets and 1 GiB embedded PCM; see
[codec validation](issue-82-codec.md) for exact bits, checksums, atomic replacement,
interrupted-save fixtures and durability-warning semantics.

Local engine fixtures exercise:

- Every current field, all clip/audio attachments, nonvisible racks and controls
  through capture, preparation and recapture.
- Exact canonical rendering across 44.1/48/96 kHz output and different block sizes,
  plus a real fresh child process reopening a native file and matching its
  reference render hash.
- Zero allocations/frees in both capacity probe and prepared capture, and in the
  owned graph swap. Replaced payloads remain owned for worker retirement.
- Actual asynchronous capture/install, edits queued before install, conflict,
  cancellation, subsequent retry, concurrent-operation Busy and blank New.
- Stopped reopen, explicit resume, old held release isolation, edited notes before
  resume and the first arpeggiator sixteenth.
- Continued held recording dirtiness, strict new-version/unknown-field/invalid
  note/float/reference validation, clean-close seals, explicit Discard and
  deferred controller GUI intents.

The largest capture fixture contains 65,536 notes, 2,048 rack slots, 16 banks and
4,096-byte names. Nine local prepared copies took median 33 microseconds, maximum
58 microseconds in the recorded development run, with zero allocations/frees.
This measures state-copy wall time on this Linux AArch64 host, not an audio-stream
XRUN/deadline guarantee or a claim that rendering 2,048 FX is practical.

Hardware audio and MIDI device QA is still the user's separate producer/composer/
live-DJ acceptance run. No physical controller, loopback latency or hardware XRUN
claim is made by these private headless engine/egui/file fixtures.
