# Local release workload gate

The release packager and transactional installer require a passing
`target/performance.json` for the exact binary, source inventory, lockfile,
license records, target, toolchain and reviewed `benchmarks/policy.json`.
The inventory includes Rust and embedded assets, test fixtures, scripts, checked
manual/README/contract and every benchmark input, including unexpected additions.
A missing, stale, malformed or failing report stops publication before install
mutations. Package verification recomputes the report from retained raw samples;
it does not need Cargo, rustc, audio hardware or a running application.

The fixed policy is reviewed for `aarch64-unknown-linux-gnu`. The baseline audio
hashes are identical across three fresh sessions with exact note-state checks.
Another target needs its own reviewed evidence; no synthetic test record qualifies
a release. The complete assembled source must still produce a fresh passing report.

## Reproduce on a local Linux host

After reviewing policy and refreshing the source inventory:

```sh
python3 scripts/license-manifest.py update
python3 scripts/performance-gate.py run
python3 scripts/performance-gate.py check
python3 scripts/license-manifest.py package --destination /path/to/new-package
python3 scripts/license-manifest.py verify-package --destination /path/to/new-package
```

When using a private Cargo target, pass the actual production artifact with
`--binary /path/to/target/release/omatainer`. The report defaults to the source
checkout's `target/performance.json`; packaging and installation use this same
location. An explicit `--report` is useful for private investigations; copy it to
the default path only after checking it against the intended binary and source.

The runner performs controlled local `cargo build` and `cargo test --no-run`
with `--release --locked --offline`, then executes the resulting test binary.
It rejects CI/GitHub Actions execution, unreviewed Cargo configuration and Rust
build overrides. Cargo home, target directory and terminal presentation options
are permitted; optimization overrides are not. Both the Cargo artifact profile
and the production binary's early `benchmark-build-info` response must identify
an optimized release build without debug assertions. No audio device is opened
by that identity command. The benchmark executable embeds the exact current
license manifest as well: a stale executable reused from another checkout fails
even if Cargo reported a plausible artifact path.

The native accessibility preflight runs the production App through AccessKit
and Linux AT-SPI on a private D-Bus session. Its dependencies, numeric action,
focus/click delivery, project persistence and preferences persistence are
required. Missing dependencies or a failed preflight stop the run; there is no
skip flag. This is native API evidence, not an Orca or user-testing claim.

## Workloads and report semantics

The fixed suite contains producer, composer, live DJ and hybrid callback
sessions; a 50,000-entry crate; five synthetic controller inputs plus concurrent
IPC; an 8,192-note project Save/New/Open round trip; and a 600-note, 600-second
virtual MIDI recording followed by save/reopen. Each workload starts fresh for
three repetitions. The private production IPC server handles sixteen connections
of 32 correlated requests each. Held chords verify distinct stable source IDs,
channel/pitch ownership and release; crate selection verifies the exact selected
and next-frame published sources. Callback capture verifies all 512 added notes
and all original notes, including pitch, velocity, position and held duration.
The synthetic names exercise the available DDJ-FLX4, NS7, APC40 mkII, MPD232
and generic keyboard profile paths. Upper unmapped MPD notes 60–83 supply the
held chords; this does not identify or qualify the user’s physical Pioneer model.
Conditions, scalar counters, exact state checks and target
qualified quantized-audio hashes belong to the reviewed policy. Dense track
parameter changes and live MIDI capture exercise the currently implemented
commands. They do not stand in for an absent automation-lane editor or general
audio-file recording workflow.

Each timing distribution retains every integer nanosecond sample. The Python
validator independently sorts each repetition and recomputes nearest-rank
p50/p95/p99, minimum, maximum, max-minus-min spread and p99-minus-p50 jitter.
Scalar bounds cover allocations, frees, admission failures, event counts,
rendered energy/peak and operation completion times. Missing samples, extra
fields, incorrect counts, nonfinite numbers, failed state checks, changed audio
hashes and forged summaries fail closed. No average conceals a failed repetition.

Callback timing measures production callback work driven without an audio
device. Render CPU uses the callback thread CPU clock. UI wall time covers the
actual offscreen App update and egui tessellation, excluding snapshot-worker
waits and GPU/compositor presentation. Long recording uses real renderer blocks
in accelerated virtual time. These are local work budgets, not proof of physical
audio deadlines, XRUN freedom, converter latency, FPS, audible quality or the
user's Pioneer/Numark/Akai hardware behavior. The report retains these limits
and host CPU, kernel, memory, affinity, governor, process priority/scheduler and
before/after workload load averages. Run qualification after stopping concurrent
compilation and unrelated test suites; any budget miss still rejects the report.

## Retention and resource bounds

Starting the controlled build replaces an older eligible report with an
incomplete attempt. Successful completion writes an atomic, fsynced report;
failed workload checks retain a failing report. The `.raw.json` sidecar retains
available raw output before parsing, including invalid evidence; `.log` retains
bounded process diagnostics. Failures before raw output exists retain the
incomplete report and available log, never a fabricated pass.

Reports/raw input are capped at 64 MiB and retained policy text at 128 KiB.
Build, test compilation and workload execution each have the policy's finite
900-second deadline and 16 MiB output budget; the native preflight has a
100-second deadline and 1 MiB output budget. Process failures terminate the
owned process group with a five-second TERM grace period and bounded KILL/wait.
No GUI or audio callback performs these operations.

Each immutable package and successful installation retains the validated report
at `.local/share/omatainer/validation/performance.json`, alongside license
records and the artifact hash receipt. The transaction journal includes this
file, so rollback restores the previous installation's evidence too. Standalone
package verification and transaction recovery still work without rustc.

Validator/package/installer regression fixtures use clearly labelled synthetic
numbers to test rejection and rollback. They cannot establish performance or
native accessibility results; release evidence comes only from the actual
local runner and production component workloads.

## Regressions found by the workload

The combined capture/fader workload exposed a history journal that protected the
first prior note inverse for a cell rather than each actually held note's inverse.
After enough short captures, this pinned an old entry and rejected later edits.
Held notes now retain their exact journal owner; overlapping-source duration updates
also reach later inverse snapshots and replay patches. Tests cover capacity,
overlapping ownership, rate pruning and exact Undo/Redo duration with zero callback
allocations or frees.

The held-chord workload exposed voice allocation stealing zero-level new attacks
while older release tails remained. Saturated voice pools now prefer released tails
before active holds. A fixed-pool regression verifies independent sources plus clip
onsets and exact releases without heap work. These fixes preserve the workload's
original zero-allocation and zero-rejection requirements.

The first assembled release attempt passed all functional/audio state checks but
failed composer wall-time limits during a concurrent local Rust test suite. Its
maximum reached 17.773 ms against the unchanged 10.667 ms budget; one repetition's
p99 reached 5.481 ms against 5.333 ms. That attempt remains failing evidence and is
not qualified. Follow-up qualification pauses our other build/test jobs, records
load and keeps all allocation, deadline, jitter and UI limits unchanged. This
separates controlled host capacity from a guarantee under arbitrary competing work.

## Qualified assembled run

The complete ordered stack passes **673 ordinary Rust tests** (12 opt-in fixtures
ignored by that command). The controlled release gate then passed its required
private native AT-SPI preflight and all eight workload groups, each with three
fresh sessions. Workload execution took 131.81 seconds.

Host: Apple MacBook Pro 16-inch M1 Pro (2021), Linux aarch64, kernel
`7.1.13-3-2-ARCH`, 10 logical CPUs, 16,141,549,568 bytes RAM, `schedutil`,
affinity CPUs 0–9, ordinary scheduler policy 0 / nice 0. Our other builds and
test suites were paused. One-minute host load was 2.681 before workloads and
1.645 afterward; this is a controlled local capacity run.

Worst p99 and maximum across the three sessions, in milliseconds:

| Workload | Timed path | p99 | Maximum |
| --- | --- | ---: | ---: |
| callback_producer | callback_wall | 2.292 | 6.063 |
| callback_producer | render_cpu | 0.681 | 0.705 |
| callback_composer | callback_wall | 2.502 | 6.939 |
| callback_composer | render_cpu | 0.690 | 0.733 |
| callback_live_dj | callback_wall | 0.462 | 1.035 |
| callback_live_dj | render_cpu | 0.060 | 0.072 |
| callback_hybrid | callback_wall | 2.829 | 3.180 |
| callback_hybrid | render_cpu | 0.698 | 0.711 |
| large_crate_ui | frame_wall | 3.492 | 4.953 |
| multi_controller_ipc | frame_wall | 4.654 | 6.656 |
| multi_controller_ipc | ipc_roundtrip | 9.460 | 10.178 |
| multi_controller_ipc | midi_dispatch | 3.176 | 5.342 |
| project_roundtrip | frame_wall | 3.704 | 3.704 |
| long_note_recording | frame_wall | 6.942 | 6.942 |
| long_note_recording | renderer_wall | 2.389 | 3.937 |

Callback wall budgets retain p99 within one block and maximum within two blocks;
render CPU p99 remains below 75% of the block duration. The 128-frame DJ block
and 256-frame other blocks are at 48 kHz. All callback and long-recorder measured
allocations/frees are zero. No workload rejected a command or lost a MIDI event.

The release preflight visited 230 native nodes, persisted UI scale 1.25 and saved/
reopened the recorded project note. It drove the production App via native APIs,
without opening an audio device, desktop window or physical controller.

Both standalone package verification and the independent source-bound report
check passed against the production release binary. Validator, package and
installer regression suites passed 6, 8 and 21 groups respectively. Packages
retain the full raw samples, recomputed summaries and conditions at the documented
validation path. The passing report does not convert the prior contention failure
into a pass or establish physical audio/hardware performance.

Production binary SHA-256: `79b4fa29f4fe436134f8ef301b6d6c1ea235f99213e1a768858bfc0a3ad48548`.
