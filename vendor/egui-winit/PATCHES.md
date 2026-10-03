# Independent physical mouse coordinates

Pinned egui-winit 0.32.3, with its original sources and MIT/Apache notices retained.
UPSTREAM.json binds the archive and original file hashes. Only src/lib.rs changes.

Keep the last physical CursorMoved position separately from touch emulation.
Physical MouseInput uses those coordinates even while touch is active. Touch
end/cancel retains the physical position after emitting its ordinary PointerGone,
so a stationary physical release is still forwarded. CursorLeft removes hover and refuses new presses, but retains the last physical
coordinates for release.
The adapter tests cover stationary release after end/cancel and stationary press
during touch. No pressure driver or new platform input backend is added.
