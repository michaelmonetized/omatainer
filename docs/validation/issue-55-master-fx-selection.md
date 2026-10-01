# Issue #55 — legacy master FX selections select real processors

The three legacy FX controls now address three serial master slots. Each owns
independent left/right histories for every supported type, prepared before
streaming. The supported cycle is exactly Echo → Reverb → Filter:

- Echo: existing 3/4-beat delay with its existing feedback and two-second storage.
- Reverb: existing rate-scaled four-comb reverb.
- Filter: independent two-pole 1 kHz low-pass on each channel.

Each slot's Wet control mixes its selected processor. Defaults remain Echo in
slot 1, Reverb in slot 2, Filter in slot 3, all at zero wet. The existing master
Echo/Reverb response is preserved. The third control now processes audio too.
Changing type resets the newly selected history; it does not restore an old
frozen tail. Other slots' histories are untouched. Sample-rate reconstruction
prepares all types at the new rate while preserving type and wet controls.

Delay reset is constant-time: reset the write cursor and valid-written count;
mask unwritten interpolation neighbors until the first complete wrap. Reverb
resets its fixed four delays this way. No delay allocation, deallocation or
sample-rate-sized buffer clear occurs on selection. Filter reset clears four
scalar histories. Unsupported types are absent from cycling; invalid slot
indices are rejected at command admission and cannot alias a valid slot.

Selected types are exposed as typed `fx_kind` snapshot values, `master_fx.types`
in IPC status, and visible Master FX selector buttons beside their Wet sliders.
The GUI buttons use the same FxSelect command as the mapped controller events.

Validation (local Linux, private Cargo target):

- `cargo test --offline`: **277 passed**.
- `cargo build --offline`: passed.
- `git diff --check`: passed.
- Synthetic legacy Numark NS7FX mapped selector/wet messages exercise every slot
  and all three types at 44.1/48/96 kHz. Each full-engine output is compared
  sample-by-sample with independent raw mono processor references; selected
  type snapshots must agree. Note-off/zero-velocity messages do not cycle types.
- Same-type slots receive distinct impulses and match independent raw histories;
  empty opposite channels stay exactly silent. Existing #13 stereo impulse,
  timing, dual-mono and sine references, and #21 rate-duration references pass.
- Delay reset equals newly zeroed storage with fractional times 1.25, 1.5, 7.75
  and near the capacity boundary, capacities 64/511/4800, feedback 0.93, four
  complete wraps, and clones made before and after the first wrap. This covers
  interpolation across the unwritten/written boundary and long feedback tails.
- Counting-allocator checks show zero allocations/frees for history reset and
  90 actual FxSelect applications. Reverb/filter reset leaves exact silent tails.
- Headless egui output paints all selected types, and actual pointer events on
  a selector enqueue the corresponding FxSelect command.

The mapping evidence is synthetic for the existing legacy NS7FX profile. It does
not qualify a physical Numark NS7 MkII, Pioneer controller, or other hardware;
those remain part of the user's final hardware QA. This change does not claim
click-free type transitions or change the separate configurable FX rack mixes.
