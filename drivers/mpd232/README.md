# MPD232 preset reader

The original Akai MPD232 Editor 1.0.8 uses the device's **Remote** MIDI port to read presets. This utility implements that read operation on Linux. It never sends a preset write, firmware command, reset, or preset selection.

```sh
cc -Wall -Wextra -Werror -O2 drivers/mpd232/read-preset.c -lasound -o target/hardware/mpd232-read-preset
amidi -l
target/hardware/mpd232-read-preset hw:5,0,3 1 target/hardware/mpd232-current.syx
```

Use the card number reported by `amidi -l`; `hw:5,0,3` is this rig's current Remote port. The output path must not exist. The reader bounds both packet size and wait time and validates the complete response before saving it.

Save the qualified preset as `midi/mpd232.syx` alongside Omatainer's `preferences.json`. Omatainer reads its actual control assignments at startup. Without a saved preset it keeps the programmable MPD profile and forwards musical notes. Preset files with overlapping addresses, pad/switch collisions, unsupported encodings or shortened mixer ranges are rejected explicitly.

In the current LiveLite preset, each control bank's eight faders control the eight track levels. Knobs control pan in A, reverb send in B, and echo send in C. Switches control mute in A, solo in B, and record arm in C. All four pad banks retain their MIDI channel, note, velocity and release. Note Repeat, Time Division, pad banks, control banks and the step sequencer run on the hardware and send their resulting notes to Omatainer. Standard Start, Continue, Stop, MMC and fixed Akai CC transport reach the engine. CC 117 stops, 118 starts and 119 arms recording on the Common channel; releases do not toggle state. These CC addresses are reserved and conflicting mixer assignments are rejected. Stop remains effective when the input queue is full.

The fixed CC addresses are documented in Akai's [MPD/MPK transport guide](https://cdn.inmusicbrands.com/akai/mpk49/mpd_and_mpk_series___transport_controls_midi_details_03.pdf_2054aa736efe9863884238e0407292e8.pdf). That guide covers the older MPD32/MPK49; applying its family addresses to the MPD232 is a software compatibility path. The connected MPD232's transport capture is still pending, separate from the captured bank-A mixer controls.

The read command is `F0 47 00 36 12 00 01 <preset-1> F7`. Akai's editor request path at `0x100017a30` calls the packet builder at `0x10001620a` with operation `0x12`; the builder supplies the Akai manufacturer byte and MPD232 model `0x36`, length `00 01`, and zero-based preset index. Its MIDI wrapper at `0x100052702` adds `F0`/`F7`. The response contains operation `0x10`, 3,475 payload bytes, and the requested preset index. The parser's control records are verified against a read from the connected device, retained as `tests/fixtures/mpd232-livelite.syx`.
