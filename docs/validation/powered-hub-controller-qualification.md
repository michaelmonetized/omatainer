# Powered hub controller qualification

The previously missing APC40 mkII buttons now work in Omatainer. Michael confirmed “Done all worked, lit as expected.” The check ran on m1pro16 on 7 October 2026 with all four controllers on the new powered hub and the installed release `0.1.0+70526b724672`.

Moving the devices changed their ALSA client numbers. The old application held stale input and feedback connections. The Hardware profile now selects the four current exact input names, and the latest installed executable was restarted with the native saved session. Master, crossfader and both deck positions matched before and after restoration.

| Controller | Saved input | Current result |
| --- | --- | --- |
| Original NS7 | `Numark NS7:Numark NS7 MIDI 24:0` | Input and feedback connected; retained four-channel 44.1 kHz ALSA output active |
| APC40 mkII | `APC40 mkII:APC40 mkII MIDI 1 28:0` | Requested buttons and state-dependent lights physically confirmed |
| DDJ-SP1 | `Pioneer DDJ-SP1:Pioneer DDJ-SP1 MIDI 1 32:0` | Input and feedback connected; user reports no known issues |
| MPD232 | `MPD232:MPD232 Port A 36:0` | Input and feedback connected; user reports no known issues; the other three ports stay disabled |

PAN → SENDS → USER → PAN → SCENE LAUNCH 1 → STOP ALL CLIPS → MASTER → DEVICE ON/OFF and two track-1 REC ARM presses all produced matching press/release pairs. One extra SENDS pair preceded that exact twenty-message sequence. The 22 ALSA input messages match the application's receive and dispatch deltas, with zero drops and no source resets. Feedback increased by 95 messages with four connected outputs and zero send failures. The APC OUT capture contains 73 successful completions, 960 bytes and no failed completions. Its final knob mode is Pan, the master device is selected, and Device On/Off changed master echo wet from 0.5 to zero. The user confirmed the expected lights and app behavior.

The original private diagnostic build was replaced by the qualified installed executable. Both the running executable and installed release have SHA-256 `3217e4bfce388cac60e0ad7bea4caf83ec0d323f6eb620d4bf3a04d2c09ed69d`. No mapping source change was required. The selected input policy is saved in the existing Hardware profile. Native Save As preserves the pre-test project and stores the final session separately. Diagnostic input and root USB capture units are inactive, MainPID 0; no reader holds usbmon open. The app remains running with all four input and feedback connections.

The user reported no known problems on NS7, DDJ-SP1 or MPD232. Those panels were not re-swept in this interval. Scene Launch 2–5 were not re-tested here. The endpoint-1 USB filter retained LED OUT transfers, not endpoint-2 input; the input proof is ALSA, exact application counters, native state and the user's physical report. Hub topology, controller initialization and executable changed together, so the original fault's cause is not isolated.

The focused controller interval ended with zero audio errors, device loss, missed deadlines and xruns. Later handback counters contain 13 missed deadlines and 14 xruns with no backend errors or device loss. This is controller acceptance, not extended musical-performance or audible-playback qualification. Exact source, executable, capture, session and cleanup evidence is retained in [the receipt](powered-hub-controller-qualification-receipt.json).
