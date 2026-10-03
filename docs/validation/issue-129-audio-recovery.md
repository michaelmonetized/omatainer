# Audio owner recovery: issue 129

The native audio owner detects callback loss, backend errors and a long suspend/owner gap without discarding its renderer or current session. Recovery stops transport, finalizes held recording notes, releases gates and preserves emergency mute. Save and Close stay available while offline.

Linux physical ALSA devices are bound to captured sysfs identity and the exact accepted output route. Verified USB identity requires a serial plus vendor/product/interface data. Alias/default outputs, ambiguous inventories and USB devices without a serial require an explicit fallback review. Stream creation rechecks physical identity before activation. A failed fallback keeps the original recovery target available.

Audio devices exposes current recovery state, retained target, generation-bound reconnect/fallback review and cancellation. Reconnecting never starts playback. Physical inputs must be released and recovery acknowledged before an explicit Play. The first resume onset ramps for two milliseconds; starting another deck does not fade an existing mix again.

## Current evidence

The focused recovery run passed 68 checks, with two opt-in tests ignored. It includes callback stalls, synthetic suspend timing, ambiguous/changed physical identity, retained graphs, failed fallback retention, renderer note finalization and resume-ramp behavior. Native App controls exercise review, cancellation and stale approval. These are software fixtures.

`scripts/check-audio-recovery.py` separately creates a private PipeWire 1.6.8 server/null sink and isolated ALSA configuration. It opens a real CPAL/ALSA stream, starts composition recording, kills only the owned server, waits for owner recovery, restarts it, reviews an explicit fallback, releases inputs and resumes deliberately. The retained native project is saved/reopened and captured MIDI remains intact. The initial successful receipt records 286 callbacks before the fault, 366 after restoration and all owned children exited. Final-source receipts are retained with the combined PR qualification.

No physical USB removal/reinsertion, converter loopback, audible listening or real system suspend is claimed yet. `scripts/hardware-probe.py` only records actual USB descriptors and backend visibility; it cannot certify compatibility. Controller/interface fault testing remains on the combined checklist.
