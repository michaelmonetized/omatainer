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
<!-- app-shortcuts:end -->

The bracket keys choose the crossfader endpoints, with the existing mixer ramp.
F uses the current filtered crate selection and selected deck. Tab navigation
between session/arrange/compose views is separate feature work.

Typing in search or another text field owns the keyboard, including the frame
that editing ends. Blocking dialogs and popups also suppress global shortcuts.
Outside those contexts, letter/space shortcuts require no modifiers; `?` uses
Shift+Slash, and Ctrl+M opens the MIDI window.

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
