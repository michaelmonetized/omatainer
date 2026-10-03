# Workspaces and panel windows

Open Setup → Workspaces or Preferences → Configure panel layout. Production,
Mix and DJ are included. Copy a layout under a name, choose its visible panels,
move panels up or down, and choose automatic or fixed heights. Fixed-height
panels scroll so their controls remain reachable. Names keep their exact Unicode
spelling. Keep at least one panel visible and no more than 32 named layouts.

Choose Separate window for Decks, Sampler, Library or Session and mixer.
Secondary windows share the same project, renderer, transport and controller
target; window focus does not select a different MIDI destination. Their fixed
header keeps session transport, emergency silence, protection/recovery state,
output errors and the current controller target visible. Keyboard and touch
ownership belongs to its window. Focus loss, DPI/monitor geometry changes,
closing or moving panels release GUI holds and require new presses.

Resize windows normally, then Use current window sizes to copy those sizes into
the preference draft. Preview changes and Apply and save persist the selected
layout and dimensions. Cancel changes retains the applied layout. Closing a
panel window or choosing Return panel to main window puts that panel back in the
main window for this session; it does not overwrite the saved layout.

Window positions follow the desktop. No absolute monitor coordinates are saved.
New windows are bounded by the main monitor's reported size; live windows clamp
to their current monitor, falling back to the main monitor when it is unavailable.
Content dimensions work even when Wayland cannot report a window position.
DPI uses logical points. A backend without native secondary viewports uses
resizable embedded windows with the same controls and headers.

Preferences version 11 stores these layouts per studio/performance profile;
versions 1–10 migrate to the default workspaces. Invalid names, missing/duplicate
panels, nonfinite/out-of-range dimensions and newer fields in older documents are
refused before application. Existing preference import/export and atomic
Preview/Apply retain cancellation and conflict behavior.

Linux aarch64 fixtures exercise actual preference controls, saved/reopened state,
embedded panels and native immediate-viewport callbacks with the actual App and
renderer. Native viewport fixtures use synthetic monitor/focus/DPI events without
OS windows. Physical multi-monitor removal and other desktop backends remain
unqualified. See the frozen validation receipt for measured results and limits.
