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

| Keys | Action |
| --- | --- |
| `SUPER+O` | Launch / focus (Hyprland, listed in the Omarchy keybind UI) |
| `SUPER+SHIFT+SPACE` | Play / stop even when unfocused |
| `SPACE` | Play / stop session |
| `1–8` | Launch scene |
| `Q A W` / `P L O` | Deck A / B play, cue, sync |
| `[ ]` | Crossfader |
| `TAB` | Session → arrange → compose |
| `?` | In-app cheat sheet |

MIDI clock in/out and live notes from any class-compliant USB device hit the selected track. APC grids launch clips. Pioneer relative jog bindings decode forward and reverse movement using their documented centered value. NS7 wheel input is deliberately unmapped: its absolute-position protocol cannot use the old guessed relative-CC/pitch-bend bindings. See the [jog decoder evidence and hardware limits](docs/validation/issue-42-relative-jog.md).

The APC40 original and mkII use separate protocol-based input profiles for eight
track faders, the master fader, clip grid and five scene buttons. Other APC40
buttons are ignored; record-arm and track-select do not operate unrelated
controls. Device-name selection and synthetic MIDI tests do not establish
physical compatibility. Controller QA remains pending; see the
[APC40 mapping evidence](docs/validation/issue-38-apc40-profiles.md).

MIDI Start and Stop use the transport handlers. Received MIDI Clock ticks expose
their accepted count and last source in `midi_clock` status; this reception hook
does not synchronize tempo or phase. See the [MIDI framing validation](docs/validation/issue-41-midi-realtime.md).

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
```

`ctl scene` requires exactly one integer from 1 through 8. The control socket's
JSON scene operation uses zero-based indexes instead: `{"op":"scene","n":0}`
launches scene 1, and `n` must be an integer from 0 through 7. Invalid scene
requests return `{"ok":false,"error":"..."}` without changing the session.

Control commands return `accepted: true` and `command_status: "accepted"` when
queued for audio processing. The response's state fields are the latest published
snapshot and may precede execution; use `ctl status` or `ctl follow` for updates.
Full or disconnected queues return `ok: false`, `accepted: false`, and an error.
Status queries have null `accepted` and `command_status` fields.
