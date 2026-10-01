# Issue #62 — third mapped wet control regression

The prerequisite #55 implementation supplies three independent stereo master
FX slots, selected types and wet values in snapshots, and the third slot's
initial two-pole 1 kHz low-pass processor. This issue's separate change adds the
specific mapped endpoint regression and factory-binding contract requested by
#62. No duplicate processor or unrelated renderer change is introduced.

The existing legacy NS7FX channel-1 CC `0x32` controls zero-based slot 2. At MIDI
value 0, the output is dry; at 127, it is the selected processor's full wet
output. The initial selection is Filter. Other slots retain their values, and
snapshots report `[echo, reverb, filter]` plus the actual wet endpoints.

Validation (local private Cargo target):

- `cargo test --offline`: **283 passed**.
- `cargo build --offline`: passed.
- `git diff --check`: passed.
- The exact factory CC travels through the real synthetic MIDI callback,
  off-thread parser, command admission, renderer and snapshot publication at
  44.1/48/96 kHz. Both endpoint waveforms match independent mono dry/filter
  references sample-by-sample on both channels. The high band is attenuated
  while the pass band remains audible; minimum and maximum cannot be inert.
- Every current factory FX address is dispatched, including 11 wet bindings
  (seven targeting slot 2) and three selectors. Minimum/maximum wet values and
  selector changes affect only their intended supported slot and agree with
  published state. This test iterates the factory catalog rather than relying
  on a hand-maintained list of addresses.
- The existing #55 tests cover all three selected processors in every slot,
  independent stereo histories, reset behavior and real GUI type labels.

This is synthetic mapping and rendered-signal evidence. The legacy NS7FX map
is not physical qualification of an NS7 MkII or any other controller. Final
hardware QA remains the user's planned producer/composer/live-performance run.
