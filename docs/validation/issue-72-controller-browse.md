# Issue 72: controller browsing uses the visible crate

Browse and load now share the existing bounded GUI request queue. The MIDI
dispatch worker advances a cursor over the GUI's published filtered view, then
captures the exact immutable source for each subsequent load. A single MIDI
packet containing Browse followed by Load therefore loads the newly selected
row even when no GUI frame ran between those messages.

The GUI applies browse events in order, clamps movement to the filtered view,
and reveals the selected row. Its browse acknowledgments do not republish a
manual selection and reset a cursor already advanced by later queued events.
Manual selection/filter/view publications advance an epoch: older browse events
are ignored, while already accepted load requests retain their captured source.
This preserves issue 40's behavior when a user browses or rescans after pressing
load. The renderer's disconnected library_sel field was removed; a raw renderer
Browse command rejects through atomic diagnostics instead of changing isolated
state.

Admission is bounded to 16 pending GUI events and dispatch remains at most eight
per frame. A full queue rejects movement without advancing the shared cursor.
Empty, retired or unresolved views fail explicitly; invalid signed counts and
invalid deck targets also reject. Producers serialize the cursor and queue
operation with a short control-side mutex. The native MIDI callback still only
uses issue 39's fixed handoff; it never acquires this mutex, resolves a selection,
or allocates a request. GUI and MIDI worker code can allocate small source
captures. The adapter holds weak references to crate/index storage, so it does
not retain large old views indefinitely. Temporary upgrades can retain a view
briefly, and a concurrent GUI index rebuild can copy indices; no blanket claim
of constant-time or allocation-free GUI/worker behavior is made.

Browse mappings must explicitly declare a relative encoding and scale 1 (one
row per wire step). Signed counts and their magnitude are preserved, with
clamping at the ends; an encoder neutral report produces no action. Absolute
Browse mappings and fractional scaling are rejected by profile validation.
No factory profile currently declares Browse, and this change adds none. The
synthetic offset-binary binding used by tests is a fixture, not a hardware claim.

Validation exercises the actual MIDI handoff and worker, parser/dispatch,
control admission, App frames, filtered crate rendering, decoder worker and
renderer application. Cases cover a same-packet Browse/Load sequence and exact
loaded WAV identity; forward/reverse/neutral and filtered end bounds; scroll
reveal; multi-frame draining plus saturated admission; later manual filtering
without load retargeting; empty/retired/invalid views and unsupported mappings.
The raw callback also receives 1,000 browse/load packets while the GUI selection
lock is deliberately held: the counted callback performs zero allocations and
zero frees. Raw audio-side browse/load rejection retains its zero-allocation
check. Existing captured-load, scanner and crate virtualization tests remain in
the full suite.

Final validation: all 342 tests passed, `cargo build` passed, and
`git diff --check` passed. Independent read-only review found no blocker in
cursor ordering, queue rollback, epochs, weak-view lifetime or callback scope.
The headless fixture no longer replaces App's already-published initial crate
with an identical but unpublished allocation; production initialization was
already correct.
