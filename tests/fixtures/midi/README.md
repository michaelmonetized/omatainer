# Original MIDI interchange fixture

This synthetic file is original project material under the root MIT license.
Regenerate it with `python3 tests/fixtures/midi/generate.py`; generation uses
Python's standard library and does not call the Rust MIDI reader or writer.
It is test-only and is not installed as factory material.

The format-1 file has two tracks and PPQN 960. Its end is tick 61440: 64 quarter
notes, or 16 bars under the initial 4/4 meter. The conductor changes from 500000
to 666667 microseconds per quarter and from 4/4 to 7/8 at tick 30720. There are
64 note pairs on channels 1 and 3 (zero-based 0 and 2), two program changes and
nine CC1 values. Notes start at `960*i+1`, last exactly 719 ticks, have pitch
`60+i%8`, onset velocity `80+i%32`, and release velocity `i%64`.

A local Mido 1.3.3 parser/writer cross-check passed these exact integer ticks,
channels, velocities, controller/program values, tempo/meter events and end
silence through both writers. This is independent library evidence; the
native UI also imported it, saved/reopened a project and exported identical
supported events. The independent DAW return is recorded below.

`ardour-9.8-region-ppqn19200.mid` is Ardour 9.8's real MIDI-region export of
that native UI output. The isolated Linux GUI used the None (Dummy) backend
at 48 kHz/256 frames, with private XDG state and a task-owned Xvfb display.
Ardour's note model retained all 64 starts/durations/channels/on/off velocities
exactly at its 1920-tick musical division. Its saved session retained the
tempo and 4/4 → 7/8 change at quarter 32.

Ardour exports the region as SMF 0 at PPQN 19200 and does not export the session
conductor. It adds four bank-select controllers and 134 proprietary meta
events. Its default region end omitted the last boundary CC; extending the
region to 65 quarter notes before export retained all nine original CC1 values
and both programs. This file records those actual DAW differences. It is a
derivative of the original MIT fixture, with no Ardour source code included.

The native UI regression test imports this unmodified return, requires explicit
proprietary-event omission review, keeps the existing session conductor and
verifies exact notes, all CC1/program values and the 65-quarter end. Export then
includes the retained conductor and supported returned lanes. A region-only
DAW export is not evidence that its absent conductor survived inside the file.
