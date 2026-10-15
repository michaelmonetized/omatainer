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

## Named sends and shared returns

Under **Named sends and returns**, add a named stereo return and choose each send's
source, destination, tap and channel map. A send has its own level, stereo balance
and mute. Return controls add solo and solo safe. The live mixer shows current
levels and peaks; changes use the same History journal as other mixer controls.
Stereo sends balance their source channels before channel mapping, including maps
to a sidechain. A mono source mapped to a stereo pair balances the destination
pair; native input and output buses use their local channel order. Mono-to-mono sends and
mono return strips ignore balance and retain their level. Balance remains unity
at the center, preserving existing stereo levels.
Pre FX and post FX taps bypass source mute, solo, fader, pan and crossfader. Post
mixer includes those controls. A return's mute changes its post-mixer tap; pre taps
continue processing. These tap rules also apply to automation and MIDI controls.

Return solo gates audible sums, including shared downstream sums. Source nodes
continue processing, and explicit secondary plugin input buses retain their
sidechain feeds. Any source feeding a soloed or solo-safe return remains available
to it. Solo safe keeps that return's audible path in the sum. Selecting an upstream
return includes its downstream processing. Existing track solo follows the track
tap rules above; return solo does not change them.

In **Native plugins**, choose **Return for effect → Append return effect** to add
an effect to a saved shared return. Review and apply the inspected routing draft.
Track duplication preserves shared return processors; deleting the return removes
its owned processor chain and dependent routes. Sidechains and extra plugin outputs
remain explicit maps. An ambiguous parallel chain or incompatible output width
refuses the draft rather than dropping channels.
Appending effects follows the declared primary bus; auxiliary buses retain their
saved metadata. An auxiliary post-mixer route that cannot follow that primary
chain requires explicit review and refuses automatic append. Track and return
contributions observe the final primary bus; auxiliary outputs keep processing
through their explicit graph routes.
Owned return effects take the bus pre-FX signal and pass their processed signal to
the next effect. The last effect's post-mixer tap applies return level, balance and
mute, so wet tails remain inaudible while muted and keep processing underneath.
Its post-FX tap remains available for an explicit pre-fader route.

Automation uses absolute song beats. Level and pan interpolate linearly; mute,
solo and solo safe step at each point. Endpoints hold outside the lane. While song
playback runs, a lane owns its parameter; stopping restores its saved manual value.
Each strip permits 16,384 points, with 65,536 total mixer points, 128 UTF-8 bytes per
name and 256 regular connections plus named sends. Graph and native delay storage
are admitted on workers. Invalid values, cycles or exhausted budgets leave the
active graph unchanged. Callback controls allocate no memory, take no locks and
perform no file operations.

Fixed reverb and echo amounts and crossfader assignments are saved per retained
track identity. New tracks start with zero sends and no crossfader assignment.
Duplication copies those controls; reorder, removal, Undo and save/reopen preserve
their identities. Fixed returns can also be selected as separate render contributions.

Contribution rendering validates a selection against one immutable capture and
keeps every source and sidechain feeder processing. Post-device track contributions
end before shared scene effects; scene contributions include that shared boundary.
Shared returns are separate passes. Post-device and explicit dry passes bypass main
processing and the final soft limiter; their original graph delay is trimmed once.
Mastered solo passes retain normal main processing and are not an exact-sum format.
Overlapping track/scene selections, ambiguous device forks, stale identities or a
changed output delay are refused. These software contracts do not establish
physical speaker mapping, device clock alignment or listening acceptance.

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
