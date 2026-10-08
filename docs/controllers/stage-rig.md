# Stage rig controls

The original NS7, Pioneer DDJ-SP1, APC40 mkII and MPD232 have dedicated native control paths in Omatainer. Keep the NS7 powered from mains. The MPD232 mapping below uses its current **LiveLite** preset, read through Akai's editor protocol without writing or selecting a preset.

## Pioneer DDJ-SP1

| Control | Omatainer action |
| --- | --- |
| Sync / Shift+Sync | Toggle deck Sync / turn Sync off |
| Slip / Shift+Slip | Slip timeline / Key Lock |
| Censor / Shift+Censor | Held Bleep / held Reverse |
| Deck selectors | Select the corresponding left/right deck |
| Hot Cue pads / Shift+pads | Set or hold eight hotcues / delete hotcues |
| Roll pads | Hold a beat-sized loop, then return to the running timeline |
| Slicer pads | Play one of eight grid slices, then return to the running timeline |
| Sampler / Velocity pads | Play the left or right eight sampler slots, with pressure in Velocity mode |
| Shift+Sampler pads | Stop the corresponding sample slot |
| Hot Loop pads / Shift+pads | Store or recall eight loops, starting from an existing hotcue when available / clear loop |
| Auto Loop pads | Select and toggle loop lengths; Parameter buttons change the range |
| Manual Loop pads | Deck select, loop active, save, slot up, loop in/edit, loop out/edit, reloop/exit, slot down |
| Auto Loop encoder / press | Half/double loop length / toggle loop |
| Shift+encoder / press | Shift loop / reloop |
| Parameter buttons | Roll/Auto Loop range, Slicer quantization, sampler banks, or half/double a manual/hot loop |
| Shift+Parameter buttons | Slicer domain, or loop shift |
| FX knobs / Shift+knobs | Independent high-resolution wet level / effect parameter |
| FX On / Shift+On | Toggle the slot / select Echo, Reverb or Filter |
| FX Beat encoder / press | Beat multiplier / reset multiplier |
| Shift+FX Beat encoder | Adjust bank parameters together |
| FX Tap | Tap deck tempo |
| FX assignments | Assign either bank to either deck |
| Sampler volume | High-resolution sampler bus volume |
| Browse / Shift+Browse | Browse tracks / crates |
| Browse press / Shift+press | Cycle native browser panels |
| Back / Prepare / Load | Return to crates / prepare the selected track / load the corresponding deck |
| Shift+Load | Return the corresponding deck to its start |

The hardware's deck 3/4 control layers address Omatainer's existing left/right pair. They do not create four independent audio decks. FX banks keep separate stereo histories for each deck. Mode buttons and Shift are managed locally by the controller; host feedback follows deck states, pad activity, sampler occupancy and effect assignments. Loop banks and these surface settings are runtime state.

## APC40 mkII

| Control | Omatainer action |
| --- | --- |
| 5×8 grid / scene buttons | Launch clips / scenes in the current session window |
| Shift+grid | Select the clip cell |
| Track faders / master / crossfader | Track levels / audience level / deck and assigned-track mix |
| Arm / Solo / Activator | Record arm / Solo / Mute |
| Track Select / Clip Stop / Stop All Clips | Select track / stop track / stop all clips |
| A/B buttons | Cycle no crossfader assignment, left side, right side |
| Arrows / Shift+arrows | Move one row/column / five scenes or eight tracks |
| Bank Lock | Hold the current session window |
| Pan / Sends / User | Track knob mode: pan / reverb or echo send / level |
| Sends again | Switch send A/B |
| Device knobs | Selected track effect mix/parameters; Parameter Bank changes the page |
| Device arrows / On / Lock | Select effect / bypass / retain the current target |
| Master Select + device knobs | Wet/parameter pairs for three master FX, Cue mix, Master level |
| Clip/Device View / Detail View | Session/Compose view / effect panel |
| Tempo / Shift+Tempo / Nudge / Tap | Whole-BPM steps / tenth-BPM steps / ±0.1 BPM / tap tempo |
| Cue level | Relative headphone volume |
| Play / Stop / Record / Session Record / footswitch | Transport / Stop / recording |
| Metronome | Toggle metronome |

Send A is stereo reverb; send B is stereo echo. Track assignments use the NS7's current contour and a five-millisecond gain ramp. Feedback publishes clip colors, Arm/Solo/Activator, selection, assignments, mode buttons and both knob-ring groups. Device knobs require a populated effect rack; Master Select offers the prepared master processors.

## MPD232 LiveLite

| Control bank | Faders 1–8 | Knobs 1–8 | Switches 1–8 |
| --- | --- | --- | --- |
| A | Track levels | Pan | Mute |
| B | Track levels | Send A: reverb | Solo |
| C | Track levels | Send B: echo | Record arm |

All four pad banks preserve their musical note, USB channel, velocity and release. Pad Bank, Control Bank, Note Repeat, time divisions, sequencer controls, preset editing and the display encoder operate in the hardware; Omatainer receives the resulting notes. Standard MIDI Start/Continue/Stop, MMC and fixed Akai CC transport are accepted on an enabled MPD input. CC 117/118/119 are Stop/Play/Record, with repeated presses and releases preserving the intended state; these addresses are reserved. Physical capture confirms these addresses on preset 1, LiveLite. The installed app's Play and Record states changed and cleared on Stop.

Physical captures cover every fader, knob and switch in Control Banks A/B/C, including switch releases at zero. Their wire addresses are CC 32–39. Faders are CC 12–19, knobs CC 22–29. The qualified preset stores banks A/B/C on wire channels 0/1/2. The knobs are endless encoders; in LiveLite their emitted values are absolute 0–127. All sixteen B/C send values match the installed app. The reader and configuration instructions are in [drivers/mpd232](../../drivers/mpd232/README.md). Read and replace the saved mapping if the hardware preset changes.

## Verification

Software fixtures exercise the production MIDI worker, command admission and audio renderer. They include the connected MPD preset and physical bank-A bytes. Connected captures and listening observations are recorded separately in [hardware qualification](../validation/hardware-resurrection.md); a mapped control is not automatically a physical listening result.

Wire sources: [Pioneer MIDI messages](https://downloads.support.alphatheta.com/software_info/dj-controllers/DDJ-SP1/DDJ-SP1_List_of_MIDI_Messages_E.pdf), [Akai APC40 mkII protocol](https://cdn.inmusicbrands.com/akai/attachments/apc40II/APC40Mk2_Communications_Protocol_v1.2.pdf), [MPD232 guide](https://cdn.inmusicbrands.com/akai/attachments/MPD232/MPD232-User_Guide-v1.1.pdf) and Akai's original MPD232 Editor 1.0.8. These native mappings include Omatainer-specific assignments listed above.
