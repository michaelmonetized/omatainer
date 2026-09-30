# Issue 17: stopped source silence

A stopped, untouched deck retires its last output through the existing two-millisecond transition envelope, then emits exact zero without reading a frozen source sample. An initially stopped deck is exactly silent. Cue, pause, media replacement, unload and EOF follow the same bounded envelope. Pitch-lock mode uses the same gate.

The deck EQ/filter histories are cleared by the source transition. Downstream master delay and reverb remain running: their existing finite-input tails are allowed to decay. This distinguishes a silent source from an effect tail. Intentional scratch contact can read and move a stopped source; releasing contact fades it to zero again.

Four new test functions cover both decks, 44.1/48 kHz, both pitch-lock states, exact fade endpoints, nonzero pause positions, stable stopped playheads, explicit restart from playable positions, default-session silence, touch/jog/release and full-engine downstream effect tails. Delay and reverb tails remain nonzero after the source fade and decay below one percent of the initial tail peak in the final second of a four-second render. Prior transition fixtures now expect silent stopped sources and verify marked source regions after playback resumes.

Local isolated `cargo test --locked`: 30 passed. These are generated-sample checks without hardware or a listening claim.
