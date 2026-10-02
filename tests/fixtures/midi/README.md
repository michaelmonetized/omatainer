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
required independent DAW and shipped GUI round trip are still pending.
