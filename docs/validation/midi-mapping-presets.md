# Portable MIDI mapping presets

In MIDI → MIDI mapping presets, select an exact connected destination port and
enter a distinct name. **Save port as preset** stores that port's learned
overrides in the active profile. Factory controls not listed in the preset retain
their existing behavior. Saving a definition does not change active assignments.

Select a saved preset to duplicate, rename, delete or export it. The optional
device revision hint is descriptive. Export writes a version 1 JSON document
containing the name, hints, factory-overlay layer and full action/wire metadata;
it carries no endpoint backend IDs. Existing destinations and symlinks are
refused. File work runs on the cancellable preference worker, with private atomic
publication and the same exact-revision save owner as Preferences.

Import reads and validates the entire document before showing its bindings.
Choose a distinct name and **Save imported preset to bank**. A conflicting name
requires a deliberate new name. Import never chooses a device or applies a
mapping. Unknown versions, fields, actions, relative formats, incompatible
message types and overlapping wire addresses refuse the whole definition.

**Review preset load** displays the exact device/backend port and replacement
bindings. **Apply reviewed MIDI preset** replaces only that port's learned layer.
Other port assignments and the saved bank remain intact. **Review factory
defaults** removes only that port's overrides. Both are session changes;
**Save MIDI assignments** retains them in the active profile. Applying a review
checks the exact config revision, unique connected endpoint and source owner
under the mapping lock. A reconnect, capture, profile change, target change or
intervening assignment edit requires a fresh review.

The bank permits 32 distinct names and 2048 total bindings per profile; each
preset permits 256 bindings and a 64 KiB document. The existing active config
limit remains 256 assignments and 64 KiB across all ports. Whole preferences
remain bounded to 1 MiB. Preferences version 18 migrates version 17 with an empty
bank, preserves existing Cue/jump mappings and custom shortcut owners, and
rejects preset fields under older document headers. Busy, unfinished or blocked
preferences cannot be overwritten by bank operations. Only the durable receipt
changes the saved bank; a file conflict preserves both prior settings and live
assignments.

Qualification uses native headless egui actions, real preference files and
synthetic MIDI dispatch workers with different machine port IDs. It checks
roundtrips for every implemented action, relative jog decoding, Cue release,
factory fallback, layer-change gate retirement, duplicate/rename/defaults,
unknown and ambiguous imports, exact reconnect/config refusal, cancelled reads
before work and after result delivery, closed/profile-changed editors, private
exports, existing files, symlinks and failed saves. Callback and renderer
allocation assertions remain zero. Physical capture and new listening tests are
paused. The current source-bound result is recorded in
[the qualification receipt](midi-mapping-presets-receipt.json).
