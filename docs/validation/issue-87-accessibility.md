# Issue 87 — keyboard and Linux accessibility

The custom-painted performance surface now publishes names, roles, values,
states and focus through AccessKit. Numeric edits, button activation and named
alternate actions use the existing GUI command path. Native sliders retain
pointer geometry and native inline value editing; the shared F2 editor adds a
consistent finite-range Apply/Cancel path.

## Coverage

| Surface | Keyboard and semantic route |
| --- | --- |
| Decks | Separate A/B names; pitch in percent, EQ/gain in percent, waveform seek in seconds; cue set/delete, play/pause, cue, unload, loop controls, match and jog alternatives |
| Sampler | Named bank/instrument selectors; physical pad identity and piano note; Space/Enter holds, assistive toggle or explicit Press/Release; inactive gaps refuse presses |
| Sequencer | Named scene/track/cell controls; launch once/loop, add/restart scene, compose arm, clip gain, track mute/solo/effects/select |
| Effects | Slot and track/scene context, enabled state, physical parameter names/ranges; master slot type and wet percent |
| Crate | Named search, visible rows with metadata/time description, complete one-based selection range, load-target controls and load actions |
| Editors/status | Named diagnostic path, load Retry/Dismiss per deck, clip gain zero/unity, ordinary native dialog buttons and value fields |
| Overflow | Named horizontal/vertical scrollbars with pixel values, arrow/Home/End/F2 operation; focused controls scroll into view |

Tab/Shift+Tab traversal has a visible focus stroke on custom controls. Shift+F10
provides every pointer-modifier/right-click alternative. The top **Actions for
…** menu offers the same choices through ordinary native Click buttons. This
route is necessary: the shipped AccessKit Unix adapter exposes Click but does
not export its custom-action metadata or ShowContextMenu through AT-SPI.

The action target retains a control identity and retires when that control is
removed/disabled. Numeric results retain their original control identity;
project integration clears these transient editors before replacing the graph.
Pointer, Space, Enter and assistive sampler holds release independently, with
one accepted engine gate per pad. Losing window focus releases all local holds.

## Validation

The real egui/renderer tests in `src/ui/accessibility/tests.rs` cover semantics,
actual focus traversal and small-window scrolling, value actions, fine steps,
F2 invalid/apply/cancel and focus restoration, named cue/solo/compose/gain
alternatives, all 16 pad identities in every instrument mode, mode-change
release, window loss, rejected admission, keyboard compose/deck preparation,
and a 50,000-row crate without full-list rendering or refiltering.

The native fixture uses actual `App::update_frame` output through AccessKit Unix
and queries/actions through Python GI AT-SPI. It runs on newly created private
session/accessibility buses, with private XDG directories and memory-only
settings. It verifies names/roles, focus, pitch SetValue, platter Click, hot-cue
set/delete through the ordinary menu, and sample/synth Press/Release reaching
the actual renderer. Its native project workflow creates an empty project,
arms composition, records and releases a pad note, undoes/redoes that note,
saves, creates another empty project, and reopens the saved note through Recent.
A separate actual-egui keyboard fixture types a custom absolute path containing
a space and exercises Save/Open, composition and undo. Project replacement also
retires every GUI pad admission owner before clearing transient input/editor
state, so a later release cannot leak reserved queue capacity. No accessibility tree transform or synthetic substitute
for the platform bridge is used.

Run after building the test executable:

```sh
cargo test --no-run
python3 scripts/check-accessibility.py --test-binary /path/to/debug/deps/omatainer-HASH
```

The harness requires Linux, `dbus-run-session`, at-spi2-core, Python GI and the
Atspi/Gio typelibs. The ignored Rust child refuses direct use without the private
harness guard. It opens no window and never changes the desktop's accessibility
settings. Human assistive-tool usability, Orca workflows and physical-controller
qualification are not established by these automated tests. These project/undo
workflows run against the prepared integration containing issues 82–86; the
complete final assembled stack passes the same native fixture.


Native validation on Linux aarch64 used egui 0.32.3, AccessKit 0.19.0,
accesskit_unix 0.15.0, accesskit_atspi_common 0.12.0, system at-spi2-core 2.60.7,
and Python 3.14.7. The expanded successful run traversed 227 native nodes and
returned 45 native actions through the real adapter, with one saved note
verified after reopening. Deliberate child failure and SIGTERM cancellation
exited within the harness deadlines and left the pre-existing desktop bus/registry
process inventory unchanged. Frame counts are diagnostic, not a performance claim.
The live-tree traversal tolerates a child disappearing during enumeration while
still requiring each named control and state transition within a fixed deadline.

The original isolated implementation passed 454 ordinary Rust tests and a
production build. The prepared project integration passes 183 GUI tests with
one native child fixture ignored by the ordinary runner; that child passes when
invoked through the private native harness. The shortcut window has bounded,
scrollable geometry so its expanded content does not cover the project menu.
The final assembled full suite passes 588 tests with 7 opt-in fixtures ignored;
`cargo build --offline` passes. The native fixture passes again on that exact
stack, traversing 227 nodes and returning 45 native actions. The saved/reopened
note remains present. License records were refreshed for the new source and
feature graph, and packaging verifies the built executable's embedded records.

A later real workflow regression fixes percentage rounding in the clip-gain
editor: setting 53% and rendering twenty idle frames now creates exactly one
history entry. One Undo restores 100%, and Redo restores 53%. Idle decimal
normalization cannot enqueue the unchanged gain repeatedly.
