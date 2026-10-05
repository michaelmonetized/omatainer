# Stage controller resurrection

Tracking: [issue #508](https://github.com/michaelmonetized/omatainer/issues/508). Base: `stack/remaining-backlog`, `1625ee063014a674d272f5d3fc01e80e8191fe2c`.

The original NS7 now has a native Linux ALSA driver. Omatainer has dedicated original-NS7 and DDJ-SP1 maps, a programmable MPD232 profile, controller feedback, periodic input reconnection, and IPC input/output counters. The qualification host is an Asahi ARM64 MacBook Pro running `7.1.13-3-2-ARCH`. The powered stage hubs and active 30-foot extensions remained connected during the tests.

## Connected evidence

| Hardware | Demonstrated | Physical checks still open |
| --- | --- | --- |
| Original NS7 `15e4:0071` | Native ALSA four-channel output at 44,100 Hz, stereo input captured through Omatainer's production input worker and recorder, completed MIDI feedback USB writes, real device clock feedback, MIDI retained across targeted USB reset | Incoming panel gestures through Omatainer, audible output/channel identity, headphone cue routing, platter touch and motor control |
| DDJ-SP1 `08e4:0181` | Real Omatainer MIDI output connection and feedback sends; direct documented application-connect/Sync LED messages completed on its USB OUT endpoint | Incoming physical gestures and visible LED-state observation |
| APC40 mkII `09e8:0029` | Real universal MIDI identity response received and dispatched by Omatainer; feedback output sends; dedicated original/mkII address spaces | Physical fader, grid and scene operation and LED color observation |
| MPD232 `09e8:0036` | Programmable-note profile and MIDI input lifecycle software tests; repeated USB enumeration/disconnection captured while its Omatainer ports were excluded | Display-restart fault and physical I/O remain open |

The first release-mode production NS7 fixture recorded 221,181 stereo frames, 5.015442 seconds, at 44,100 Hz. Its four-channel output ran 351 callbacks with zero backend errors, lost-device events or missed deadlines. Capture reported zero overflow and underrun. Both recorded channels contained finite low-level analog samples, with no NaN or infinity. No output tones or media were played. Feedback connected three hardware outputs, sent 86 messages with zero application-send failures, and completed nine NS7 MIDI payload bytes on USB. The single application input event was the APC identity response; it is not an NS7 or Pioneer panel gesture.

The repeatable fixture is ignored by ordinary tests because it requires the physical original NS7. Run it alone, with other NS7 audio applications closed:

```sh
mkdir -p /home/michael/Projects/omatainer/t/usb
TMPDIR=/home/michael/Projects/omatainer/t/usb \
OMATAINER_NS7_QUALIFY_DIR="$PWD/target/hardware/ns7-qualification" \
cargo test --locked original_ns7_production_audio_and_midi_io \
  -- --ignored --nocapture --test-threads=1
```

Its receipt records application metrics, recording frames, and native USB counter deltas. The analog recording stays in ignored `target/`; manufacturer driver packages, disassemblies, USB captures and build logs also stay there. Native counters include actual USB completions, independently of ALSA enumeration and application enqueue success.

The final-driver repeat also passed: [retained receipt](hardware-resurrection-receipt.json), 220,167 recorded stereo frames (4.992449 seconds), 345 output callbacks, 266,752 input frames, and zero application backend/deadline errors, input overflow/underrun, native MIDI errors or native PCM errors during that run. It completed 290,466 playback frames, 290,424 capture frames and 6,584 clock packets on USB.

Ordinary tests passed 1,642 cases with zero failures and 39 hardware/service ignores. The unchanged exhaustive invalid-scene test was skipped in that repeat; it passed earlier in this worktree. MIDI qualification passed 132 cases. The native C vector checks passed with bounds/undefined-behavior sanitizers, and the module built against the running ARM64 kernel. A fresh release application build completed from this worktree and replaced the old installed binary after retaining a backup.

The final application's USB capture contains SP1 application-connect `09 9b 09 7f` and state-dependent LED messages, followed by successful OUT completions. NS7 feedback contains completed 42-byte vendor transfers carrying Play/Cue LED payloads. APC's real identity response arrived on its USB IN endpoint and reached the application input worker. Extended empty-project output remained running with zero application backend or deadline errors. This is a silent service check, not a musical performance or a physical listening check.

## Transport and reset behavior

The NS7's vendor interfaces do not expose standard USB Audio/MIDI descriptors. The new driver owns both interfaces, registers ordinary ALSA raw MIDI and PCM, validates high-speed endpoint layout, preserves the manufacturer's status bits and uses its frequency setup sequence. It supports four S32_LE playback channels and two S32_LE capture channels, with 24 significant bits at 44,100 Hz. It strips the MIDI transport padding, bounds every transfer, and acknowledges output payloads only after complete USB writes.

The initial firmware transport accepted writes while its audio clock returned zero-length/zero-valued feedback. Setting only output/capture endpoint rates did not resolve that state. Manufacturer 3.3.11 also sets the clock endpoint's rate. The completed rate sequence and a targeted USB reset started real clock and capture transfers. The driver now verifies rate readback and requires a valid clock within 500 ms of preparation.

On a targeted USB reset, audio URBs and MIDI transfers stop before reset; MIDI resumes afterward without changing the ALSA port. Active PCM reports an XRUN. Omatainer records the backend error and stops the interrupted output through its existing audio owner. Reopen the output after reset. Resets do not automatically resume a performance.

The installer builds and tests locally, links the module into the running kernel's module catalog and loads it through its device alias. `--reset` adds one reset specifically for `15e4:0071`. Reinstall after kernel upgrades; this is not a DKMS package. Keep the checkout/module path in place.

## Controller behavior

DDJ-SP1 selection precedes the broad DDJ matcher. Its four wire deck banks mirror Omatainer's two decks. Hotcue pads, separate shifted deletion notes, loop controls, browser navigation, loading and paired 14-bit FX controls use the official addresses. Select the SP1's documented non-Serato MIDI utility setting when using Omatainer. Other pad modes and unmapped surface notes are ignored. Application-connect, Sync, loop and hotcue LEDs follow engine state outside the audio callback.

Original NS7 deck controls share MIDI channel 1 with separate note addresses. Its wheel CCs carry wrapping absolute positions; each source keeps independent history and clears it across input epochs. Fader and trim values combine per deck so turning trim cannot reopen a closed fader. Feedback uses documented Play and headphone Cue LED CCs. Motor commands, platter touch, unknown browser messages and uncertain legacy addresses remain unmapped.

APC original and mkII maps remain separate. Feedback mirrors the clip grid and requests the standard MIDI identity once per output connection. Discovery excludes Omatainer's own ports, preventing recursive connections to its virtual inputs/outputs. Input policy also limits feedback connections. Performance protection gates automatic device changes.

The MPD232 allows each pad, knob, fader and switch to be programmed. Its profile preserves every incoming musical note, channel and velocity rather than consuming eight notes as MPK hotcues. Assign configurable controls through MIDI learn and save the assignments. MIDI Start/Stop and Clock retain the existing transport handlers; MMC is not mapped to transport. Its output owner supports the standard identity request; proprietary LED/program writes are not implemented. No software change has been demonstrated to fix its display restarts. The manufacturer's power specification is 6 V DC, 1 A, center-positive for its optional adapter; its manual also requires a powered hub for USB power.

MPD232 was absent from some instantaneous inventories because it repeatedly disappears between enumerations. Live sampling found USB power management set to `on`, runtime state `active`, and zero suspended time before each disconnection. Autosuspend was not triggering these observed restarts. Its firmware declares 500 mA bus power and has a malformed HID endpoint count; ignoring HID earlier did not stop the cycling. Its Omatainer inputs were excluded during the live reset observation. No persistent HID quirk or firmware flash was applied. A stable power/cable baseline and a firmware-version check are still needed to isolate the remaining cause.

## Protocol sources

- [Numark original NS7 downloads](https://www.numark.com/product/ns7), Mac driver 2.2.6 and [3.3.11 package](https://www.numark.com/images/product_downloads/Numark-NS7_3.3.11.zip): MIDI framing, status preservation, PCM bit planes, sampling-frequency requests and clock observer.
- [Numark NS7 reference manual](https://www.numark.com/images/product_downloads/ns7_reference_manual___v1.1.pdf): physical controls and audio connections.
- [Mixxx original NS7 mapping](https://github.com/mixxxdj/mixxx/blob/main/res/controllers/Numark%20NS7.midi.xml) and [wheel script](https://github.com/mixxxdj/mixxx/blob/main/res/controllers/Numark-NS7-scripts.js): original wire controls. Its preliminary mapping contains conflicting legacy addresses; those were not copied.
- [Pioneer DDJ-SP1 official MIDI list](https://downloads.support.alphatheta.com/software_info/dj-controllers/DDJ-SP1/DDJ-SP1_List_of_MIDI_Messages_E.pdf): deck banks, shifted pads, browser, FX and LED messages.
- [Akai MPD232 user guide](https://cdn.inmusicbrands.com/akai/attachments/MPD232/MPD232-User_Guide-v1.1.pdf): programmable notes/controls, USB ports, transport modes and power.

This establishes connected I/O and software behavior, not physical listening or a full stage-performance acceptance test. Small-buffer full-engine operation previously underrran under build load; the connected qualification uses 2,048 frames. Lower-latency settings need a separate sustained performance check.
