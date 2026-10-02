# Issue 111: Standard MIDI File interchange

Project-menu import/export now exchanges supported musical events with exact
source ticks, reviewed mapping/merge/conductor choices, guarded one-step Undo,
native persistence and cancellable worker IO. This layer depends on issue 110's
piano roll. It adds no external MIDI device/channel output routing.

## Supported behavior and bounds

SMF 0 has one track; SMF 1 has 1–128 parallel tracks. PPQN is 1–32767. Format 2,
SMPTE division and RMID are rejected. Input and output are bounded at 16 MiB,
262144 events and 65536 notes. Strict readers validate chunk lengths, VLQs,
running status, status widths, values, end markers and matched note gates.
Same-channel/pitch overlaps pair FIFO. Dangling note-ons refuse; orphan releases
are retained and reported. Unsupported SysEx/proprietary events, header/chunk
extensions and unknown meta events require explicit omission review.

Notes preserve onset/release velocity, channel and integer start/duration.
Standard channel messages, text bytes, key, tempo, meter and trailing silence
are retained in immutable native lanes. Native track instruments use the track
sound; imported controller/program lanes are interchange data. The source-lane
inspector shows their actual values. Export writes source coordinates, without
flattening clip loop/launch transformations. Muted-note inclusion, SMF 0/1,
PPQN and current-session/original-source conductor are explicit choices.
Inexact conversion refuses until nearest-tick rounding is enabled.

Track rows can be ignored or split by channel. Several rows can share a cell;
replace and merge are distinct choices. Channel splitting attaches metadata to
the first row of each file track, as explained in the dialog. Audio destinations
refuse. Tempo/meter conflicts require keeping the session or selecting an
authoritative file track. File conductor playback supports 40–240 BPM, 4096
points of each kind and denominator values through 128. Kept-session imports
retain other source tempos for original-source export.

Conductor playback uses integer microseconds rather than a rounded BPM. Tempo
dependent master effects update at map transitions. Metronome denominator beats
and accents follow the meter; the toolbar displays actual meter/bar/beat. Manual
tempo changes remove the map and Undo restores it. Ordinary unmapped transport
keeps its existing clock and unchanged audio reference behavior.

Schema 6 persists lanes, conductor, note channels, release velocities and exact
source timing. Earlier schemas reject the presence of these fields, including
null, and continue to migrate their original notes deterministically. Exact
ticks remain authoritative while their visible projections match. Pitch,
velocity and mute edits retain them; time edits replace stale source timing.
The piano roll exposes precise source values separately from visible projection.

Imports respect 8192 notes per cell, 65536 per session and 16 MiB of MIDI lanes.
The native project metadata bound is 64 MiB. Worker preparation checks the
prospective state serialization and reserves 128 KiB for the GUI envelope.
One import uses up to 64 clip inverses plus a conductor inverse. Every target,
project epoch and conductor baseline is rechecked before mutation. Changed
targets, held recording, protection and canceled claims leave the whole session
intact. Live mixer gains are retained. Renderer application, rejection and
Undo/Redo allocate/free no heap; owned payloads retire on the existing worker.

Inspection opens regular files without following a leaf symlink and rechecks
descriptor/path fingerprints. Preparation/export/encoding poll cancellation.
Export writes/synchronizes a private temporary and atomically claims an unused
destination. Existing/racing/symlink destinations are never replaced. Cancel or
a directory replacement leaves no published/temporary file. Post-publication
durability errors explicitly report publication. The review window scrolls on
a 720-line display; source rows/events/warnings are virtualized and numbers use
the existing native accessible numeric route. Offline help covers seven controls.

## Independent library and DAW evidence

The original fixture generator is Python standard-library code, independent of
the Rust reader/writer. Its two tracks contain 64 notes at PPQN 960, channels
0/2, a one-tick onset offset and 719-tick durations, varying on/off velocities,
nine CC1 points, two programs, two tempos and a 4/4 to 7/8 change at quarter 32.
Its end is quarter 64. Native Project-menu import/save/reopen/export preserves
every supported event and the trailing silence. Mido 1.3.3 independently read,
wrote and reread both directions with exact integer comparisons.

Ardour **9.8**, Linux aarch64, independently imported the actual native UI export
using its GUI `Editor:do_import` with `SMFTempoUse`. It ran at 48 kHz/256 frames
on **None (Dummy)**, a task-owned Xvfb display and private XDG/DBus state. No host
desktop configuration or physical audio connection was changed. Ardour's note
model assertions verified all 64 starts/durations/channels/on/off velocities
exactly at its 1920-tick musical division. Its saved session XML independently
retains tempo 120 → approximately 89.999955 BPM and meter 4/4 → 7/8 at quarter 32.

Actual `MidiRegion:do_export` returned SMF 0 at PPQN 19200. Ardour omits its
session conductor from region export, adds four bank controllers and 134
proprietary meta events, and its initial region end cut the last boundary CC.
Extending the region to quarter 65 before a second export retained all nine
original CC1 points and both programs. These actual differences are preserved
in the unmodified committed `ardour-9.8-region-ppqn19200.mid` fixture.

The actual Omatainer App UI imports that return only after proprietary-omission
review, keeps the existing session conductor and verifies all note values,
all nine CC1 points, both programs and the 65-quarter end. Native UI export then
includes the retained conductor, bank/controller/program lanes and no unsupported
events. A region-only export is not evidence that its absent conductor survived
inside the returned file. The round trip explicitly uses the retained-session
conductor choice and independently checks Ardour's saved conductor.

External DAW receipts are in
`/home/michael/Projects/omatainer-work/issue-111-ardour-probe-v1/`: the imported
session, genuine returned files, model-assertion screenshot, dummy-backend
screenshot and `daw-verification-v1.json` with content hashes. The first boundary
export remains preserved alongside the successful extended export. Task-owned
Ardour, DBus/AT-SPI and Xvfb processes were stopped after capture.

Primary references: [Ardour import example](https://github.com/Ardour/ardour/blob/master/share/scripts/s_import_files.lua),
[MIDI region export implementation](https://github.com/Ardour/ardour/blob/master/libs/ardour/midi_region.cc),
and [Ardour manual](https://manual.ardour.org/ardourmanual.html).
No Ardour source is bundled in this MIT implementation.

## Qualification

The first full suite passed 1154 tests and failed two rate-pruning fixtures whose
fixed 1100 KiB budgets assumed the earlier note struct size. Those fixtures now
derive their one/two held inverse reservations from `size_of::<MidiNote>()`,
assert that pruning is still forced, and retain their original ownership, budget,
saved-checkpoint and Undo/Redo assertions. All 50 rate-filtered tests then passed
(one maintainer opt-in ignored). No production memory budget, performance policy
or audio reference was changed to accommodate these failures.

Qualification results are appended after the final source freeze. Focused
evidence already covers the original and actual DAW-returned native UI workflows,
worker-held/queued cancellation, source replacement, malformed inspection,
exclusive publication races, 64-cell/conductor atomic Undo, precise high-PPQN
timing, merge rescaling, native save above the previous 8 MiB bound, and every
one of 128 gates over the complete tempo-changing fixture at 48/96 kHz with no
callback heap work. This is automated App/egui/AccessKit/renderer evidence;
it is not a claim of an Omatainer desktop window, Orca or physical hardware QA.

The inherited issue 107 supplemental quiet-host wall-max failures remain
unresolved. A passing unchanged standard gate does not erase them. Final
producer/composer/live-DJ listening, hardware integration, Orca and backend XRUN
qualification remain for the user's final run. No issue was closed or PR merged.
