# omatainer

Omarchy-native DAW + live DJ surface. One clock, one mixer, one window:

- **Session** clip grid (Ableton-style launcher)
- **Arrange** timeline
- **Compose** piano roll
- **Two decks** with spinning platters, Serato-style waveforms, hot cues, loops, vinyl jog, sync, EQ, filter, and a crossfader

Hardware is optional. USB class-compliant MIDI is first-class. Factory maps ship for Akai APC Mini / APC40 / MPK, the original Numark NS7 and NS7FX, and Pioneer DDJ-FX / FLX / SB / 400. Anything else is MIDI-learnable and still plays from the keyboard.

It reads the current Omarchy theme (`~/.local/state/omarchy/current/theme/colors.toml`) and font, registers in the keybind menu, and drops a Quickshell bar chip next to the rest of the shell.

## Install on this machine

```bash
~/Projects/omatainer/scripts/install-omarchy.sh
```

Then `SUPER+O` launches or focuses it. Right-click the bar chip to play/stop without opening the window.

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

MIDI clock in/out and live notes from any class-compliant USB device hit the selected track. APC grids launch clips. Pioneer / Numark jog wheels scratch the platters.

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

```bash
omatainer ctl status
omatainer ctl togglePlay
omarchy-shell -q omatainer togglePlay
```
