# Audio routing

Open **Setup → Audio routing → Refresh routes**. The draft belongs to the inspected
project revision. Enable **Use explicit routing**, define channel aliases and buses,
then choose sources, tap positions, destinations and individual channel maps.

Physical channel numbers start at one in the editor. Aliases keep their exact
channels across device changes; absent channels remain silent. Input and output
aliases and buses support up to 32 logical channels. Record aliases support up to
26 channels, matching the pinned WAV decoder. Split wider captures across aliases.
There are at most 32 aliases per direction, 32 buses, 256 connections and 4,096 maps.
Saved physical addresses extend through channel 64; the pinned Linux CPAL/ALSA
backend advertises at most 32 live channels. A saved address does not make an absent
or unadvertised device channel available.

Track pre FX includes instruments, pads, clip audio and mapped inputs. Post FX
includes the track EQ and effect rack; post mixer includes mute, solo, gain and pan.
Deck pre FX is the decoded source. Post FX includes deck gain, EQ, filter and the
transition envelope; post mixer also includes crossfader gain. Bus post mixer adds
its gain and mute. Scene and main taps surround their existing effect chains.

Default track sends still follow the launched scene's effect bus. Default deck
sends still use the crossfader and main mix. Disable those sends when using an
independent route. Explicit additional routes sum with existing sends. Mono maps
can average left and right using two maps at gain 0.5. Alias names are unique and
saved endpoint identities survive reordering, deletion and slot reuse.

Stop all transports, review and confirm the routing draft. Feedback cycles,
invalid channel maps and changed project state reject the complete edit. Cancel
preserves current audio. History can undo and redo applied routing; Save and Open
retain aliases, routes, buses and input choices. Opening never enables an input.

Enter an exact input device/backend, channel count and sample format under **Live
input choices**. Preview discovers its advertised configuration at the active
output's nominal rate. Confirm **Stop and enable input** to open it. Input faults,
missing callbacks or output changes stop input; another preview and confirmation
is required. Mapped input is silent while disabled or unavailable. The fixed queue
reports missing input frames and overflow. Separate device clocks are not resampled;
these counters do not measure physical latency or exact driver dropouts.

Physical output and input meters report normalized digital peak levels. A stopped
output can play a one-second, ramped −40 dBFS channel test; cancel or starting a
transport ends it. Every physical output, including routes that bypass the main
mixer, obeys the same emergency-silence envelope.

Apply and refresh before capturing a record source. Choose a new WAV path, alias
and maximum duration. Capture writes floating-point samples on a worker, up to
128 MiB per file. Stop finalizes completed audio; Cancel removes the partial file.
An overflow, missing frame, invalid sample or file failure prevents publication.
Existing files are preserved. Routing, project, audio and emergency-stop boundaries
finish the active capture so one file keeps one source and sample rate. Licensed
provider preview also finishes captures before its first frame; new captures are
refused until preview stops. Transient preview audio never enters these WAV files.

The existing measured stereo performance-source attribution is unavailable while
explicit routing is enabled. An active measurement becomes incomplete at that
boundary; old stereo measurements are never reused as evidence for a new route.
Playlist events remain available. Hardware converter, USB and external interface
qualification must be reported separately from private software loopback results.
