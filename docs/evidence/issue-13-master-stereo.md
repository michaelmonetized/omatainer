# Issue #13: master effect timing and stereo state

The master delay and reverb now each own independent left/right histories.
Each processor advances once per output frame for its channel. The existing
serial delay-then-reverb routing and mix controls remain the same; both channels
receive the same tempo, delay duration and wet controls at each block boundary.

The stereo policy is dual mono: no crossfeed or decorrelation is introduced.
An isolated channel stays isolated, distinct stereo signals remain distinct,
and identical left/right inputs produce identical outputs.

Master reverb comb delays retain their original 48 kHz durations in seconds at
other output rates, rounded to the nearest frame. `Reverb::new()` retains the
48 kHz defaults for existing non-master callers. A master sample-rate change
resets both channels' delay/reverb tails together and reapplies the shared wet
controls on the next block. Other rate-dependent processors remain issue #21;
slot wet/dry equations remain issue #54.

## Local evidence

`cargo test -- --nocapture` passed all 45 tests on the stack through issue #7.
The four new master regressions exercise the real `RtEngine::process` path:

- Isolated left/right impulses at 44,100 and 48,000 Hz and 90/120/180 BPM,
  with 64-, 511- and 1,024-frame block sizes. Delay arrival equals the configured
  output-frame duration, with equal neighboring-frame interpolation for a
  half-frame delay. The original 48 kHz / 120 BPM fixture arrives at **18,000
  stereo frames**, rather than the original 9,000.
- Reverb's first reflection arrives at frame 887 at 48 kHz and frame 815 at
  44.1 kHz, within half an output frame of the same 887/48,000-second duration.
  Its gain and sign match the original comb, and the silent channel stays zero.
- Distinct 233/997 Hz stereo sines and dual-mono sines match two independent
  mono EQ/delay/reverb reference chains within 1e-6 at both sample rates,
  across dry, partially wet and fully wet combinations. Dual-mono output
  channels remain exactly equal.
- Tempo and wet control updates reach both channels. Rate changes between
  48 and 44.1 kHz reset primed histories without retaining either channel's
  old tail.

Fixtures render 256 silent preroll frames before the measured source so deck
load transitions are outside the master measurement. `git diff --check` passed.
These are local sample/renderer assertions; no physical audio-device latency,
underrun or controller behavior is claimed.
