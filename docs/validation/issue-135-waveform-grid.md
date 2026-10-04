# Issue #135: deck waveform grids and output positions

Deck waveforms place saved beats, downbeats, named/color cues and saved loop boundaries around a centered playhead. Four-beat bars and eight-bar phrases start at the saved downbeat, including negative pickup coordinates. Peaks and markers share the saved local-tempo beat coordinate. Tracks without a saved grid retain a source-time window and explicit missing-grid status.

Link zoom shares the selected deck's 2, 4, 8 or 16-bar span; unlink exposes independent deck choices. The phase display shows the other deck's signed nearest-beat offset from the selected reference. Native projects preserve these view settings. Legacy projects use linked four-bar defaults; unsupported or inconsistent view data is rejected.

A fixed 65,536-frame, 2 MiB history is prepared before streaming. The actual output callback records both deck source positions and media identities after each frame, without allocating or locking. Backend scheduling estimates choose a retained output frame, preserving actual renderer motion through mapped tempos, loops, seeks and scratches. Reader overlap, unknown timing, future/overwritten frames, clock discontinuities, stream replacement and media mismatches refuse an estimate. The waveform then visibly labels renderer position. CPAL supplies its reported scheduling interval. JACK/PipeWire playback ranges are read on the owner; disconnected, ranged or conflicting paths do not invent a common latency.

These are nominal rendered source positions with backend scheduling estimates. They do not measure acoustic output or pitch-lock transient delay. No physical device, DAC, screen-reader user experience or universal real-time guarantee is claimed.

## Qualification

Frozen source: `86363664e3916e20b4e0e095830b565322af8a8d`. All 569 manifested inputs validate. The immutable 34-file package records this revision with `source_tree_modified: false`; embedded notices and actual AArch64 ELF verify.

- 1,470 ordinary checks pass: 1,469 in the main run (291.491 seconds) and the exhaustive scene-boundary check separately (261.760 seconds). Thirty-five opt-in checks are outside that count.
- 666 selected optimized checks pass in 78.885 seconds. The separate optimized two-deck keylock/recording regression also passes, for 667 unique optimized checks.
- Eight package fixtures and six performance-validator fixtures pass.
- Actual converted callback PCM clicks agree with retained output positions across 44.1/48/96 kHz source/output combinations and a tempo anchor, with output peaks within four frames of the oracle. Full callbacks perform zero Rust heap operations. Additional callback checks exercise queued reverse scratching, loop wrap, seek, oversized conversion chunks and clock retirement.
- Actual egui shapes verify downbeat, bar, cue, loop and fixed-playhead coordinates across a tempo anchor. Native zoom actions keep a one-hour track playing at both reference window sizes. A 24-hour/96 kHz arithmetic fixture retains one-frame movement. Project Save/New/Open preserves independent zoom and rejects inconsistent linked settings without replacing the saved file.
- Debug and optimized executables pass real private JACK2 1.9.22 and PipeWire 1.6.9 graph/reconnect checks, including available retained output timing. All four processes exit zero and all 12 measured Rust callback lifetimes have zero allocations/frees. C-library allocation is not intercepted.
- Private native AT-SPI exercises 175 actions and 276 nodes, including linked and independent zoom: 1,803 debug App frames and 1,307 optimized frames. Its bounded workflow allowance increases from 70 to 90 seconds for the added actions; workload performance budgets are unchanged.
- The complete CPU-6 release gate passes eight workloads with three fresh sessions each and the reviewed audio/state goldens. It runs from 10:01:58.533231 to 10:05:09.315321 UTC on 2026-10-04. Long-note recording frame P99/max values are 4.123, 1.347 and 1.287 ms, within the existing 16.67 ms P99 and 50 ms maximum budgets. Recorded host load averages are 10.07 before and 4.20 after; this is a local policy pass, not an isolated-machine measurement.

## Retained failures and diagnosis

The first complete gate fails long-note recording frame-time limits: P99 values 28.901, 17.489 and 70.874 ms. Recording correctness, reopen, command and allocation checks pass. Its full raw report and host conditions remain retained.

A separate test-only CPU diagnostic runs the same recording/save/reopen flow in three fresh sessions on CPU 6. Frame CPU P99/max is 1.241–1.273 ms; wall P99/max is 1.297–3.845 ms. Its source patch and distinct executable are retained separately. It does not reproduce the earlier stalls or establish their precise cause; it is not the qualified artifact.

A second gate attempt is invalid: Cargo reports the shared test artifact fresh after a diagnostic worktree overwrites its executable. Its SHA identifies the diagnostic, and the validator refuses its extra sample fields. This attempt is excluded. A package-scoped release clean and explicit rebuild reproduce both original qualified hashes byte for byte before the final complete passing gate. No workload budgets are relaxed.

Evidence: `/var/tmp/omatainer-issue-135-complete` and `/home/michael/Projects/omatainer-work/issue-135-complete-*`. Diagnostic data and source remain under the corresponding `issue-135-diagnostic-*` paths.

| Artifact | SHA-256 |
| --- | --- |
| Debug tests | `92fdb030a14491a864987de5c52a8808fd39dcd3032010e2daac778f7d79248a` |
| Optimized tests | `3553bd96bd555257ed58b63c6269e9926962b61c11d8a534161a448529bf1c9a` |
| Production | `bc6aaa6bfdedbcb37e5318673dd7b2b776d1eb317f94a8d054751a4cedeea9a8` |
| Separate CPU diagnostic | `08b406c64e83f64a935360d249d2b42c255fae6ae2339a5174d9479c9880ac1c` |
