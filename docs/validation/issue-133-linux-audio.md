# Issue #133 Linux audio backend qualification

Frozen source: `de8fe3631c8693518c632213b1e2e54e89dd2928`, stacked on
PR #504, `stack/issue-132-background-jobs`. Native local Linux
`aarch64-unknown-linux-gnu`, Rust/Cargo 1.98.0. No hosted Rust or physical-interface
qualification is claimed.

## Delivered workflow

The Linux profile editor discovers ALSA or an existing JACK server. The latter
uses either JACK2 or PipeWire's JACK client library selected at launch. It exposes
named output/capture ports and persists exact endpoint/channel maps. Saving never
switches an output or enables input. Preview, stopped confirmation, cancellation,
rollback and explicit recovery share the existing owner and project guards.

The server owns graph rate, quantum and callback scheduling. Process buffers are
allocated before activation; notifications publish atomic state. The owner handles
link restoration and quantum display changes. Missing or temporarily inactive
endpoints remain disconnected and retry without choosing another endpoint.
Duplicate owned clients, wrong directions and known feedback paths are refused.
Rate changes/server loss retire the native stream, retain the unique renderer and
project media, and require explicit stopped reconnect. Input never resumes
automatically. Physical calibration remains the separately qualified ALSA workflow.

Native stream teardown unregisters ports before closing the client and joins the
input/output owners before application exit. This prevents renderer ownership from
being stranded after server loss and native library finalization from racing a
detached owner. See [setup, bounds and route-review limits](../linux-audio.md).

## Source-bound checks

- 1,450 ordinary checks pass: 1,449 serial checks in 320.56 seconds and the direct
  exhaustive scene-boundary check in 270.00 seconds. Thirty-five opt-in checks are
  excluded from the ordinary count. Both direct processes return exit zero.
- 401 unique optimized checks pass in 54.11 seconds, including audio ownership,
  routing, profile storage/UI, cancellation, recovery, background jobs, provider,
  video, MIDI, project, support and native command workflows.
- Eight source/package validator fixtures pass in 3.343 seconds.
- The unchanged CPU-6 release gate passes eight workloads with three repeats,
  reviewed hashes/budgets and zero measured callback heap work. It runs from
  `2026-10-04T06:20:17.621687+00:00` to
  `2026-10-04T06:23:22.640740+00:00`.
- Native AT-SPI preflight exercises 158 actions, 271 nodes and 608 App frames.
  It uses the actual App/renderer and native accessibility API without a desktop
  window, Orca or physical-device qualification.
- All 559 manifested files, actual embedded records, the immutable package and its
  executable verify. Packaged revision is the frozen source above with
  `source_tree_modified=false`. Its ELF machine is AArch64. JACK is dynamically
  loaded and absent from the executable's mandatory shared-library dependencies.

## Real backend fixtures

`scripts/check-linux-audio.py` starts and retires only owned private servers. Both
ordinary and optimized binaries run the production native graph adapter and the
actual shipped profile/audio settings controls. All eight graph/UI processes exit
zero, including native client shutdown.

Each graph run reopens saved endpoint state, leaves a missing endpoint unconnected,
creates that exact endpoint and observes one link, repeats restoration without
duplicates, and removes it again. It refuses a duplicate application client and
wrong-direction endpoint. Actual output and explicitly enabled input callbacks
observe a quiet rendered signal through the backend loopback. The private server
changes quantum from 128 to 256 without replacing the stream, then restarts from
48 kHz to 44.1 kHz. Explicit reconnect retains the same media Arc and keeps playback
stopped.

| Real backend | Channels | Debug software return | Optimized software return | Measured Rust callback allocations/frees |
| --- | --- | --- | --- | --- |
| JACK2 1.9.22, private dummy driver | 2 F32 | 2.488 ms | 2.359 ms | 0 / 0 in all three stream lifetimes |
| PipeWire JACK library/server 1.6.9, private null sink | 2 F32 | 2.512 ms | 2.534 ms | 0 / 0 in all three stream lifetimes |

Callback counts are respectively 87/28/4 and 80/24/4 for JACK debug/optimized;
50/14/29 and 51/14/27 for PipeWire debug/optimized. Counts include the reopened
output. Timing is host callback observation plus frame offset, not physical
converter latency. Rust allocator instrumentation covers the production Rust
callback body; C/C++ library allocation is not intercepted. JACK runs in
non-real-time dummy mode; no real-time scheduling or zero-xrun claim follows.

Each native settings run edits a graph endpoint through its named text control,
previews, saves and reopens Preferences. Saving preserves the output generation.
Canceling output confirmation preserves it; explicit Apply changes it. The graph
calibration action is refused before stopping the graph. These four runs exercise
58, 48, 62 and 46 actual App frames for JACK debug, PipeWire debug, JACK optimized
and PipeWire optimized respectively.

The retained CPAL 0.15.3 / ALSA 1.2.16.1 path also passes real 32-channel capture
through a private PipeWire 1.6.8 server at 48 kHz. Every channel has the expected
isolated test-signal energy in decoded 26-channel and six-channel WAVs. The debug
run overlaps compilation and records 5,760 overflow and 1,280 missing-input frames;
the optimized run after compilation records zero overflow and 4,131 missing-input
frames. These counters and silent gaps are retained, not called dropout-free or
physical interface qualification. See the existing [routing bounds](../audio-routing.md).

The 1.6.9 ARM fixture packages are privately extracted; installed audio libraries
and desktop/system configuration are untouched. All three signatures verify under
Arch Linux ARM build key `68B3537F39A313B3E574D06777193F152BDBE6A6`:

- pipewire: `d711f2ba44cf9be3800b168f1d68d7791e2838c2bbefde26deff56a26eaccae2`
- libpipewire: `21e81a38ea0217e0b0675b00c1c98bb547098f00d689201a27c34824e442d66e`
- pipewire-jack: `118a016d2cc8af0349b1dc73a6b6f64f37df13ae3023689dcc2d97d088bf31ad`

## Failure evidence and retained artifacts

Provisional runs exposed native PipeWire port-map cleanup, teardown during C
library finalization, the erroneous retention of a renderer after a disconnected
client close, and JACK port appearance before activation. The final implementation
and exit-zero reruns address each. Earlier failed logs, binaries and system cores
remain separate from final qualification. One provisional driver also selected the
host AppImage as its Python child interpreter; the driver now resolves `python3`
through PATH. No failed process is counted as a pass.

Immutable final artifacts are under `/var/tmp/omatainer-issue-133-final`, with
source-bound compiler-artifact JSON and process/gate/package receipts adjacent to
the worktrees under `issue-133-final` names. ALSA configurations, WAVs and unchanged
receipts retain their original `/var/tmp/omatainer-133-alsa-debug` and
`/var/tmp/omatainer-133-alsa-release` paths.

- Production: `689be3f5567bd1b9e490f5bc52fabc29cd3f88769f46e9b461ddef535c887898`
- Optimized tests: `70e1073530a21dd4e81cfde265f1ea8a83098e472a709a72758bca12e269e45d`
- Debug tests: `96b559380c82b50e50e3a84dd39f779a297d69066d5567f38f89cd8650d126f2`

This validation prose is added after source freeze and is outside the manifested
source inventory. Physical converters, USB removal, acoustic feedback, external
monitor paths, live studio loads and other platforms remain unqualified.

## Combined backlog integration

PR #500 retains this published layer above #505. The parent measurements above
apply to its frozen source, not to the combined branch. On the combined working
source, 286 selected ordinary checks pass, including invalid media targets, owned
renderer retirement, independent sampler-editor fixtures, project restoration,
backend configuration, saved graph endpoints, backup, performance guards,
background jobs and native UI controls. The debug executable SHA-256 is
`6ff8857b851b57837fd7c97393f833719499e3d3f27a62114061ff34d30bf5da`.
The direct receipt is `/tmp/omatainer-wrapup/merge-133-focused-verified.json`;
its source guard reports no changes. One empty worker selector is explicitly
excluded from the count. Full, native-server and optimized combined-source
qualification follows the final source freeze.

The previous frozen combined source `6fa8f507f6f1dc9640b014b337ea3bc12ccbbd69`
returned 1,562 passes and three failures. These were the changed invalid-target
refusal, its raw renderer retirement fixture, and parallel sampler fixture
capacity. The production shared sampler limits remain unchanged. Their failure
logs remain in `/tmp/omatainer-wrapup/merged-backup-ordinary.log` and are not
counted as qualification.
