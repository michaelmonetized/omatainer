# Named ordered crate qualification

The crate manager creates, renames, nests, orders and deletes collections of
stable catalog TrackIds. One track may belong to several crates; direct members
retain manual order and children remain separate views. Copy/move operations
change memberships only. Deleting a subtree requires a confirmation with its
crate and membership counts, and preserves catalog tracks and source audio.
All tracks retains the existing sort; named-crate filtering retains manual order.

The existing single metadata owner validates revision-qualified edits and
persists the candidate catalog. Its one outstanding request includes the terminal
receipt. Cancellation before publication rejects the edit; afterward the actual
durable, committed-unconfirmed or unknown result is shown. Bounded admission,
performance protection and close coordination reuse the established owner.
Catalog schema 6 migrates schemas 1–5 to an empty forest while retaining track
IDs, preparation and analysis. Imports commit tracks and forest together or
reject conflicting identities/order without partial application.

The owner builds immutable TrackId-to-row/track lookup tables alongside the exact
catalog and row publications. Different Arc identities cannot reuse an old map.
Previous table destruction occurs on the metadata worker. The GUI virtualizes
tree and member rows, and stores at most 4,096 selected members in manual order;
steady frames do not scan the entire membership list to rebuild that selection.
Controller loads keep admission-time source identity across later view changes.
Projects retain only a selected crate ID; they cannot replace the global forest.

Earlier [model](issue-105-crate-model.md) and
[catalog-owner](issue-105-catalog-foundation.md) records describe their focused
foundation results and integration bases. Their pending UI statements concern
those earlier layers, not a claim that pure model tests complete this feature.

## Integration checks

Two new lookup tests passed: exact row/catalog/version pairing, including saved
labels for unavailable rows, and actual worker publication plus off-GUI retirement
of the previous table. The initial integrated library filter passed 72 cases but
failed the new manager test because the text input lacked an explicit accessible
name. The field now publishes its own name. The next manager attempt exposed an
incorrect test shortcut modifier; it now uses the Linux command/Control event
semantics, rather than concatenating fixture names.

Three real-App manager groups then passed in 0.78 seconds:

- Native egui actions create nested overlapping crates, copy and manually reorder
  members, rename, cancel deletion, reopen the actual store and verify order.
  A controller Browse/Load burst captures the selected manual-order file before
  the GUI switches views, and the actual decoder loads that captured source.
  Original FLAC bytes and fingerprints remain unchanged.
- Previously exposed membership-removal and subtree-confirmation actions cannot
  target newly selected members or a different crate. Fresh actions apply to the
  intended identities; performance protection rejects rename without changing it.
- A persisted 4,096-crate tree with a 10,000-member selected crate reuses filtered
  indices and tree construction across 20 steady frames. Only visible member
  rows are exposed/rendered, not the whole membership list.

The first separate native AT-SPI attempt stopped because the visible name label
and input shared an accessible label and the harness found the static label.
The input remains named Crate name; its short visible label is now Name.
The ordinary New crate draft supports native creation without fixture-injected
collection mutations. Typed names/rename are covered by real egui key/text
events; the pinned native adapter exposes text/focus without EditableText
mutation. The second and third native attempts exposed a harness ordering error: an
earlier Focus acknowledgement could satisfy the later Create click wait. The
harness now waits for the exact requested action and confirms text focus. The
corrected native workflow passed 150 actions, 243 visited nodes and 1,447 App
frames, including root/child creation, overlapping copy, manual reorder,
cancelled deletion and deletion of the child alone. The existing 160-action
capacity and 70-second timeout were unchanged. These failed attempts remain in
`issue-105-native-v1.log` through `v3.log`; the pass is `v4.log` under the local
work evidence directory.

On the assembled tree based on final #104 (`77163b9`), all 969 ordinary tests
passed with 20 opt-in/maintainer entries ignored in 89.02 seconds (two threads).
This includes selected-child deletion falling back to the correct parent/name,
missing saved project view falling back without recreating a forest, and a
catalog refresh preserving the captured viewport anchor. Manual regeneration
and source inventory checks passed. Only a trailing blank line was removed
after that suite; the final release build binds the refreshed inventory.

## Final release qualification

The source-bound release gate passed on 2026-10-01 from
12:39:23.400195 to 12:43:28.807809 UTC, with all eight workload groups
passing three repetitions under the unchanged performance policy. Its fresh
native workflow passed 150 actions, 243 visited nodes and
1319 App frames, including the named-crate workflow. The native action
capacity remained 160 and the deadline remained 70 seconds.

Worst repeated callback wall p99 / maximum in milliseconds were producer
0.783628 / 1.392088, composer 0.830711 / 0.959128, live DJ
0.097417 / 0.211418 and hybrid 0.866211 / 1.004253. All four measured
callback groups had zero callback-wall or render-CPU deadline exceedances,
allocations, frees and rejected commands. Measured MIDI drops were zero.
Large-crate frame p99 was 3.088635 ms; multi-controller frame p99 was
5.389018 ms, IPC round-trip p99 8.606696 ms and MIDI dispatch p99
3.196970 ms. These are local software workloads, not physical-device latency.

Independent evidence verification, package creation and package verification
passed on the unchanged release executable. All seven CLI protocol cases,
six follow-protocol groups, five runtime-isolation groups and safe-startup
checks passed. Local evidence is preserved as `issue-105-final-performance.*`
and `issue-105-final-package` in the work evidence directory.

SHA-256 bindings:

- Production executable: `94706c9142d1c6ce2d189c151242c601f74e95fafa2507a54ddcc7af401d830e`.
- Release test executable: `c206cba833142c821ed56099415c75da36f3cab457c947ec3d3cc4a8c5210a43`.
- Source inventory: `76f1854ca87d24b7bf6262ace1d3e02b7fac55066cc34b7dfd0b652425b83ac9`.
- Final performance report: `750e9d926e5dabbdb070096429c38ee10db90218b0caf9c0176e05ce0aeb1675`.

No physical controller, human screen-reader, listening or hardware storage
result is inferred from these software fixtures. Final producer, composer and
live-DJ hardware acceptance remains with the user.
