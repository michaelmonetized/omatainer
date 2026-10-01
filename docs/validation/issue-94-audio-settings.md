# Issue 94 — professional audio settings and qualified loopback timing

The saved profile and the active stream are distinct. Preferences Apply writes
version 3 without disrupting performance. Audio devices and latency previews the
saved output and current calibration route; a separate confirmation either changes
the output or performs an opt-in loopback probe. Neither path resumes playback.

## Ownership and failure policy

CPAL 0.15.3 streams stay on a dedicated owner thread. The existing
`OutputCallback::new(rt, channels)`, `render` and `render_timed` headless interfaces
remain intact. Production uses a unique boxed graph lease and a preallocated return
slot; Linux ALSA `Stream::drop` wakes and joins its worker before the owner consumes
the graph. The owner retains that slot through failed build/play and stream teardown.
There is no renderer mutex or GUI/device-state read from a callback. A backend that
fails to return ownership cannot cause a second graph to be manufactured or creative
admission to reopen. Non-Linux stream ownership is explicitly unsupported until its
contract is verified. OS calls can still block; cancellation cannot interrupt them.

The output commit is the atomic false-to-false cancellation check after successful
build/play and before enabling callbacks. Cancellation ordered before that decision
wins; later cancellation cannot turn committed success into failure. A separate
shutdown flag silences managed callbacks regardless of a delayed backend return.

A project seal closes creative admission and drains prior commands before device
work; pending GUI intentions reject the transaction. Existing project Close/install
ownership is preserved. If the target and rollback both fail, the graph remains on
the owner. An independent offline flag rejects creative commands while zero-frame
processing services project capture, file Save, New/Open install, library fences,
physical releases and Close. No timeline advances and no fake audio metrics are
published by that service loop. Snapshot publication retries on an owner clock even
when no frames render, so a held reader cannot strand an installed project behind
an obsolete display revision. DSP preparation uses the current logical sample rate
and installation rejects a graph prepared for an obsolete rate. Same-rate switches
also clear stereo EQ histories and rebuild saved MIDI/arp launch state; explicit
Play resumes with no render allocation instead of leaving an empty event schedule.

## Capabilities and metrics

Discovery lists input and output devices, formats, rates, channel counts and buffer
limits, bounded to 256 devices per direction and 4096 ranges per device. Missing
names, duplicates, incompatible modes and a preview that changed before activation
fail visibly. Exact backend/device names are persisted; CPAL provides no portable
hardware serial identity. Channel numbers are ordered interleaved channels, not
invented physical connector labels. Output remains main L/R on channels 1/2, mono
sum for one channel, and silence on additional channels; this adds no cue routing.

The active display says **backend-accepted logical configuration**. CPAL/ALSA uses
nearest-rate negotiation internally and exposes no physical-rate getter; the UI
therefore does not call the request a physically negotiated rate. Observed output
callback frames and backend output timestamp scheduling estimates are separate.
The roundtrip buffer estimate is the sum of explicitly requested input/output
buffers at the logical rate, excluding driver/converter time. It is unavailable
when either buffer is backend-selected.

## Opt-in calibration

The user chooses input/output channels and a -60 to -24 dBFS probe level, connects
a suitable line-level cable or interface loopback, disables monitoring, turns down
external speakers, and explicitly confirms. The real CPAL path temporarily opens
the selected input/output, plays three distinct deterministic coded probes on only
the chosen output, captures up to three seconds into preallocated buffers, and
restores session output while leaving performance stopped. There is no input monitor.

Both callbacks stamp a shared host `Instant` at entry. Analysis matches the actual
recorded codes and reports the median input-callback minus output-callback entry
time, nominal callback resolution at the logical rate, and three-repeat spread.
This is **host callback-to-callback loopback return latency**, not physical converter
roundtrip. Independent ALSA `StreamInstant` origins are never subtracted. An accepted
result retains the exact profile, stream plans, channels and probe level. New
attempts/cancellation, profile/configuration changes or offline recovery cannot show
an unrelated prior result as current.

Quality gates require three ordered distinct probes, strong normalized correlation,
a clear unique peak, adequate signal/noise margin, finite unclipped data, contiguous
bounded callback stamps, finite duration and consistent timing. Silence, random noise,
duplicate/echo ambiguity, clipping, broken stamps, insufficient input, backend failure,
shutdown and timeout produce **no measurement**. Binary-search stamp lookup bounds
analysis cost even for very small callback blocks. Analysis runs off the callbacks.

## Evidence

- Threaded fake backend runs the actual managed callback and project queue. Tests cover
  44.1/48/96/192 kHz, preserved controls, stopped performance, rollback, double failure,
  concurrent project capture, real project-file save/reopen while offline, obsolete-rate
  install rejection, cancellation, late-cancel commit, shutdown during delayed open,
  same-rate MIDI/arp resume, and offline snapshot retry after reader contention.
- Real egui App actions cover preview/confirm/keep-current, saved preference reopen,
  all four rates, rollback/offline recovery, explicit calibration, cancellation,
  missing loopback/timeout and invalidated/unsupported previews. These are renderer
  and UI fixtures, not a physical-device success claim.
- Production probe routing and capture are checked for zero callback allocation/free;
  managed steady-state rendering and shutdown silence are likewise measured. Synthetic
  captured PCM uses all four rates and verifies matched sample positions and qualified
  timing; corrupt/noisy/ambiguous/inconsistent evidence is rejected.
- A read-only native inventory ran on Linux aarch64 using ALSA. It reported three
  output and three input logical devices (PipeWire/default and Apple J316 aliases),
  no discovery errors and no truncation. This queried capabilities; it did not play
  an output stream or run a loopback probe. Evidence: `/tmp/issue94-native-inventory.log`.
- Final full suite: **636 passed, 8 opt-in ignored** (`cargo test --
  --test-threads=1`, `/tmp/issue94-core-full.log`). A subsequent assertion-only
  extension checked actual positive/negative clipped PCM through capture; all six
  calibration groups passed again (`/tmp/issue94-calibration-final.log`).
- Production `cargo build` and `git diff --check` passed. No external desktop
  configuration, current audio route or physical interface was changed during QA.

No physical loopback measurement, external interface activation, Pioneer/Numark/Akai
controller test, native-window assistive-technology session or human listening QA
is claimed. The user's final producer/composer/live-DJ hardware qualification remains
separate. Issue 96's performance permit can wrap destructive worker admission and
`seal_for_audio`; preview remains nondestructive.

## Assembled contextual help

The issue 89 catalogue now supplies the audio preference editors, preview and
confirmation actions, cancellation, capability expanders and timing surfaces.
Output settings describe saved intent separately from live confirmation. Input
calibration controls specify one-based channels, dBFS level and the active output
rate; measured results retain the host-timing qualification. The setup lesson and
generated offline manual use the same supported workflow.

On assembled base `e6a5702`, all **668 standard tests passed; 10 opt-in tests were
ignored**. Eight audio UI groups include all 13 preference editors' AccessKit
purpose/units, popup choices, cancellation and Focus → F1 on the real probe
confirmation without starting an operation. The manual/catalogue synchronization
check passed. Production `cargo build` and `git diff --check` passed.

The private Linux AT-SPI harness passed against the fresh test binary, including
native Preferences scale access, project Save/New/Open, Undo/Redo, cue and pad
workflows (230 native nodes in this run). Evidence is in
`/tmp/issue94-help-full.log`, `/tmp/issue94-help-build.log` and
`/tmp/issue94-help-native.log`. This is actual App/renderer plus native API evidence;
no native window, Orca, desktop configuration or physical audio device was exercised.

The ordered integration additionally refreshed embedded source records and passed
a fresh production build, seven native CLI protocol cases and native package
verification. The assembled native accessibility run performed 75 actions and
persisted/reopened one project note and UI scale 1.25.
