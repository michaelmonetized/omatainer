# Native mouse coordinates during touch

Source freeze: `bbbdf957671e47b66fb338ec69593b6521e3abcf`. Base: PR #497.

Upstream egui-winit clears its pointer coordinates after touch end/cancel, so a
stationary physical mouse release can disappear before reaching App input.
The pinned adapter now retains physical cursor coordinates independently,
uses them for physical MouseInput, and retains the last release position after
CursorLeft while refusing new outside-window presses. Touch still emits its
ordinary pointer release/gone sequence, followed by physical PointerMoved when
the mouse remains inside so hover and wheel targeting recover immediately. The production native event branches
call the same helpers exercised by the adapter regressions. Physical buttons
preserve an active touch location; touch release uses its native location even
after physical mouse movement. Physical releases outside the window finish with
PointerGone and leave no stale adapter position.

The exact upstream archive and original hashes are retained in UPSTREAM.json.
Only src/lib.rs changes behavior. Original MIT/Apache notices and all modified
source are included in the provenance inventory and package; global registry
sources are untouched. PATCHES.md describes the small behavioral change.

Locked Linux aarch64 qualification: five adapter regressions, 11 actual touch
App/renderer fixtures, 17 native accessibility fixtures, seven direct pad fixtures,
six runtime license fixtures and eight package fixtures pass. Direct pad fixtures
now collect input through the same begin/finish lifecycle as App; runtime license
checks account for all 11 retained native adapter files. Vendor source is collapsed
by GitHub for review. The adapter checks use State with no display handle, not an OS window.
The preserved App test executable is
`/home/michael/Projects/omatainer-work/native-mouse-location-qualified-tests`, SHA-256
`acb9e03db5430ba79e2d7b1201cb9d09c8676b42f92c4f53e39c246122ce3f2a`.
The compiled artifact was selected from Cargo's JSON output. An initial copy
used the old dependency graph's filename and was caught before publication;
qualified touch/accessibility logs bind the corrected executable. A preliminary
package-specific feature flag was unsupported; the qualified adapter invocation
uses the locked production graph. Adjacent location-qualified logs retain evidence. A misnamed accessibility
filter selected zero tests and was rejected; the corrected filter runs all 17.
No separate full-suite or optimized gate is claimed for this correction; the
workspace layer requalifies the complete stack. Physical touch/pen/mouse and
monitor checks remain pending as described in the parent receipts.
