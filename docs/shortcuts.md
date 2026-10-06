# Commands and keyboard shortcuts

Setup → Commands opens the searchable command palette. Ctrl+Shift+P opens it
outside text fields and dialogs. A saved override on that chord takes precedence;
the palette then offers Ctrl+Shift+K or Ctrl+Shift+F3. Its Setup button displays
the current free chord. If all three are assigned, use the Setup menu with Tab.

Search matches command descriptions, editor/navigation/performance contexts and
current bindings. Up/Down selects a result; Enter runs it; Escape or Close commands
cancels. Typing and IME composition cannot dispatch musical shortcuts. Commands
use the same admission, performance protection, safe-mode and undo paths as their
ordinary controls. Explicitly running a command does not require its shortcut to
be enabled. The palette lists the existing bindable performance and editor actions;
project file commands retain their separate Project menu and reserved keys.

Preferences → Keyboard shortcuts edits the active draft profile. Type a key name
or use Capture key, then press the intended chord. Escape cancels capture. Reserved
navigation/project keys are refused, and current binding collisions are shown.
Reset all shortcut bindings restores defaults in the draft. Preview and Apply
save; Cancel preserves the applied profile. Text fields own typing keys; editor
commands and performance commands only dispatch outside text and blocking dialogs.

Key capture, dispatch and displayed names use egui's logical key, so Latin layouts
such as AZERTY follow the typed key rather than its physical position. egui-winit
0.32.3 falls back to the Latin physical key for unsupported non-Latin characters;
that fallback is shown as the supported egui key name. Capture does not promise
OS-wide shortcuts or interception of clipboard/IME keys handled by the platform.

Use the Preferences import/export path with Export bindings only to write a
version 2 JSON document for the edited profile. It contains only overrides and the
performance shortcut switch. Existing destinations are refused. Import bindings
into draft validates bounded JSON, unknown actions, reserved keys and collisions;
then it changes only that profile's draft bindings. Preview/Apply persists them.
All file work uses the existing cancellable preferences worker and atomic export
publication; device names, paths and other preferences are absent from the bundle.

Version 1 bundles remain readable. Preferences version 16 and shortcut bundle
version 2 add selected-deck beat jump actions. Migration preserves existing key
owners and disables any conflicting new default. Configure that jump action in
Preferences to choose another key. Older document headers cannot carry new jump
action IDs.

Hold the visible deck CUE button, focused Space/Enter or A/L to audition from pause. Release returns to the main cue; Play during the hold continues playback. Cue while playing stops and returns. MIDI Learn exposes Deck hold Cue audition separately from the existing one-shot Set/return Cue. Independent local and controller holds release separately. [Cue audition details](validation/cue-audition.md).
