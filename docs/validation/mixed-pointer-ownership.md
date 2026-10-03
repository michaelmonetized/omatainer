# Physical mouse ownership during touch input

Source freeze: `08113b0e29189bd60520cfd590fdf16eb8752a63`. Base: PR #496.

A held touch previously suppressed every mouse pad press and pitch/crossfader
movement. Performance controls now read independent physical mouse state after
separating egui-winit 0.32.3's ordered, adjacent touch-emulation messages. Mouse
ownership survives emulated movement, release and cancellation; mouse and touch
can share a pad and release only their own hold. Ordinary widgets continue using
the platform pointer. The native event fixture now follows the pinned adapter's
actual Touch/PointerMoved/PointerButton ordering.

Locked Linux aarch64 qualification: all 11 native touch/input fixtures, 17 native
accessibility fixtures and 25 keyboard-matching fixtures pass (groups overlap).
Eight license/package fixtures pass. These include actual App controls and renderer
for mouse-first and touch-first pad holds, simultaneous fader drags, shared pads,
emulated release/cancel, focus loss, invalid contacts and queue rejection.
The preserved test executable is
`/home/michael/Projects/omatainer-work/mixed-pointer-final-qualified-tests`, SHA-256
`3f037c798840ff3e9a8b0970b88dfd055c08a74e89ef0d433fbf97d6c01ee77f`.
Adjacent final build, manifest, fixture and package logs retain the results.
A preliminary compile used a scalar coordinate where a position was needed;
it was corrected before qualification. No separate full-suite or optimized gate
is claimed for this correction; the next feature layer requalifies the stack.
Physical mouse/touch/pen and backend acceptance on supported hardware remain
pending as documented in the parent qualification. Native fixtures have no OS window.
