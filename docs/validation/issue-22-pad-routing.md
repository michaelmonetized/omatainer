# Issue 22: one pad source and one mixer destination

Instrument pads now trigger only the sampler Poly. Each source frame is routed
once into a fixed stereo bus for the track captured at onset, then processed by
that track's EQ, FX, gain, pan, mute and solo gate. Sample one-shots carry their
own destination as well. Source instrument/octave behavior is independent of the
track instrument. A retained route keeps synth releases on their original track;
a sample stores its route independently so later pad activity cannot move it.

Track EQ has separate channel history, including output-rate reconfiguration.
Track FX uses its existing stereo processor entry point. No master/scene effect
routing policy is changed in this layer.

Captured MIDI and pad notes remain immediately visible in the clip, with issue
20's actual held duration. Runtime metadata keyed by the original note index
prevents that new clip note from mirroring its monitored input. It stays excluded
while physically held, including after Record-off. On release, the first eligible
on/off pair is shifted together to a future occurrence, no earlier than the next
loop after capture. One-shots join playback on a later explicit launch. The same
policy filters the arpeggiator's chord cache. Ordinary edits still chase notes;
other notes at the same pitch are independent. Stops, explicit replacement and
relaunch clear the policy; project note serialization is unchanged.

Validation:

- `cargo test`: 134 tests passed on the assembled issue-21 prerequisite checkout.
- Eight new tests exercise actual interleaved output for all pad instruments and
  drum/synth/audio destination kinds against a single-source numerical reference;
  sample and instrument mute/zero gain/pan/solo/FX; right-only stereo sample gain;
  original destination through selection changes and release/one-shot tails.
- Capture tests cover MIDI and pads, ordinary scheduling and arpeggiation, holds
  across multiple loops, Record-off while held, instantaneous gates, one-shots,
  natural one-shot capture finalization, subsequent edits/relaunch and unrelated
  same-pitch events. Counts observe the actual Poly trigger and MIDI gate trace.
- Existing scheduler append/edit tests retain ordinary chase and exact-boundary
  reconciliation checks. Prior fixtures expecting duplicate pad voices or captured
  note preview playback now assert the deliberate single-monitor policy.
- Warm pad routing, octave changes, releases and rendering have zero measured
  allocations/reallocations/deallocations. Recording insertion can still grow its
  note list and matching policy vector; this does not claim allocation-free edits
  or snapshot publication.
- Existing output-rate tests also check the added right EQ coefficients.
- `cargo build` and `git diff --check` passed. These are local engine tests and
  build checks; hardware controller and final mode QA remain separate.
