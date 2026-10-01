# omatainer

Omarchy-native DAW + live DJ surface. One clock, one mixer, one window:

- **Session** clip grid (Ableton-style launcher)
- **Arrange** timeline
- **Compose** piano roll
- **Two decks** with spinning platters, Serato-style waveforms, hot cues, loops, vinyl jog, sync, EQ, filter, and a crossfader

Hardware is optional. USB class-compliant MIDI is first-class. Factory maps ship for Akai APC Mini / APC40 / MPK and Pioneer DDJ-FX / FLX / SB / 400, with legacy partial NS7 / NS7FX control maps. These profiles do not establish physical compatibility; NS7 motorized wheels and NS7II support remain unverified. Anything else is MIDI-learnable and still plays from the keyboard.

It reads the current Omarchy theme (`~/.local/state/omarchy/current/theme/colors.toml`) and font, registers in the keybind menu, and drops a Quickshell bar chip next to the rest of the shell.

## Install on this machine

```bash
~/Projects/omatainer/scripts/install-omarchy.sh
```

Then `SUPER+O` launches or focuses it. Right-click the bar chip to play/stop without opening the window.

The installer stages and validates the complete integration before publishing it.
Failed publication or desktop validation restores the prior files. Each successful
install prints a recovery journal under `~/.local/state/omatainer/installations`;
restore its saved state with `scripts/install-omarchy.sh --recover /path/to/journal.json`.
Running app instances retain their executable until you close and relaunch them.

## Play without files

The default session is a four-clip house sketch (drums, bass, keys, pad). Press **space**. Scenes **1** and **2** are filled.

Drop wav/mp3/flac onto a platter, or put tracks in `~/Music` and load with **F** / **→ A** / **→ B**.

## Native projects

Use **Project → Save project as…** to choose a `.omat` path. Native projects embed
the session's media alongside clips/notes, instruments, mixer/effects, sampler
banks, deck cues/loops/positions, selections and the crate/panel view. The current
factory controller mapping schema is recorded; there is no editable mapping
configuration to save in this build.

**Save project** updates the current file. **Save project as…** changes the current
path; **Save copy…** writes another file while keeping the current path and unsaved
state. Path dialogs accept absolute paths or paths relative to the application's
working directory. Replacing an existing destination in As/Copy requires the
explicit checkbox. The title marks unsaved edits; changes made during a save stay
unsaved after that captured version finishes writing.

**New project** creates an empty clip session with the factory sampler resources.
**Open project…** and **Recent projects** restore a saved session with playback
stopped. Press **Space** to resume remembered session clips; each deck's play
button resumes its saved position. New, Open and window close offer
**Save changes / Discard changes / Cancel** when needed. File operations run in a
worker and expose cancellation and visible errors; a failed open preserves the
current session. An unresponsive engine cannot silently authorize a clean close;
explicit Discard can still exit without saving.

The versioned file includes exact decoded audio, so original source files are not
required for playback after reopening. Physical held keys, scratch touches,
connections and DSP tails are transient. Default limits are 8 MiB metadata,
256 embedded media entries and 1 GiB PCM; unsupported or corrupt projects are
rejected without replacement. See the [project workflow evidence](docs/validation/issue-82-ui.md)
and [file format and atomic save rules](docs/validation/issue-82-codec.md).
## DJ library

Tracks discovered by Scan or successfully loaded from a dropped file are stored
in `$XDG_DATA_HOME/omatainer/library.json` (normally
`~/.local/share/omatainer/library.json`). Stable track IDs, metadata, tempo
provenance, duration, playback history, cues and loop preparation survive restart.
This catalog is independent of DAW projects. Its saving/error status appears
below the crate controls; row tooltips include track ID and typed location.

**library… → Import catalog** imports an Omatainer catalog JSON on the background
worker. Version 2 is the current format; the documented flat version 1 layout
migrates without changing IDs or preparation. Conflicting identities or unknown
fields/formats are rejected rather than discarded. Removable-volume and provider
references stay distinct, but loading them is explicitly unavailable until a
resolver is implemented. No provider/network media is fetched.

The main cue, eight hot cues and saved loop range/arming state return on a later
load; playback stays paused. Replaced file bytes keep their location's ID but get
fresh preparation, with old fingerprint versions preserved in the catalog. A
moved path is a new location. The current engine has no editable beat-grid model;
only its implemented tempo analysis and cue/loop preparation are persisted.

Saves publish atomically and retain `library.backup.json`. A malformed or newer
store is never reset to an empty library. Repair/restore it while the app is
closed, then restart; errors remain visible. Normal close waits without blocking
the UI for earlier deck edits and the background save. **Close without saving**
explicitly accepts any uncommitted changes being lost. See the
[storage schema, recovery policy and validation](docs/validation/issue-85-dj-library.md).

## Keybinds (also under `?`)

Omarchy desktop bindings: `SUPER+O` launches/focuses the app;
`SUPER+SHIFT+SPACE` controls session playback while unfocused.

<!-- app-shortcuts:start -->
| Keys | Action |
| --- | --- |
| `Space` | Play / stop session |
| `1` | Launch scene 1 |
| `2` | Launch scene 2 |
| `3` | Launch scene 3 |
| `4` | Launch scene 4 |
| `5` | Launch scene 5 |
| `6` | Launch scene 6 |
| `7` | Launch scene 7 |
| `8` | Launch scene 8 |
| `Q` | Play / pause deck A |
| `A` | Cue deck A |
| `W` | Toggle deck A tempo sync |
| `P` | Play / pause deck B |
| `L` | Cue deck B |
| `O` | Toggle deck B tempo sync |
| `[` | Crossfader fully to A |
| `]` | Crossfader fully to B |
| `F` | Load selected crate item onto selected deck |
| `? (Shift+/)` | Show / hide shortcut help |
| `F1` | Show / hide shortcut help |
| `Ctrl+M` | Show / hide MIDI window |
| `Escape` | Close effect chain |
| `Ctrl+Z` | Undo last creative edit |
| `Ctrl+Shift+Z` | Redo next creative edit |
| `Ctrl+Y` | Redo next creative edit |
<!-- app-shortcuts:end -->

The bracket keys choose the crossfader endpoints, with the existing mixer ramp.
F uses the current filtered crate selection and selected deck. Tab navigation
between session/arrange/compose views is separate feature work.

Typing in search or another text field owns the keyboard, including the frame
that editing ends. Blocking dialogs and popups also suppress global shortcuts.
Outside those contexts, letter/space shortcuts require no modifiers; `?` uses
Shift+Slash, and Ctrl+M opens the MIDI window.

## Keyboard and assistive controls

Tab and Shift+Tab traverse controls; focused custom controls show an outline and
scroll into view. Space or Enter activates the focused button. A focused pad
holds its note until that key is released or focus leaves; Space on a focused
control does not also change the session transport.

Arrow keys adjust a focused numeric control in its displayed units. Shift makes
fine adjustments, Home/End select the limits, and F2 opens a direct value editor
with Apply/Cancel. Invalid values stay in the editor for correction. The crate's
**Crate selection** control supports arrows, Page Up/Down, Home/End and direct
one-based row numbers through F2, including rows outside the visible viewport.
Enter loads its selected row onto the selected deck.

Shift+F10 opens the focused control's alternatives: cue deletion, loop out,
clip launch mode, compose arm, gain editing, mute/solo, and pad Press/Release.
The **Actions for …** button exposes the same menu to assistive tools through
ordinary buttons. Focus a control with alternate actions first to choose that menu's target. Assistive
click on a pad toggles a hold; **Release pad** explicitly ends it. Window focus
loss releases local pad holds.

Linux accessibility uses the existing eframe/AccessKit AT-SPI bridge. Names
include deck, track, scene and effect context; values and states are exposed in
the native tree. The automated private-bus fixture verifies real AT-SPI queries
and actions; Orca and human workflow qualification remain to be performed.
See [accessibility validation](docs/validation/issue-87-accessibility.md).

MIDI clock in/out and live notes from any class-compliant USB device hit the selected track. APC grids launch clips. Pioneer relative jog bindings decode forward and reverse movement using their documented centered value. NS7 wheel input is deliberately unmapped: its absolute-position protocol cannot use the old guessed relative-CC/pitch-bend bindings. See the [jog decoder evidence and hardware limits](docs/validation/issue-42-relative-jog.md).

The APC40 original and mkII use separate protocol-based input profiles for eight
track faders, the master fader, clip grid and five scene buttons. Other APC40
buttons are ignored; record-arm and track-select do not operate unrelated
controls. Device-name selection and synthetic MIDI tests do not establish
physical compatibility. Controller QA remains pending; see the
[APC40 mapping evidence](docs/validation/issue-38-apc40-profiles.md).

Ctrl+M shows each MIDI port's discovery/connection state and failure reason.
**Retry / rescan MIDI** checks current ports in a background worker and retries
failed connections without reopening working ones. Keyboard and mouse remain
available throughout. A disconnected device is detected on an explicit rescan
or when its input worker ends; automatic hotplug detection is not implemented.
See the [connection lifecycle validation](docs/validation/issue-73-midi-connections.md).

MIDI Start and Stop use the transport handlers. Received MIDI Clock ticks expose
their accepted count and last source in `midi_clock` status; this reception hook
does not synchronize tempo or phase. See the [MIDI framing validation](docs/validation/issue-41-midi-realtime.md).

The sampler offers **samples**, **analog**, **keys**, and **pad**. Samples use the
Kit/Perc/Hits banks. Analog is a fast saw/square bass, Keys blend saw and sine with
a medium release, and Pad blends sine with a detuned saw and a longer envelope.
Changing the selection affects new presses; held notes keep their original sound.
Sample pads **1–8** occupy the bottom row and **9–16** the top row, preserving the
piano layout's natural/accidental positions. Number N triggers bank slot N−1.
Hover a pad for its slot or piano MIDI note; blank piano positions are inactive.

Theme colors, shell base font size, and fontconfig's current monospace font
reload in a background worker, including font-only edits. The next valid update
replaces the applied style; incomplete writes retain the last valid settings.
See the [theme reload validation](docs/validation/issue-78-theme-reload.md).

The installed face resolved by `fc-match monospace` is used first for both UI
text families. Fontconfig may supply an installed substitute for an unavailable
family. If resolution or font-file validation fails, Omatainer keeps the last
valid font, or uses its bundled Ubuntu/Hack/emoji fallback before the first valid
selection. Bundled symbol fallbacks remain available alongside the chosen font.
See the [selected-font and glyph validation](docs/validation/issue-79-selected-font.md).

## Layout

```
transport · bpm · scene · midi
browser | session / arrange / compose | deck A platter + waveform
        | mixer + xfader              | deck B platter + waveform
```

Not Ableton plus Serato with a bridge — one document. Decks are extra mixer buses that stay live while clips launch.

## Paths

| | |
| --- | --- |
| Binary | `~/.local/bin/omatainer` |
| Control socket | `$XDG_RUNTIME_DIR/omatainer.sock` |
| Config | `~/.config/omatainer/` |
| Plugin | `~/.config/omarchy/plugins/omatainer/` |
| Theme | `~/.local/state/omarchy/current/theme/colors.toml` |

The runtime directory must belong to the effective user, have mode `0700`, and
have trusted directory ancestors without symlinks. If `XDG_RUNTIME_DIR` is unset,
the app and CLI use `${TMPDIR:-/tmp}/omatainer-<effective-UID>/omatainer.sock` in a
private `0700` directory and print a warning. Invalid directories or endpoint
collisions fail without changing their owners, permissions, or contents. The old
shared `/tmp/omatainer.sock` endpoint is never used or removed.

```bash
omatainer ctl status
omatainer ctl togglePlay
omatainer ctl scene 1 # scene numbers are 1 through 8
omarchy-shell -q omatainer togglePlay
omarchy-shell omatainer scene 3 # shell scenes are also 1 through 8
```

`ctl scene` requires exactly one integer from 1 through 8. The control socket's
JSON scene operation uses zero-based indexes instead: `{"op":"scene","n":0}`
launches scene 1, and `n` must be an integer from 0 through 7. Invalid scene
requests return `{"ok":false,"error":"..."}` without changing the session.

The shell service's `scene(n)` and public `omatainer.scene` IPC method also take
one-based scene numbers 1 through 8. They validate before queuing, preserve the
number as a separate CLI argument, and report invalid requests as `rejected`
with `accepted: false` without launching a control process. For example,
`omarchy-shell omatainer scene 3` runs `omatainer ctl scene 3`, which sends
`{"op":"scene","n":2}`. Queued/result records include the captured `arguments`
array so multiple pending scene requests remain distinguishable.

Control commands return `accepted: true` and `command_status: "accepted"` when
queued for audio processing. The response's state fields are the latest published
snapshot and may precede execution; use `ctl status` or `ctl follow` for updates.
Full or disconnected queues return `ok: false`, `accepted: false`, and an error.
Status queries have null `accepted` and `command_status` fields.

`ctl follow` maintains one read-only status connection, with at most four updates
per second. It reconnects after a server restart with bounded backoff (up to four
seconds), and exits cleanly if its output consumer closes. Followers share the
eight-client IPC limit; normal command connections retain their 32-request cap.

Performance diagnostics are available from **Diagnostics** in the audio status
bar. The panel separates callback deadlines, render CPU, UI update wall time,
predicted output latency and queue pressure. Optional sampled track/device
costs include their coverage limits. Start a bounded 30-second capture, then
export a redacted private JSON file or reopen one for inspection. Exact backend
XRUN counts remain unavailable. See [diagnostic measurement and export
semantics](docs/validation/issue-81-diagnostics.md).

**Edit → Undo / Redo / History** reverses supported creative edits while keeping
transport and physical performance gates separate. Continuous drags form one
entry unless another source edits between them. History shows applied/redo
entries, memory use and explicit rejected edits; Ctrl+Z and Ctrl+Shift+Z (or
Ctrl+Y) use the same actions. Text fields keep their own Undo. Returning to saved
content restores a clean project indicator; New/Open start a new history.
See [history and save semantics](docs/validation/issue-83-history-ui.md).

### Content provenance and licenses

Open **Content & licenses** in the application for factory sound/preset IDs,
embedded-font terms, component sources, commercial-use/redistribution summaries
and complete offline notices. Original factory sounds are generated in code;
reference-product assets, proprietary SDKs, IR files and ML models are not
bundled. Imported media and fonts selected from your system retain their own
terms. See [the provenance records](licenses/manifest.json) and
[validation details](docs/validation/issue-86-content-license-manifest.md).

Maintainers refresh reviewed records with `python3 scripts/license-manifest.py
update`, then rebuild. `check` verifies the locked native source/component
inventory. Create a retained release with `package --binary PATH --destination
NEW_DIR`, and check it with `verify-package --destination DIR`. The installer
validates and retains the same records, including an executable hash receipt,
under `~/.local/share/omatainer/licenses` and its per-install recovery journal.
`omatainer licenses --manifest` and `omatainer licenses --notices` print the
records embedded in the executable without opening audio or the desktop.
