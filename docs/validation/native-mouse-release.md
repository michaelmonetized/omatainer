# Native mouse coordinates during touch

Source freeze: `8c2da009a2b82a3c0c08bab2ffb46360af4cdaec`. Base: PR #497.

Upstream egui-winit clears its pointer coordinates after touch end/cancel, so a
stationary physical mouse release can disappear before reaching App input.
The pinned adapter now retains physical cursor coordinates independently,
uses them for physical MouseInput, and retains the last release position after
CursorLeft while refusing new outside-window presses. Touch still emits its
ordinary pointer release/gone sequence. The production native event branches
call the same helpers exercised by the adapter regressions.

The exact upstream archive and original hashes are retained in UPSTREAM.json.
Only src/lib.rs changes behavior. Original MIT/Apache notices and all modified
source are included in the provenance inventory and package; global registry
sources are untouched. PATCHES.md describes the small behavioral change.

Locked Linux aarch64 qualification: three adapter regressions, 11 actual touch
App/renderer fixtures, 17 native accessibility fixtures and eight package fixtures
pass. The adapter checks use State with no display handle, not an OS window.
The preserved App test executable is
`/home/michael/Projects/omatainer-work/native-mouse-final-qualified-tests`, SHA-256
`a77d80fc31b053bec4b3de40bfc9d753bcc18482151e351e0f1f987cd10098e0`.
The compiled artifact was selected from Cargo's JSON output. An initial copy
used the old dependency graph's filename and was caught before publication;
qualified touch/accessibility logs bind the corrected executable. A preliminary
package-specific feature flag was unsupported; the qualified adapter invocation
uses the locked production graph. Adjacent final/qualified logs retain evidence.
No separate full-suite or optimized gate is claimed for this correction; the
workspace layer requalifies the complete stack. Physical touch/pen/mouse and
monitor checks remain pending as described in the parent receipts.
