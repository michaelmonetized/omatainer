# Live Pioneer and APC40 checks

The connected DDJ-SP1 passed the sixteen HOT CUE pads, six FX On buttons, and
six full-range FX knob checks. The APC40 mkII passed all eight track faders,
Pan knobs, Arm, Solo, Activator and Track Select inputs. Its Pan, Sends and User
selector buttons produced no raw MIDI in either host mode or the isolated
generic-mode check during this earlier interval. Software selection changed
native Sends/User modes and their physical lights correctly. The later
[powered-hub check](powered-hub-controller-qualification.md) physically qualifies
PAN/SENDS/USER, Scene Launch 1, Stop All, Master, Device On/Off and REC.

The [receipt](live-controller-acceptance-receipt.json) belongs to installed code
`7db2dc58faa4962f2eff824cde2a7374d2fbafea`, executable SHA-256
`617c8161972f638d5f83b25c52e975f8a119ffdb461f23c7c77067d52674add1`.
The installed executable and running GUI, PID 1771583, matched that hash.
Later loop-editor source changes are outside this physical qualification.

The later [grid row-order check](apc40-grid-order.md) independently qualifies
all forty grid inputs and eight Clip Stop inputs, and the corrected scene-1,
track-8 pad's physical yellow/green/yellow loaded, launched and stopped states.
Its executable and test receipts have their own source identities. Scene
Launch, Stop All and selector inputs were unresolved in that earlier check;
the powered-hub receipt records the later recovered controls.

## Pioneer

Every HOT CUE pad returned a press and release, set its native deck cue, and
caused the matching cue-set LED message. All six FX On buttons were observed
on and off in the native surface state, with both LED values sent. Raw MIDI
and outgoing feedback establish application I/O; outgoing bytes alone do not
establish physical LED appearance or acoustic quality.

The first knob pass produced no CC messages. The completed repeat captured
6,836 CC messages, forming 3,418 paired fourteen-bit FX assignments. Each of
the six knobs reached 0 and 16,383, ended at zero, and matched zero in its native
FX slot. All 3,418 assignments reached command consumption. That repeat had
zero command refusals, MIDI drops, underruns or deadline overruns. It does not
reproduce a separate knob-decoder loss.

Addresses and resolution agree with the
[Pioneer MIDI message list](https://downloads.support.alphatheta.com/software_info/dj-controllers/DDJ-SP1/DDJ-SP1_List_of_MIDI_Messages_E.pdf).
Shifted parameters and the other performance controls retain their separate
software and physical acceptance boundaries.

## APC40 mkII

After audio recovery, every track supplied two Arm presses, two Solo presses,
two Activator presses and one Track Select press. Arm, Solo and Activator
feedback included both zero and one for every track. Track 1 was selected
already, so its redundant selection did not require another LED-on message;
subsequent selections produced the expected changes. Michael explicitly
confirmed that track 1's Arm light turned on and then off.

All eight physical faders traversed values 0–127 and ended at 127. All eight
Pan knobs supplied input. Their final raw values were
`127, 127, 123, 127, 127, 127, 127, 127`; the native feedback rings matched those
values exactly. The third knob's full 127 endpoint is not claimed.

The requested Sends A, Sends B and User pass produced additional knob values,
but none of the corresponding selector presses arrived. Native knob mode
stayed Pan, with both send banks still zero. Resending mode `0x41` did not
produce selector messages or visible lights. The documented alternate mode
`0x42` was then sent and returned eight track-fader positions, master 127 and
crossfader zero. A native command restored master zero while playback and
recording stayed stopped. The alternate-mode comparison captured only the
two Arm presses and releases, with matching native feedback. None of the
four selector presses arrived. Michael reported that Arm changed, Pan stayed
lit, and Sends/User stayed dark.

A separate software-generated Sends press and release entered the existing
APC input connection. Native knob mode changed from Pan to Sends, and the
application sent Pan off and Sends on, with no refusals or dropped input.
Michael confirmed Sends lit and Pan dark. A software User gesture then
changed native mode to User and sent Sends off and User on, again without
refusals or dropped input; Michael confirmed both light changes. Master
stayed zero and playback/recording stopped. These injections qualify the
installed application path and observed lights; they do not qualify physical
selector presses.

The generic-mode `0x40` comparison disconnected the APC input subscription
to Omatainer while keeping the independent raw capture subscribed. Setup
returned ten fader positions to the capture and none to Omatainer. Two
physical Arm presses produced one Note On and one Note Off, matching the
controller’s documented generic toggle behavior. Pan/Sends/User produced
no messages; Michael reported that only Record Arm changed. Omatainer
received no input and sent no feedback during this physical check.

Host mode `0x41`, the app input subscription and native Pan mode were restored.
Master remained zero and playback/recording stopped. The first restoration
launcher failed before changing state because system Node rejected an
inherited option; the retry removed that option and verified restoration.
The absent selector input is isolated from Omatainer’s input processing and
Sends/User light output. Its hardware or firmware cause is not established.

Both host modes and the selector/LED addresses are specified in Akai's
[communications protocol](https://cdn.inmusicbrands.com/akai/attachments/apc40II/APC40Mk2_Communications_Protocol_v1.2.pdf).
No firmware flash or hardware preset write was performed. Scene/grid playback,
the remaining device controls and physical selector operation are still open.

## Audio recovery and limits

An earlier live interval recorded a 5.357480790-second callback stall. The
audio owner retained the project, stopped the unavailable output and latched
recovery. APC button input continued arriving, but commands were refused and
no new light states were sent. Clearing the recovery latch alone did not
restart the output; the first short Arm retry was refused as audio unavailable.

The native GUI's **Audio offline → Reconnect retained output → Confirm retained
output reconnect** reopened the same NS7 ALSA output: four-channel i32,
44,100 Hz, buffer 512, observed callback 256 frames. Callback advancement
resumed, and the subsequent physical Arm check worked. The musical material
and deck positions were retained; output stayed muted and playback stopped.

The recovered APC mixer interval recorded 64 underruns and 63 deadline
overruns. It had zero command refusals and zero dropped MIDI input. These
receipts qualify the demonstrated controls, not extended stage reliability.
No new listening qualification is claimed. Original captures, native snapshots,
the small host-mode sender and analysis remain under ignored
`target/hardware/pioneer-apc/`, with immutable capture-prefix hashes in the
receipt.
