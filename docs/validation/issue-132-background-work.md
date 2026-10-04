# Issue #132 background worker qualification

Frozen implementation and deterministic recovery fixture: `091fe2994330bb83a70b7b4e88f6abdcd9fbb9d5`. Linux
`aarch64-unknown-linux-gnu`, Rust/Cargo 1.98, local native builds. This layer is
stacked on #503 and addresses #132.

## Behavior

The existing expensive media workflows now share bounded admission, stable
request IDs, cancellation, declared memory reservations and measured progress.
The native Setup window exposes actual queued/running requests and cancels the
captured identity. Worker admission never enters the audio callback. See the
[workflow limits](../background-jobs.md) for policy and publication boundaries.

Equivalent deck/analysis requests reuse one pending or active decode. Cancellation,
supersession, an independent deck eject, changed media and project replacement
prevent stale preparation from replacing a newer deck. An oversized declared WAV
is refused before PCM decoding/allocation. Full queue refusal preserves an
already active source identity. Retired records cannot pin or cancel reused
optional-work flags.

## Qualification

- 1,447 ordinary checks pass: 1,446 serial checks in 294.83 seconds plus the direct
  exhaustive scene-boundary check in 260.15 seconds. Thirty-three opt-in checks
  are excluded from that ordinary count. Both direct processes returned exit 0.
- 326 unique optimized checks pass in 45.46 seconds, including real native UI
  cancellation of indexing with prior crate/playing media preserved, provider
  request cancellation, source-bound analysis, MIDI/project archive workflows,
  and video render/save/reopen through the shipped UI.
- The retained recovery action regression passes three additional isolated
  optimized runs. Its fixture now captures the two deliberate references before
  starting automatic journaling; stale-action and exact-restore assertions remain.
- Eight source/package validator fixtures pass in 5.628 seconds.
- Three CPU-6 optimized background-pressure runs process 768 actual output
  callbacks each: 128 frames, 48 kHz stereo, 2.667 ms deadline. They exercise the
  production file decoder, admitted file-read/hash pressure, cancellation,
  supersession and full queue/memory refusal. All 2,304 measured callbacks have
  zero deadline overruns and zero heap allocations/frees. Worst measured wall
  time is 0.671 ms; worst p99 is 0.491 ms. Every run completes three current
  decodes, records both capacity refusals and retains the same playing source.
  Audio SHA-256 matches in all three runs:
  `72d69eab54aed8fb4926b379621c7ddf3e581dbf494d5ea6af36d6be58873203`.
- Actual owned Linux worker CPU/scheduling/I/O policy is observed, and the parent
  thread remains unchanged. Reservations are admission accounting, not a process
  RSS or operating-system memory cap. I/O scheduling depends on the kernel scheduler.
- The actual production executable refuses invalid deck targets and extra CLI
  override arguments before opening IPC. Its embedded source/license records match.

- The final CPU-6 release gate passes all eight workloads with three repeats,
  unchanged reviewed budgets/audio hashes and zero measured callback heap work.
  It runs from `2026-10-04T04:33:56.949716+00:00` to `2026-10-04T04:36:49.501000+00:00`.
  Private native AT-SPI exercises 158 actions, 271 nodes and 588 App
  frames. The first gate failed wall-clock limits during a separate Rust build;
  its complete report is retained as `issue-132-final-contended-gate.json`.
  The successful retry uses the same source, binaries, affinity and budgets.
- All 555 manifested source/build/gate files, actual embedded records, the
  immutable package and its actual executable verify. Packaged source revision
  is `091fe2994330bb83a70b7b4e88f6abdcd9fbb9d5` with `source_tree_modified=false`.

The pressure check uses native software callbacks and private local fixtures.
It does not prove physical backend dropout counts, hardware compatibility,
uncached disk latency or a whole-process memory bound. Existing hardware/backend
receipts belong to their own frozen sources. Job monitoring is transient; durable
state and actual application remain owned by each workflow's existing receipts.

## Retained artifacts

The immutable binaries and package are under
`/var/tmp/omatainer-issue-132-final`. Compiler-artifact records, direct-process
receipts, optimized selection, repeated-pressure reports, gate/package logs and
source-freeze receipts are adjacent to the worktrees with `issue-132-final` names.

- Production: `f9b7aaa8f204bb938d45a677b789941bbf12ec1d9beaa4971f9b58ea866a9041`
- Optimized tests: `ab306783c1dbfae17fa9cea950e062b65ceeffbe488e9ffe3795cfc92e7500f0`
- Debug tests: `69a9d3b169b9e293676ffc9bdffc4fa41833408005d78303173ce47dddf9414b`

Validation prose added after the frozen implementation is outside the manifested
source inventory. No hosted Rust qualification is claimed.
