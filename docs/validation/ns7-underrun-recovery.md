# Original NS7 output recovery

A scheduling underrun in the previous stage build retired the NS7 output after about nineteen minutes. The GUI remained open but PCM was closed. The earlier short quiet qualification is retained as historical evidence, not a sustained reliability claim.

Code revision `240fed5c0275396b2b328232098c2b0c1508619c` uses CPAL 0.18.2 and its recoverable ALSA underrun path. An underrun resets audible timing and arms the existing short output ramp without retiring the engine. Fatal output errors still retire the stream. MIDI uses midir 0.11.0 to share the current ALSA dependency. Linux hardware callbacks request real-time scheduling through rtkit; the application stays unprivileged. Other hosts retain their existing display names and buffer meaning.

The isolated stage profile uses `hw:CARD=NS7,DEV=0`, 44,100 Hz, I32 and four channels. Its saved 512-frame buffer becomes two 256-frame periods. Stable ALSA card names retain physical identity and the NS7 headphone routing. A USB reset marks an open driver PCM stream disconnected, keeping that event distinct from a scheduling underrun. MIDI retains its port and silent clock across the reset.

The fresh ordinary suite passed 1,699 tests with zero failures in 349.31 seconds, retaining 42 explicit hardware/service ignores and filtering the one unchanged exhaustive invalid-scene case that passed earlier in this worktree. The connected tests used silent output:

- An injected 40 ms callback stall produced one real underrun. Output recovered on the existing stream and reached 102 callbacks with no fatal backend error or lost-device event. The intentional stall produced one recorded deadline overrun. The hardware callback used SCHED_RR priority 10.
- The production fixture recorded 221,184 stereo frames and captured 267,264 input frames. Playback ran 292 callbacks with no underrun, fatal error or missed deadline. Three feedback outputs sent 476 writes without application-send failures; native USB PCM and MIDI completion errors remained zero. After closing both PCM streams, the MIDI owner completed another 4,457 silent playback frames.
- A muted GUI USB-reset check retired the output and closed PCM while both decks remained stopped. ALSA reported errno 77 as a fatal backend error, not a lost-device event; `device_lost` stayed zero. The receipt retains all 1,366 error callbacks before owner retirement. That reset was not classified as a recoverable underrun.

The forced fresh release finished in 57.23 seconds. The release artifact, installed binary and running GUI match SHA-256 `0a6a939cf2ad62af5b43806aa4645a8fb31f98d557e3a4f39c73648e0afc6e5c`. The native module was rebuilt and installed against kernel `7.1.13-3-2-ARCH` after sanitized C vector checks. Dependencies and source hashes have refreshed license catalogs.

The final GUI is visible on workspace 10 with both tracks and playheads restored, paused, at the retained 14% master level. Its callback has real-time priority. An 87-second quiet interval added 15,067 callbacks with no underrun, backend error, lost-device event, priority denial or missed deadline. Four feedback connections remain live. This is a bounded runtime check, not an extended stage-performance claim.

During UI cleanup, a scaled pointer movement accidentally started deck 2. Safe Stop halted it, and a fresh paused launch restored its saved playhead. The correction and state receipts are retained.

The [machine-readable receipt](ns7-underrun-recovery-receipt.json) records results, hashes, earlier failed check expectations and the final restored state. No new listening or physical-button claim is inferred from these tests. MPD232 bank B/C and transport gestures remain pending while Michael is AFK; the independent four-port capture stays open without a timeout.

## Subsequent status-capture exit

A background status recorder later exited with OS I/O error 5. Omatainer retained the same PID and binary throughout. A subsequent snapshot at about 49 minutes of uptime contained 514,129 audio callbacks with zero underruns, backend errors, lost-device events, priority denials or missed deadlines. Both decks remained paused at their saved positions. This is continuous quiet-output evidence, not a musical-performance check.

The recorder now runs as a read-only transient user service with direct file output and restart on failure. It produced 574 valid status frames with no stderr output or service restarts. The four-port MPD capture remains active without a timeout. The [status-capture receipt](ns7-status-capture-recovery-receipt.json) preserves the original exit and current process continuity; the exact cause of the old recorder's I/O error remains unestablished.
