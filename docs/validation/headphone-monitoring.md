# Independent headphone monitoring

Issue #176 keeps cue material out of the audience mix in both the original
stereo renderer and the explicit routing graph. The separate headphone bus
has independent level, selected-deck PFL, cue/master blend and optional split
cue. Multiple selected cues sum before the headphone level. Cue taps follow
deck gain, EQ and filtering, before physical channel and crossfader gains.
Split cue sends half the selected stereo sum to the left and half the master
stereo sum to the right. Level, source, blend and split transitions use fixed
five-millisecond gain ramps.

In **Audio routing**, add a stereo output alias and choose it as the headphone
output. Review and apply the draft while stopped. The chosen physical channels
must be separate from every program connection's actual output map. Unused
channels in a wider alias do not create a false conflict. A missing headphone
pair remains silent and retains its identity; it never selects another pair.
The original NS7 still supports its verified output 3/4 fallback and physical
A/B versus Master policy when explicit program routes leave those channels
unclaimed. Physical NS7 mix/mode controls select that policy. Generic outputs
have no implicit headphone route.

The **Headphone controls** section selects PFL or A/B sources, cues, level,
blend and split. Published availability, output numbers and meters describe
digital routing. Quiet left/right routing checks produce one ramped second
of 997 Hz at minus 40 dBFS on the chosen headphone side. They require stopped
transports and an available pair. Cancellation, playback, route/project/output
changes and emergency stop end them. They cannot implicitly feed main.

The typed automation action `monitor` takes an adjacent tagged control, for
example `{"op":"volume","value":0.5}` or
`{"op":"pfl","value":{"deck":0,"enabled":true}}`. `source` accepts `pfl`
or `deck_mix`; `blend`, `mix` and `volume` require finite values in 0..1;
`split` and `master` require booleans; `tone` accepts side 0 or 1 and
`cancel_tone` stops a check. Invalid values/types and scheduled monitor actions
refuse admission. Unavailable or playing routing checks complete as rejected
jobs. `state.monitor` publishes applied settings and route availability.

The optional saved `monitor_output` alias uses nested routing format version
2. Existing version-1 routing documents remain unchanged and readable. New
choices persist with project routing through the existing review, Undo and
project owners. Older builds reject version-2 routes; retain a document copy
for rollback. Headphone level, source and split settings remain transient;
existing saved cue/master blend is preserved.

Nine new engine/native/API checks, existing NS7 regressions and the complete
1797-test software suite pass (43 explicitly ignored cases). The
[source-bound receipt](headphone-monitoring-receipt.json) retains the earlier
corrections, unfiltered result, exact binary and source inventory. Native slider
and API changes update the saved blend through dirty-state tracking and
Undo/Redo. Fresh ARM64 release `0.1.0+a0a0a52a933a` is installed at the default executable path, matching SHA-256 `2e156490f9f22f8d6398b7f51d1694a5c2a311fe2490eb38f596a883c84eff16`. The separate controller session's private GUI and control followers remain alive with their original executable; this update takes effect on the default app's next launch. No new
physical capture or listening test is requested by this thread. Simultaneous
physical main/headphone capture required for hardware acceptance remains open.
