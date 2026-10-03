# Touch and pen input

Hold multiple sampler pads, both deck pitch faders and the crossfader together.
Each contact owns the control it pressed until it lifts or cancels. Moving off a
pad keeps it held. Multiple contacts can share a pad; its last owner releases it.
A fader belongs to its first contact until release. Contact identity includes the
input device, so equal contact numbers from different devices stay independent.

Pads and faders claim their drag. Touches outside them retain ordinary platform
scrolling and single-pointer controls. Preferences → UI scale enlarges the existing
hit targets. Mouse dragging, focused Space/Enter, numeric keyboard entry and
assistive actions remain available. Mouse, keyboard and touch pad owners release only
their own hold. A touch does not block mouse presses or fader drags on other controls. Setup → Touch & pen explains these gestures.

Opening a blocking editor or menu, losing window focus, changing target geometry
or DPI, project replacement and safety recovery release touch holds. Moves from
cancelled contacts never restart them: press again. A rejected command creates no
held pad owner. Short taps retain press/release order, including when layout needs
another pass. Up to 32 contacts and 256 touch events per frame are accepted; an
oversized event batch cancels current holds rather than leaving a partial gesture.

## Pressure and platform limits

Reported normalized pressure scales the pad's initial sample gain or synth attack,
and recorded MIDI velocity (the previous full attack records 110). Fader position
ignores pressure. Pressure changes after attack do not retrigger notes. Invalid
position or pressure cancels that contact; end/cancel still releases it even when
the final position is invalid. Missing pressure uses the previous full attack.

The pinned egui-winit 0.32.3 adapter forwards device/contact identity, start/move/
end/cancel and reported winit force, while emulating a mouse for the first touch.
Performance controls separate the adapter's adjacent emulated pointer messages
from physical mouse input to prevent duplicate attacks. Physical mouse button
ownership survives a touch start, move, end or cancellation.
The pinned winit 0.30.13 Linux Wayland and X11 touch handlers explicitly emit
`force: None`; pressure is unavailable on these backends. A pen presented as an
ordinary mouse retains mouse controls. This change does not add a tablet driver.
Runtime contacts are never saved in preferences or projects. Saved UI scale and
keyboard bindings retain their existing preference persistence.

Linux aarch64 native App/renderer fixtures cover simultaneous holds, independent
faders, event order, scrolling ownership, focus/DPI/editor/recovery cancellation,
invalid input, queue saturation, pressure and recording. They run without an OS
window. No touch or pen device is connected to this machine; physical multi-touch,
pen, device removal and listening checks remain pending. A driver disappearance
that supplies neither cancellation nor focus loss cannot be established here.
See the frozen qualification receipt in `docs/validation/issue-127-touch-pen.md`.
