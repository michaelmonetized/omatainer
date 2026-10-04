# Linux audio backends

Choose **ALSA** or **JACK** in a Preferences profile. **Preview changes**, then
**Apply and save** persist choices without opening an output. Open **Audio devices
and latency → Preview saved audio → Use saved audio now → Stop and change output**
to apply them while stopped. Cancel keeps the current output. Unavailable backends
and failed opening produce a visible error; rollback restores the prior output
when possible. Playback never resumes automatically.

## Supported paths

| Path | Startup | Clock and scheduling |
| --- | --- | --- |
| Direct ALSA interface | Select its exact advertised ALSA device | CPAL requests rate/buffer; the driver accepts the logical configuration. Callback scheduling follows the system's ALSA configuration. |
| PipeWire through ALSA | Select the installed PipeWire ALSA device | PipeWire may adapt the requested logical rate/buffer. This path has ALSA channels, without named graph-port control. |
| JACK server | Start the server before Omatainer; use `JACK_DEFAULT_SERVER` for a named server | The server owns sample rate, quantum and callback scheduling. Omatainer never starts or reconfigures it. |
| PipeWire graph | Start Omatainer with the system's PipeWire JACK library, commonly `pw-jack omatainer`; choose JACK | The PipeWire graph owns its clock and scheduling. `PIPEWIRE_REMOTE` selects its namespace when needed. |

The graph path dynamically loads `libjack.so.0`. Install the distribution's JACK
client library or PipeWire JACK compatibility package before launching it. Source
builds also require the distribution package supplying `jack.pc` for pkg-config. A
missing library or server is an error, with no automatic ALSA fallback. Library
selection happens at process startup; changing implementations requires restart.
ALSA and graph discovery run on workers, never inside the audio callback.

Set server rate/quantum and real-time permissions through your existing JACK or
PipeWire setup. PipeWire's real-time module and JACK's real-time mode depend on
system permissions; the application does not grant privileges or change the
desktop's configuration. A smaller quantum shortens the nominal buffer period but
must be qualified with the actual interface and workload. Software fixtures cannot
establish reliable real-time scheduling or audible physical output.

## Named ports and saved links

After discovering a JACK profile, its editor exposes **Named graph ports**. Choose
an output channel count, add exact destination links and save the profile. The
application creates `Omatainer:output_01` through the selected channel count.
Outputs never connect automatically to speakers or other system ports.

Saved input links use exact source names. **Audio routing** separately previews and
confirms live input activation: choose backend `JACK`, device `JACK graph`, F32 and
the required channel count. Only activation creates
`OmatainerCapture:input_01` through that count. Saving input links never enables
capture. Output changes and recovery stop input; enable it again explicitly.

Each direction permits 64 links, with unique channels and exact external endpoint
names. Missing or temporarily inactive endpoints remain disconnected; accepted
links retry on the owner worker and reconnect when the exact endpoint returns.
Already connected links are retained once. Duplicate owned client names, wrong
directions and known graph paths back into the application's capture/output are
refused. Nothing substitutes a different endpoint.

Graph review follows the actual directed connections and processing clients;
physical connectors are independent terminal endpoints. Hidden processing inside
external devices/clients and acoustic feedback cannot be inferred from JACK port
metadata. Review external monitoring and routing before enabling capture.

## Changes and recovery

The graph adapter supports 1–64 F32 planar channels, rates 8–384 kHz and quanta
16–32,768 frames. Its interleaving buffers are allocated before activation.
Quantum notifications change only atomic state on the callback; the owner updates
the displayed accepted quantum without replacing the stream. Excessive quantum,
rate changes or server shutdown stop the output while retaining the project.

**Reconnect retained output** reviews the same retained library/server namespace
and saved exact routes. It follows that server's current rate/quantum after explicit
confirmation. Playback stays stopped; existing input recovery still requires
acknowledgment. A changed namespace/library requires a fresh preview and output
confirmation. The retained identity identifies a logical server, not physical
hardware. Native clients and ports retire on their owners before process exit.

Optional physical **Measure loopback** remains the ALSA workflow. It is disabled
for a graph output before stopping audio. The private backend driver measures
software callback-to-callback loopback separately; it does not claim converter
latency. See the [qualification receipt](validation/issue-133-linux-audio.md).

References: [JACK client lifecycle](https://jackaudio.org/api/group__ClientFunctions.html),
[JACK callbacks](https://jackaudio.org/api/group__ClientCallbacks.html), and
[PipeWire JACK configuration](https://pipewire.pages.freedesktop.org/pipewire/page_man_pipewire-jack_conf_5.html).
