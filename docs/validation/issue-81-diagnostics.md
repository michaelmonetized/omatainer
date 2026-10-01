# Issue 81 — bounded performance diagnostics

Open **Diagnostics** in the shipped audio status bar. The panel separates:

- Render-thread CPU divided by the real callback buffer duration. Existing
  `CLOCK_THREAD_CPUTIME_ID` measurement excludes command handling, conversion
  and publication; sleeping is not counted as CPU.
- Complete callback wall service time, deadline overruns, maximum elapsed time,
  backend errors and device-loss notifications. Commands, DSP, output conversion
  and snapshot handoff remain inside the callback wall measurement.
- Predicted output latency from CPAL's playback timestamp minus its callback
  timestamp. This is a backend prediction to playback, not measured round-trip
  latency. Invalid/unavailable timestamp differences remain unknown.
- Previous `App::update_frame` wall time, including control/layout work. This
  excludes GPU presentation and frame pacing; it is not FPS or audio latency.
- Audio queue depth/capacity, observed producer high-water, reserved gate
  releases, full-queue rejections, all rejections, accepted/coalesced totals;
  library queue pressure and MIDI discard/reset counters. High-water is an
  observed sample, not a claim of an unsampled instantaneous maximum. Reserved
  releases and stops explain why ordinary admission can reject below capacity.

CPAL 0.15 exposes backend errors but no trustworthy exact hardware XRUN count.
The UI and exported `dropped_buffers: null` preserve that distinction; neither
service-time overruns nor MIDI discards are relabeled as hardware XRUNs.

## Track and device attribution

Opening the panel or starting a capture enables an optional sampler. It times
one actual audio sample frame every 2,048 rendered frames using `Instant`:
track totals, scene-bus totals, both decks, the three master devices, shared pad
sources, and the first 16 FX devices of each track/scene chain. Additional FX
still render normally and the omitted count is explicit. Timing storage and
publication are fixed arrays/atomics; callback publication never allocates,
locks, retries, formats strings or waits for a GUI reader.

The panel identifies every measured device by its original track/scene/slot
and effect kind. It displays wall nanoseconds per sampled frame and the buffer
share estimated if that frame repeated. This sampling can miss event spikes;
track/scene totals include their FX, so the figures are not additive. Timer and
scheduler overhead are included. A zero timer difference is shown as below
resolution, not proof of zero cost. These numbers are not per-track CPU or a
worst-case headroom guarantee. The continuous callback meters remain the
primary deadline measurements, including profiling overhead when enabled.

## Capture, export and reopen

**Start capture**, **Stop capture** and **Cancel capture** operate on at most
30 seconds / 120 GUI samples, taken no faster than 250 ms apart. A delayed GUI
does not fabricate the missing intervals. Capture includes the actual callback
metrics, sampled profile, queue counters and previous UI update time. Metadata
contains app version (optional build revision, otherwise explicitly unavailable),
OS, architecture, logical CPU count, audio backend/format, MIDI status-entry
count, track/scene/clip counts and session tempo. Status entries can include
fallback/error entries and are not represented as physical input-device count.

Export uses an allowlisted schema that never includes media paths, titles,
usernames, hostnames, port names, raw MIDI messages or note content. Serialization
and file I/O run on one worker; only one operation can run at a time. The user
chooses the path. A 0600 temporary file is written/synced and published with a
same-filesystem hard link, which never overwrites a prior file or follows a
destination symlink. Temporary files are removed on failure/cancellation.
Publication wins a later cancellation and is reported as exported. Reopen uses
bounded regular-file reads without following symlinks, enforces a 2 MiB limit,
checks schema/counts/timeline/device indices, and displays captured data without
restoring or altering the live engine. Corrupt input preserves the previous
capture. Filesystem calls already in progress may finish before cancellation is
observed; the GUI and audio callback do not wait for them.

## Validation

- Real callback with an 8 ms injected device wait: independently bracketed
  wall time contains the measured device, track and callback costs; actual
  render CPU excludes the wait. Known 12 ms output prediction and real
  frames/rate/channels reach metrics exactly; an overrun is counted.
- Dense 64-voice / 18-device fixture: profiled output equals an unprofiled
  renderer bit for bit. Exactly 16 device timings and two omitted devices are
  reported. Twenty warmed callbacks crossing publication periods perform zero
  allocations and frees on the callback thread. Disabled profiling stops
  publishing new samples.
- Real queue saturation proves capacity, depth, observed high-water, full
  rejection and reserved-release reporting before/after renderer drain.
- Actual egui workflow opens the shipped panel, starts/stops/cancels capture,
  exports and reopens a private real file, and rejects existing destinations
  and malformed input without changing project state or the previous file.
  A controlled paused file worker proves cancellation and ongoing UI frames.
- Private metadata containing a user path/title/port name is absent from the
  exported JSON. File permissions, duration/sample/byte limits, schema and
  invalid point rejection are checked.
- Actual callback metrics and an independently bracketed 6 ms UI delay are
  displayed separately; the UI delay leaves audio measurements unchanged.
- The prior issue 48 control/publication/conversion wait tests, output-format
  matrix, backend-error tests, reader contention, coherence and actual IPC
  coverage remain in the full suite.

Platform: local Linux aarch64, hardware-free CPAL callback and real egui/file
fixtures. No physical audio device, MIDI controller, loopback latency, or
hardware XRUN claim is made. The scope follows the distinction between audio
processing and UI work in the [Ableton resource reference](https://www.ableton.com/en/live-manual/12/computer-audio-resources-and-strategies/),
without claiming equivalent vendor metering. CPAL timestamp meaning is defined
by [`OutputStreamTimestamp`](https://docs.rs/cpal/0.15.3/cpal/struct.OutputStreamTimestamp.html).

Optional comparative benchmark (`compare_optional_sampled_profiling_cpu_cost`,
explicitly run): seven alternating trials of 32,768 stereo frames, 64 held
voices and 16 drive slots measured median calling-thread CPU of 36.607 ms with
profiling off and 36.706 ms with it on (+0.27%). This compares the optional
sampler in this local optimized development build, not the instrumentation
against an old source revision or physical-device deadline behavior.

Final local validation: full `cargo test` passed 375 tests with 4 ignored
benchmarks (the new comparative benchmark was run explicitly above), production
`cargo build` and `git diff --check` passed. The final targeted rerun passed
all seven diagnostics groups, including clicking open the actual track/device
table and matching its painted timing/percentage values to real callback data.
A separate read-only peer review found no material blocker.
