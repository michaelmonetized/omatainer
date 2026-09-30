# Issue #16: release the input's original voice

Live voices retain the Live/Clip category introduced by #4 and now also carry
an exact input key: MIDI connection ID/channel/note, or sampler pad identity.
Each actual MIDI connection receives a monotonic ID captured by its callback;
matching device names or channels cannot collapse independent inputs.

Note-off finds the key in the bounded voice pools instead of using the current
selection. A repeat note-on from the same gate releases its previous destination
and retriggers that gate at the current destination. Voice stealing replaces
the key along with the voice, so a later release from the old owner is harmless.
Existing clip-only release and plain offline synthesis APIs remain separate.

Instrument pads record their original track and actual pitch. Octave changes
transpose only held pad owners at their original destinations; MIDI and clip
voices are unaffected. Release uses the stable pad key even after selection,
scene, octave, bank, or sampler-instrument changes. Sample one-shots retain
their intentional tails. Existing routing to both the global sampler and the
selected track remains in place for #22; both owned gates are released.

Local verification:

```sh
cargo test --offline --locked --bin omatainer -- --test-threads=1
cargo build --offline --locked
```

The production build and all 64 tests pass on the #10 base, including eight new tests covering original
destinations after scene/selection changes, equal-pitch devices and channels,
independent clip/pad/MIDI gates, retriggers and stolen voices, instrument/octave
changes, finite one-shot tails, and velocity-zero note-off. A decoder-to-engine
test uses two identically named synthetic MIDI connections and distinct source
IDs. Input on/off and held-pad octave processing have zero measured heap
allocations. All existing clip lifecycle and arpeggiator regressions also pass.

This is synthetic engine/MIDI validation; no physical hardware was exercised.
