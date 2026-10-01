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
the actual renderer. No accessibility tree transform or synthetic substitute
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
qualification are not established by these automated tests. Project/undo
workflows are validated again when this delta is assembled above issues 82/83;
this implementation branch starts from issue 81.


Native validation on Linux aarch64 used egui 0.32.3, AccessKit 0.19.0,
accesskit_unix 0.15.0, accesskit_atspi_common 0.12.0, system at-spi2-core 2.60.7,
and Python 3.14.7. The successful run traversed 219 native nodes and returned
21 native actions through the real adapter. Deliberate child failure and SIGTERM
cancellation exited within the harness deadlines and left the pre-existing
desktop bus/registry process inventory unchanged. Frame counts are diagnostic,
not a performance claim.


Final local checks: `cargo test -- --test-threads=1` passed 454 tests with five
opt-in fixtures ignored. A subsequently added actual scrollbar/scroll-view
regression passed separately (no production-source change). The native harness
passed again against the same implementation. `cargo build` passed. New Rust modules pass rustfmt
and the complete delta passes `git diff --check`.
