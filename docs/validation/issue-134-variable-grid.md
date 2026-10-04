# Issue #134: manual local tempo maps

Tempo anchors join constant-tempo source segments with a continuous beat coordinate. The initial downbeat and tempo are followed by at most 64 ordered anchors. Every position is finite and every segment stays between 20 and 400 BPM. Distinct source positions whose floating-point beat boundaries collapse are refused. Inserting sorts by source time; replacing an exact position updates its local tempo; deleting extends the previous segment and recomputes later coordinates. Original decoded audio, analyzed BPM and absolute cue positions remain unchanged.

Deck grid → Tempo anchors exposes named source-seconds/BPM entries, Use playhead, Insert/update and exact-identity Delete actions. Draft changes do not reach playback until the existing receipt-qualified Apply. Cancel, source replacement, invalid values, protection and history admission retain the prior applied map. Applying a complete map is one history entry. Slip shifts the downbeat and every anchor. Global stretch/half/double scales all local tempos while retaining source anchor positions; every segment must stay in range. Meter remains four beats per bar.

Loops use mapped beat-to-source conversion across boundaries, including quantized in/out, reloop, half and double. Match reads local tempo and continuous phase. Its target tempo is captured at the action, as in the existing Match workflow; it does not create a live master-follow link. A synchronized track with tempo anchors advances by that target's beat clock and converts back to source frames, including beat-coordinate loop overshoot. Scratch retains direct source movement. Source files are not rewritten.

Preparation uses bounded fixed atomic words, and existing library identity, reanalysis, native project and recovery paths carry the entire map. Legacy uniform grids deserialize with an empty anchor list. The native JACK crash was traced to constructing the enlarged inline command batch on its callback stack. The renderer now retains that fixed batch before streaming. History inverse arrays now come from 476 reusable buffers prepared before streaming. The retirement worker clears and returns them, and stopped rate pruning keeps the same buffers for held inverses. This prevents enlarged map values from turning every inline transaction transfer into a large stack copy. The renderer refuses creative admission when prepared capacity is exhausted; it never allocates a replacement buffer in the callback. Existing retirement, shutdown and history limits remain active.

## Fixture provenance

Generated ramping clicks and deterministic humanized timings provide exact timing oracles. Separately, an unchanged human-played electronic drum performance from Google's Groove MIDI Dataset is retained with its unchanged MIDI timing record, attribution, CC BY 4.0 license and member hashes in tests/fixtures/beatgrid. Its measured performed intervals vary around the original 92 BPM metronome. Tests use nearby performed kick/snare timings as quarter-beat anchors and exercise the actual recorded PCM, rather than generating a replacement sound. It is a Roland TD-11 performance, not an acoustic recording. Archive members passed ZIP CRC validation after range retrieval; the full archive checksum was not measured.

## Checks and limits

Frozen source: `2c0cc7ad6a2b66b172049873f010cd6af5398f82`. All 565 manifested inputs validate; the package records this exact revision with `source_tree_modified: false`.

- 1,460 ordinary checks pass: 1,459 in the main run (293.918 seconds) and the complete scene-boundary check separately (264.935 seconds). Thirty-five opt-in checks are omitted from that ordinary count.
- 576 selected optimized checks pass in 62.255 seconds. The separate optimized two-deck keylock/recording regression also passes; it is one additional unique test.
- Eight package fixtures pass. The immutable 34-file package, embedded manifest/notices and actual AArch64 ELF validate.
- Debug and optimized binaries pass real private JACK2 1.9.22 and PipeWire 1.6.9 graph/reconnect checks. All four processes exit zero; all 12 measured Rust callback lifetimes have zero allocations/frees. C library allocation is not intercepted.
- Private native AT-SPI performs 169 actions and visits 271 nodes: 1,660 debug App frames and 777 optimized App frames. It exercises inserting/applying a playhead anchor, exact deletion, cancellation, persistence and other existing workflows.
- The unchanged CPU-6 release gate passes all eight workloads with three repeats, matching the reviewed audio/state goldens and existing budgets. It ran from 08:06:04.987306 to 08:11:16.352259 UTC on 2026-10-04. Another build/benchmark was active during this run; its measured host conditions are retained. This is a policy pass, not an isolated-machine or physical-device measurement.

Retained evidence: `/var/tmp/omatainer-issue-134-complete`; execution receipts and logs use `/home/michael/Projects/omatainer-work/issue-134-complete-*`. Earlier failures and the original command-batch native crash trace remain in separate folders.

| Artifact | SHA-256 |
| --- | --- |
| Debug test executable | `64af1d8fddae6e6854a3937a5edc77a30c73d03c196a1f17c79d7f95f0404333` |
| Optimized test executable | `e966a3cac0971b1aa3c90fa50506ef9873686e1eb929b71d72db4d01df18cd5b` |
| Production executable | `4f998dde79ab03dd49cb941ae3d6ae0ba62d971b0d761c0050f0dace811f45c0` |

The 64-anchor stress fixture traverses every anchor on both decks, produces finite full callbacks and performs zero renderer heap operations. Its three CPU-6 repeats after competing build/benchmark processes exited measure P50 0.169 ms, P99 3.367–3.397 ms and maximum 3.520–6.017 ms against a 2.667 ms, 128-frame/48 kHz block. This fixture has no timing acceptance threshold. It proves bounded functional behavior and zero heap work; its measured wall times do not prove deadline safety. A temporary test-only CPU diagnostic adds thread-clock measurements around the same render loop without changing product code. It reports P99 whole-callback CPU 0.175 ms and render CPU 0.174 ms. Its source patch and distinct executable hash are retained separately; it is not the qualified artifact.

After restoring the exact frozen source, three new runs of the original qualified executable measure P99 0.187–0.200 ms and maximum 0.235–0.959 ms, below the 2.667 ms block. This establishes the final local software measurement, while the earlier slower runs remain part of the evidence. The exact reason for the earlier host-time variation was not established; it must not be represented as a proven scheduler, paging or CPU-frequency diagnosis. No physical-device or universal real-time guarantee follows. Local software evidence does not prove physical output, device deadlines, controller compatibility, perceived quality, automatic detection or Serato parity.
