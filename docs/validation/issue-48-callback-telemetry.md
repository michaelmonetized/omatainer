# Issue #48 — separate render CPU and total callback timing

The audio callback now measures wall elapsed time from its entry through command
handoff/application, DSP, scratch/channel conversion, snapshot handoff, and
output sample conversion. This includes scheduling delays and waits. The
measurement ends immediately before the fixed atomic telemetry update; it does
not include asynchronous snapshot-worker work or host work after CPAL returns.

Render CPU is a separate measurement of the rendering thread's actual consumed
CPU (`CLOCK_THREAD_CPUTIME_ID`) around DSP, excluding command handling,
publication and conversion. Its fraction is CPU nanoseconds / buffer-duration
nanoseconds. Clock failure, an empty buffer, and no completed measurement are
reported as unavailable, never replaced by wall time or a fabricated zero CPU
reading. The legacy snapshot `cpu` alias is nullable and now has this actual
render-CPU meaning. The structured `audio` telemetry is the preferred interface.

The nominal deadline budget is frames / output sample rate. `deadline_overruns`
counts callbacks whose total elapsed time exceeds that budget; last and maximum
elapsed/overrun durations are exposed too. This is a service-time overrun count,
not proof of a hardware dropout. Pinned CPAL 0.15 exposes device-loss and generic
backend stream errors but no exact dropped-buffer count, so `dropped_buffers`
is null and the UI explicitly says unavailable. Backend errors and device-loss
events have independent counters; the error handler no longer synchronously
formats/prints errors.

The live GUI and bounded IPC status path read the same last-completed telemetry
without waiting for the periodic state publication. The callback writes fixed
atomics only. A reader makes one bounded sequence check; if it overlaps a write,
the measurement is unavailable for that read while monotonic event counters
remain available. No telemetry lock, allocation, formatting or retry loop is
introduced on the callback. Existing scratch-buffer growth and command/media
retirement are separate issues; this change does not claim to remove them.

Validation (local Linux, private Cargo target):

- `cargo test --offline`: **272 passed**.
- `cargo build --offline`: passed.
- `git diff --check`: passed.
- The actual extracted CPAL callback receives test-only delays of 15 ms during
  control handling, 20 ms in publication, and 10 ms before output conversion.
  Its completed measurement includes all 45 ms and reports the resulting
  overrun against a 10 ms buffer budget. These waits are excluded from CPU.
- A separate 30 ms wait *inside* the measured render region proves that the CPU
  clock does not mistake render wall time for consumed thread CPU.
- Correct frame budgets and all f32/i16/u16 conversions are exercised at
  44.1/48/96 kHz with mono, stereo and six-channel output.
- Counting-allocator evidence: 60 warmed production callbacks plus metric reads
  cross periodic publication boundaries while the public snapshot reader holds
  its mutex; **zero allocations and zero frees** on the calling thread.
- 100,000 concurrent writer updates verify coherent completed samples and
  monotonic counters. Borrowed backend errors update counters with zero heap
  allocation/free and do not invent a dropped-buffer number.
- A real UnixStream IPC status request sees a just-completed callback before the
  next periodic snapshot. Headless egui output independently asserts visible
  labels for actual render CPU, callback elapsed/budget, overruns, errors and
  unavailable drops.

This is deterministic synthetic/headless application evidence. No physical
controller, sound device, backend-dropout or live-performance measurement is
claimed.
