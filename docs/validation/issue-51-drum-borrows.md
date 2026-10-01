# Issue 51: borrowed drum banks in the sample loop

The renderer borrows each track's immutable sample bank beside its mutable
fixed voice array. Inactive slots skip sample lookup entirely. The loop performs
no Arc clone/drop: source inspection shows zero reference-count operations in
production tick_drums, versus six clones and six drops per call before. Two idle
drum tracks at 48 kHz therefore no longer perform 576,000 clones/drops each second.

The renderer exclusively owns these fields. A borrow cannot outlive the tick,
and sample-rate/bank reconstruction clears existing voice slots before later
rendering. This preserves the existing deterministic replacement policy without
moving decoding, bank creation or destruction into the warmed sample loop.

Validation on issue49 prerequisite:

- All 218 Rust tests and production build pass locally; three new test groups.
- The previous loop is retained only as a test oracle/benchmark. Every output
  sample, position and retirement matches it at 44.1/48/96 kHz with 0/1/6/16 voices;
  active scenarios require nonzero energy. Clip-owned gain remains intact.
- Idle and 16-voice warmed rendering allocate/free nothing. A unique private
  sample bank is fully retired by a real sample-rate change; old weak handles
  expire, voice slots clear and new hits use the replacement bank safely.
- Nine alternating runs of 100,000 frames measure the old and borrowed loops
  after warmup, with enough sample data that active cases cannot become idle.
  Median local headless wall times (dev opt-level 1, one test thread):

| Voices | Previous loop | Borrowed loop |
| --- | --- | --- |
| 0 | 2.375ms | 0.831ms |
| 1 | 2.408ms | 1.119ms |
| 16 | 6.623ms | 5.555ms |

These timings describe this isolated loop, not desktop FPS, complete callback
latency or a physical audio-device deadline. Timing improvements are reported,
not used as fragile pass/fail thresholds. Peer review found no remaining blocker;
`git diff --check` passes.
