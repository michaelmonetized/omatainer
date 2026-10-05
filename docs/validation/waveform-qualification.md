# Waveform and consolidated native qualification

Frozen source `096dff237e5351e22221ab2033d1919ad53338c9` passes **1,602 ordinary tests**, the optimized eight-workload gate and fresh native JACK/PipeWire graph/settings checks. All 615 source hashes remain unchanged. The [receipt](waveform-qualification.json) binds the results to the source, compiler artifacts and retained binaries. This qualifies the local software checkpoint; the remaining backlog and draft PR are incomplete.

The ordinary run covers every one of the 1,602 non-ignored test names exactly, audited against the executable's listing. File fixtures pass 1,551 tests in 311.37 seconds; socket fixtures pass 50 in 10.34 seconds; the isolated exhaustive scene test passes in 259.27 seconds. Thirty-six opt-in tests are excluded from that count. Socket fixtures use a short real directory under the Omatainer project because the complete worktree path exceeds the Unix socket limit. A symlink alias is deliberately refused by runtime isolation checks.

Fresh compiler metadata records `fresh: false` for debug tests, optimized tests and production. Their SHA-256 values are:

| Artifact | SHA-256 |
| --- | --- |
| Debug tests | `48ba10c99081b0055454167fa418e740e6d8f4d38058ac777e6f4039ffe61819` |
| Optimized tests | `6b8264d7764fc0a0a69bc6d0e7fa1e77133a2a6ad2e6146a4bb32d7fddaae23c` |
| Production | `2595676599baeedf217b528832ea812bde6fa2fa598ed8d9d14faa5e6a294e8c` |

The unchanged optimized policy passes all eight workloads with three fresh sessions each, normal CPU affinity 0–9, `schedutil`, ordinary scheduling and nice 0 on the Apple M1 Pro Linux host. The actual App/renderer and native AT-SPI API preflight passes 166 actions, 278 visited nodes and 1,604 App frames. It has no physical window or screen-reader session. No budget, golden hash or policy was relaxed.

The complete CPU-6 attempts remain failed: Composer callback wall maxima were **17.230 ms** and **13.771 ms** against a 10.667 ms limit, while render CPU maxima were 0.808 and 0.888 ms. The first overlapped other tests; the second did not. These measurements do not establish a particular cause. Normal-affinity acceptance does not claim single-core or hard real-time qualification. Two additional attempts ended before benchmarks because fixture IPC paths were too long; neither is counted as passing.

Three additional optimized runs exercise two full 64-anchor maps, 8,192 notes and 4,096 callbacks per buffer size. These use actual timestamped OutputCallback rendering and its position-history writes. Every run has finite, nonzero audio, zero Rust heap allocations/frees and render CPU p99 below 75% of the buffer period. All observed wall maxima are below their periods:

| Frames at 48 kHz | Buffer period | Render CPU p99, three runs | Callback wall maximum, three runs |
| --- | --- | --- | --- |
| 128 | 2.667 ms | 0.453–0.454 ms | 0.588 / 0.683 / 2.422 ms |
| 256 | 5.333 ms | 0.899–0.902 ms | 1.458 / 1.036 / 1.035 ms |
| 1,024 | 21.333 ms | 3.554–3.576 ms | 3.818 / 4.921 / 5.907 ms |

Both retained test binaries pass the private JACK 1.9.22 and signed PipeWire 1.6.9 graph adapter and actual settings workflows: eight fresh runs total. They verify exact route reopen/return, duplicate and direction refusal, 128→256 quantum changes, stopped reconnect at 44.1 kHz and retained project media. Save and Cancel retain stream generation; reviewed Apply changes it. Rust callback allocations/frees remain zero. Dummy/private servers do not prove C/C++ heap behavior, hardware timing or uninterrupted physical capture.

An intermediate shutdown change caused a native PipeWire abort in `pw_memmap_free` through `jack_client_close`. The fix restores explicit port unregistering and fences latency readers before retiring ports. The concurrent regression test refuses late callbacks and waits for admitted readers. Both native backends and the complete suite pass after that fix. The failed source, core backtrace and attempted qualifications remain retained locally.

Waveform tests cover actual irregular click PCM, reverse/piecewise-tempo output, stale/expired/overwritten history, timestamp jitter, long media, grid/second labels, cue/loop markers, linked views and preference persistence. Renderer fallback is explicit where key-lock grains or outgoing fades lack one source position. Effect tails and physical display/converter alignment remain unqualified.

[Routed input evidence](input-integrity-consolidation.md) still records missing frames at 128 and 512 frames and correct refusal to publish incomplete WAVs. This checkpoint's full suite includes the input-integrity changes, but does not turn those failed captures into uninterrupted recording acceptance. Pioneer/Numark/Akai/keyboard USB input, external audio, suspend and listening remain pending. The [earlier 32-anchor qualification](remaining-backlog-qualification.md) retains its own source binding and observed limits.
