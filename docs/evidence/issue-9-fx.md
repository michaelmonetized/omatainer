# Issue #9: independent FX instance state

Each `FxSlot` constructs and owns the state for its effect at the engine sample
rate. Delay, reverb, chorus, Spread, filter and EQ keep separate channel histories.
Each compressor/gate has a stereo-linked envelope detector; each chorus has its
own modulation phase. Arpeggiation remains the upstream MIDI processor.

Slot processing is serial stereo: later processors receive both outputs of the
previous slot. There is no shared processor selected by effect type, no processor
allocated lazily on the first sample, and no always-on chain-level Haas buffer.
Effect type is read-only (`id()`); replacing a type requires a new slot constructed
with `FxSlot::new(id, sample_rate)`.

## Lifetime policy

- Parameter/mix edits preserve the selected slot's history.
- Bypass returns dry input and freezes that slot's history without running its
  processor. Other slots continue processing. Re-enabling resumes the frozen
  tail. Bypass transitions are immediate; no crossfade is claimed.
- Removing or replacing a slot discards only its history. Surviving slots retain
  their state when the vector moves them. A newly constructed slot starts empty.
- Tail state belongs to the slot, never its vector index or effect type.

## Deliberate audio differences

An empty chain is now an exact stereo identity. The former chain-wide Haas tap
delayed the right channel by one sample even with no Spread slot. That unintended
tap is gone, including after a single mono effect. A neutral Spread is also an
identity. A single non-neutral Spread or Balance retains its amount behavior;
Spread owns its own left/right delay history. Multiple width/balance processors
now occupy their actual chain positions, so stereo changes survive downstream
processing and duplicate Spread delays cannot share a buffer.

The time effects' existing double wet interpolation is retained for issue #54.
Spread/Balance's existing disregard for slot mix is retained for issue #60;
their separate instance/chain order behavior is necessarily established here.
This change does not add consumers for the generic controls tracked by #75.

## Validation

`cargo test --locked` includes numerical comparisons for two slots of all 13
types against separately constructed primitive-based reference processors. The
oracle never constructs an `FxSlot` or `FxChain`. It covers delay, reverb, chorus,
compressor, gate, filter, EQ3/5/8, Spread, Balance, drive and Arp's audio passthrough.
A mixed chain additionally checks compressor/gate and different EQ types together
with serial stereo effects. Reference tolerance is 0.000001 per output sample.

The impulse fixture asserts the first echo at sample 12000 with one fully wet
zero-feedback delay and sample 24000 with two in series. Lifecycle fixtures edit,
bypass/re-enable, remove and reinsert slots while checking the surviving history
against independent references. Separate stereo fixtures check channel isolation.

At 48000 Hz on this 64-bit build, a slot occupies 424 inline bytes. Additional
heap storage is allocated only for its selected processor (measured by the test
allocator): Delay 768000 bytes, Reverb 115680 bytes, Chorus 19200 bytes, Spread
7680 bytes; all other processors use zero heap bytes. The larger time-effect
storage provides independent channels and instances. An empty chain has no DSP
heap buffers. Control-to-audio construction/disposal safety remains issue #10.

These are isolated numerical and allocation measurements, not hardware listening
or device callback deadline qualification.
