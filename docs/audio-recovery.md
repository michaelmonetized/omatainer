# Recovering lost audio

Device errors, a stopped callback stream or an owner/suspend gap over five seconds
retire the output on its owner thread. The project stays open. Recording ends at
the renderer's captured position; held notes, transports and GUI/controller input
ownership enter the existing recovery state. Save remains available offline.
Long explicitly requested buffers use a callback timeout of at least four buffer
durations. Recovery never starts playback automatically.

Open Audio offline in Setup. Reconnect retained output asks for confirmation and
reopens only the accepted physical identity with the same channels, rate, sample
format and buffer. Linux ALSA card routes retain a USB vendor/product/serial and
interface identity or a fixed onboard/PCI device path. USB devices without a
serial, ambiguous identities, missing routes and changed capabilities fail closed.
Card numbering and names may change without selecting a different physical unit.
Identity is checked again after opening and playing the still-muted stream; a
replacement during opening is retired before output is enabled.
The retained anchor belongs to this running session; it is not a portable hardware
database or a new preference setting.

Default/server aliases cannot establish a physical identity. Preview saved audio
or choose an available fallback in Preferences, save the settings, preview the
output and deliberately confirm Stop and change output. Alias routing remains
the audio server's responsibility, including rerouting an already live stream.
Recovery does not infer that the server's current default is the lost interface.
An offline DSP reset cannot select a default output as a hidden fallback.

After successful reconnect or fallback, release keys, pads and platter controls.
Choose Recover inputs, acknowledge that you released them, then press Play when
ready. Acknowledgment takes effect when the renderer drains prior work. Existing
emergency mute stays latched; only a successful deliberate stopped DSP reset
removes it. Managed transport starts ramp the complete output over two milliseconds.
The saved audio profile, project routes and musical edits remain unchanged by
reconnect. Explicit fallback uses the saved profile you previewed.

Linux aarch64 qualification includes actual App/worker confirmation, cancellation
and saved-state fixtures, reported backend errors, a stalled callback, physical
identity/renumbering fixtures and real CPAL/ALSA streams against a private
PipeWire null sink. `scripts/check-audio-recovery.py` exercises stream removal and
reopen, server restart, and a six-second process freeze while recording and
performing. It creates isolated runtime/configuration paths and retires only its
own child processes. ALSA enumeration may query hardware controls; the fixture
defines only a private default PCM and opens no physical audio stream.

Physical USB unplug/replug, actual system suspend/resume, external interface
listening and other desktop audio backends remain unqualified. Process freeze
proves the owner-gap path; it is not system suspend. See the validation receipt
for frozen measurements and artifact identity.
