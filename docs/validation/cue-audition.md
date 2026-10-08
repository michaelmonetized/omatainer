# Hold Cue audition

Each loaded deck has a visible CUE button. From pause, press stores the current
main cue and auditions from it while held. Release returns to that point and
pauses. Repeated taps stutter from the cue. Press Play during the hold to continue
after release. Cue during ordinary playback stops and returns to the stored cue;
releasing it does not restart playback. Main cue preparation retains its existing
library persistence; source audio files are never rewritten.

Hold the primary mouse button on CUE, focused Space/Enter, a touch contact, or the
deck's Cue shortcut (A/L by default). Pointer/touch movement outside the button
keeps its hold until release. Assistive Click toggles a visible hold; explicit
Press Cue and Release Cue actions are available. Set/return Cue in the command
palette, platter alternatives and MIDI Learn retains one-shot behavior.

MIDI Learn adds Deck hold Cue audition for exact-deck Note assignments. Note On
presses; Note Off and zero-velocity Note On release. Existing Deck set/return Cue
assignments remain unchanged. Existing factory Note Cue addresses use the hold
path; the original NS7's stateful raw Cue path already used it. No new hardware
addresses are inferred or physically qualified in this batch.

Independent local inputs merge only their renderer edges. Releasing mouse, a key
or one touch leaves another owner active. MIDI devices and remote clients retain
separate owners. Queue admission reserves the final release even under overload.
Focus loss, dialogs, source replacement, panel movement/closure and safety
boundaries retire local holds and require a fresh press. Key release uses the
captured deck and key even if modifiers, selection or focus changed.

Preferences version 17 admits the new learned action. Version 16 profiles migrate
without changing existing Cue assignments or custom beat-jump keys. New action
names under older headers are refused. Save remains explicit. Project schema 15
and portable shortcut schema 2 are unchanged.

The press/release, stutter and Play-latch reference is [Serato's Temporary Cue
manual](https://support.serato.com/hc/en-us/articles/233435527-Temporary-Cue).
Omatainer keeps its existing stored main-cue preparation described above.

Software qualification exercises native pointer, focused/global keys, assistive
and multiple touch inputs, actual learned MIDI workers, source retirement,
release reservations, source/project/safety changes and Cue/Play event orders.
An ordinary-playback stereo reference checks nonzero audio during audition;
callback allocation/free checks cover onset, release and rendered blocks. The
adjoining receipt records results, source hashes and any corrections. Physical
captures and new listening acceptance remain paused.
