# Audio command handoff

Tracking: [#5](https://github.com/michaelmonetized/omatainer/issues/5), in the consolidated [PR #510](https://github.com/michaelmonetized/omatainer/pull/510).

The installed stage application crashed during the physical MPD232 control check
on 2026-10-06 at 06:43:54 EDT. The core records `SIGXCPU`, `SI_KERNEL`, on
`cpal_alsa_out`. Its instruction pointer is inside Crossbeam's bounded-channel
receive backoff loop. A MIDI sender is simultaneously inside `memcpy` in that
same command channel's send operation. The kernel journal records signal 24 and
no OOM kill in that interval. This establishes the unfinished command handoff;
the exact scheduling delay before the signal is not recoverable from the core.

Production commands now use the existing `rtrb` single-producer/single-consumer
ring. The existing admission mutex serializes writers, making its producer
single-owner without adding an audio lock. The audio consumer reads only
published slots. A writer stopped before publication leaves the next command
for a later block instead of making the callback spin. FIFO, queue capacity,
reserved releases/stops and payload-byte admission keep their existing owners.
Crossbeam command receivers remain only in test fixtures.

The regression reserves an unpublished slot while retaining the producer
admission mutex, then renders 128 audio callbacks on another thread. Audio must
finish before the producer resumes, preserve the previous master gain, and
allocate/free nothing. Publishing afterward must deliver the new gain exactly
once. The complete command admission and surface checks also exercise releases,
Stop, simultaneous producers, payload limits and all three MPD control banks.

The separate four-port ALSA recorder survived the GUI crash. Michael physically
operated all eight faders, knobs and switches in Control Banks A, B and C. The
initial pass used Pad Bank; the repeated Control Bank passes returned wire
channels 1 and 2 for B and C. All 72 controls match the saved LiveLite preset.
Stop → Play → Stop → Record → Stop returned channel-zero CC 117/118/117/119/117,
each at value 127. These are physical MIDI receipts; native engine acceptance
after installing the corrected release is recorded separately.

Raw MIDI, crash details, build logs and the extracted core inspection stay in
ignored `target/hardware/`. The extracted private core copy was deleted after
inspection. The system's original coredump was retained. The compact
[qualification receipt](audio-command-handoff-receipt.json) records source,
tests and the subsequent running application check.

The complete final ordinary suite passes 1,771 checks, with zero failures,
43 explicit ignores and no filtered cases. The earlier run under simultaneous
build load had ten decoder/library completion timeouts; its log is retained,
as are successful isolated reruns. Both connected silent NS7 fixtures pass:
duplex capture/feedback and recovery from an injected 40 ms callback stall.

The clean source revision `7db2dc58faa4962f2eff824cde2a7374d2fbafea` was packaged,
verified and installed. Its manifest, installed executable and running GUI
agree on SHA-256 `617c8161972f638d5f83b25c52e975f8a119ffdb461f23c7c77067d52674add1`.
The GUI remains on workspace 10, with master zero and both decks paused at their
restored positions. Repeated physical B/C controls reached the corrected app;
all sixteen final send values match the last emitted knob values. Three complete
Stop/Play/Stop/Record/Stop sequences produced the expected native playing and
recording transitions. There were 1,251 additional MIDI messages, all dispatched,
with no input drops, resets, command submission refusals or pending commands.

At the retained final snapshot, audio had completed 51,480 callbacks with four
recoverable underruns and four deadline overruns, maximum overrun 5.529 ms.
There were no backend errors or device-loss events. The app survived these checks;
this does not qualify extended musical performance or acoustic latency.
Both temporary capture services were stopped after the check. The independent
pre-existing control follower and installed GUI were preserved.
