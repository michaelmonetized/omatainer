# Beat jump

Each deck exposes backward, forward and size controls beneath its platter.
Choose 1/8, 1/4, 1/2, 1, 2, 4, 8, 16, 32 or 64 quarter-note beats. The initial
size is 4. The selected size survives source changes for this launch; it is not
saved track preparation or project state.

Shift+[ and Shift+] jump on the selected deck. Alt+[ and Alt+] select the next
smaller or larger size. Pending deck selection takes precedence over a lagging
renderer snapshot. Text editing, blocking dialogs, shortcut disablement, repeat
and safe mode retain their existing input guards. Commands and Preferences expose
these same actions and their effective bindings.

MIDI Learn exposes Deck beat jump backward, forward, smaller and larger. Assign
Note messages to either deck. One press jumps; Note Off and zero-velocity Note On
do nothing. Exact endpoint identity, channel, conflicts and ordinary queue
admission remain enforced. Factory hardware maps have no guessed new addresses.

The engine converts the current source position to the manual musical grid,
adds the signed size and converts back to original source frames. Without a manual
grid it uses the source BPM. Pitch, key lock and output sample rate do not change
the chosen source beat distance. Forward always means later source music, even
while Reverse is held. Jumps never start a stopped deck, change its stored cue or
create an Undo entry. The normal short jump transition also rebases spindle
playback; source PCM stays intact.

An active loop retains its boundaries. The destination wraps within its musical
span, including across tempo anchors. Integer jumps retain fractional beat phase
unless the file boundary clamps the destination or a non-integer loop wraps it.
Fractional jump sizes intentionally change beat phase. Bleep and slip return
clocks move by the same musical distance. Temporary roll/slice loops keep their
bounds while their return clocks jump within the underlying saved loop, if any.
Invalid bounds refuse the jump without discarding held clocks.

Preferences version 16 and portable binding version 2 introduce these action IDs.
Older profiles and version 1 bundles migrate in memory. Existing key owners are
preserved; a conflicting new jump default becomes disabled. Save is explicit.
New jump IDs under older headers are refused.

The reference pad workflow has separate backward/forward and smaller/larger
controls in [Serato's DDJ-GRV6 guide](https://support.serato.com/hc/en-us/articles/10864409704975-AlphaTheta-DDJ-GRV6-Quickstart-Guide).
This implementation defines its own loop behavior above; it makes no hardware
mapping or product-parity claim.

Qualification covers an independent mathematical position oracle and rendered
stereo null comparisons for constant/variable tempo, 44.1/48/96 kHz source and
output rates, pitch lock on/off, source bounds, loop wrapping, held return clocks,
zero callback heap work, native accessible controls, MIDI-learn input and strict
migration. Results and source hashes are in the adjoining qualification receipt.
No new physical gesture or listening checks are part of this software batch.

The consolidated full run also exposed a replacement-selection race in the existing
library UI. Removing its waiting-for-save label shifted the generated identities
and geometry of the candidate controls. The status row now stays present with a
reserved height through waiting and readiness. The real filesystem qualification
retains a candidate's accessible action across both states, checks identical ID
and bounds, selects it and completes verified relocation without changing source
bytes or the unrelated playing deck's rendered output.
