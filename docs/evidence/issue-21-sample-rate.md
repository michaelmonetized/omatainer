# Issue #21: prepare every processor for the output sample rate

`RtEngine::set_sample_rate` is preparation for a stopped renderer. Production
calls it in `audio::start`, before constructing the CPAL callback and starting
the stream. There is no running-callback rate-change command. Real device
selection/switching remains issue #94.

The rate-dependent inventory is now covered as follows:

| State | Rate behavior |
| --- | --- |
| Track and sampler instruments | Rebuild every voice's ADSR coefficients in place and clear oscillator/filter history; preserve instrument, cutoff and voice capacity. Oscillator increments already use Hz/output rate. |
| Track and both deck EQs | Rebuild one-pole coefficients, clear histories and preserve all gain controls. |
| Synth, deck and FX state-variable filters | Clear histories on reconfiguration; processing uses the current rate. Preserve the original 48 kHz cutoff bounds in Hz, with Nyquist headroom, instead of changing the upper cutoff with rate. |
| Master delay/reverb | Rebuild both channels, retaining the shared controls. Delay duration follows tempo on the next block; reverb combs retain their 48 kHz durations rounded to the nearest frame. |
| Every track/scene FX slot, including bypassed slots | Rebuild only its private processor state; retain order, type, bypass, mix and parameters. EQ coefficients and delay, reverb, chorus and Haas buffers use the new rate. |
| Slot delay | Preserve its existing 48 kHz 12,000-frame duration as 250 ms. Its existing wet law is unchanged (#54). |
| Chorus and Haas | Buffer capacities follow the output rate; existing second-based delay formulas and the 0.7 Hz chorus oscillator continue at the new rate. |
| Compressor/gate detectors | Convert the original per-sample smoothing into the same time constants at the new rate. |
| Generated drums/pad banks | Regenerate at the output rate, including rate-scaled hat filter smoothing. Clear positions referring to old generated buffers. |
| Deck overlap grains/rate smoothing | Scale the original 1,024-frame window and half-window to the same duration, rounding the hop to a whole frame. Rate smoothing retains its time constant. |
| Loaded audio, pad playback and deck source positions | Source buffers retain their own sample-rate metadata. Existing source/output rate ratios preserve pitch and duration without resampling the source asset during reconfiguration. |

Actual rate changes discard old effect tails, held/releasing synth voices and
filter/grain history. They clear active finite drum/pad voices. Musical beat,
clip starts, deck source positions and controls remain intact. Ordinary MIDI
clips chase their currently active notes on the next rendered sample; arps
resume at the next step. Decks fade from silence over the existing 2 ms
transition. Equal rates leave all state and tails untouched and allocate
nothing. A zero-rate request is ignored.

Rate preparation owns its allocations before callback ownership begins. No
buffer growth or coefficient rebuild was added to per-sample processing.
Publication and unrelated command allocation are separate work (#25/#59).

## Local evidence

`cargo test -- --nocapture`: **92 tests pass**, on the assembled stack through
#13 plus #14. Nine new regressions construct at 48 kHz and reconfigure to
44.1 and 96 kHz:

- All track and sampler voice kinds retain attack, decay and full-scale release
  times within 2 ms (including the existing floating-point linear-envelope
  rounding). A4 oscillator phase completes 440 periods per second within one
  period. Generated sample metadata matches the new rate.
- Every track/scene rack, including primed processors bypassed at reconfiguration,
  matches a freshly prepared rack exactly on stereo impulses and distinct sines.
  Slot order, bypass and control values survive the reset.
- Slot delay, reverb and Haas impulse arrival times remain within one output
  frame of their intended durations. Master delay/reverb arrivals match their
  expected per-channel sample indices, including the half-frame interpolation
  of a 375 ms delay at 44.1 kHz.
- Two seconds of stereo chorus output match independent raw delay processors
  exactly. Its first modulated echo is within two samples of the continuous-time
  8 ms / 0.7 Hz delay equation. Compressor/gate threshold crossings retain their
  original time constants within one sample.
- Track/deck EQ impulse decay agrees with the independent exponential response
  within 1e-5. Filter/EQ sine responses at 80/250/1,000 Hz stay within 4% of the
  48 kHz reference. A separate high-cutoff SVF probe measures gain 0.5 within
  0.002 at the original 6,875.49 Hz cutoff cap at 44.1/48/96 kHz.
- Reset and same-rate no-op assertions cover voices, finite samples, musical
  positions, loaded source identity, controls and next-sample clip chase.
  OLA window duration stays within one sample; deck smoothing matches its
  time-domain reference. Existing pitch-lock and source-transition tests pass.
- Allocation instrumentation observes allocation during preparation, then
  **zero allocation/free activity** in a warmed 1,024-frame production callback
  after it takes ownership. This block excludes periodic snapshot publication.

`cargo build` and `git diff --check` pass. These are local synthetic DSP and
callback tests; no physical device switch, hardware latency or underrun claim
is made.
