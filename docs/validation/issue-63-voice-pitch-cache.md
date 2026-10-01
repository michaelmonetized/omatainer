# Issue 63: cache oscillator pitch between changes

Each voice stores its fundamental and fixed 0.997-detuned phase increments.
Construction, note/retrigger, held-note transposition, A4 tuning and sample-rate
changes refresh those values. Steady rendering performs no pitch exponential or
detune multiplication. Unchanged/clamped note edits and unchanged tuning do not
refresh the cache. Engine rate preparation still rebuilds voices according to the
existing stopped-output policy; direct Voice callers also refresh when their
sample-rate argument changes.

The internal `set_tuning_hz` hooks validate finite A4 tuning from 20 through
20,000 Hz. They preserve phase, envelope, instrument and input ownership; tuning
is retained when an individual voice adopts a new instrument or the pool is
prepared at a new rate. This introduces no user-facing tuning control or MIDI
pitch-bend support. Note changes must go through `trig` or `set_note`.

## Numerical validation

The cached expression retains the former f32 arithmetic order. Tests compare an
independent legacy oscillator/envelope/filter renderer, which deliberately
recomputes pitch each sample:

- Waveform and both oscillator phases are **bit-identical** (zero numerical
  tolerance) for MIDI notes 0, 12, 36, 57, 69, 84, 108 and 127, all three synth
  identities, and 44.1/48/96 kHz. Comparisons include 432 Hz tuning changes and
  direct held-voice rate changes to 88.2 kHz.
- All 128 MIDI notes at 44.1/48/96 kHz have bit-identical expected cached
  increments. Frequency reconstructed from phase wraps over one second differs
  from the f32 reference by **less than 0.02 Hz**. A per-voice test counter proves
  that steady samples perform no pitch-cache refreshes.
- Held octave changes, same-pitch retriggers, clamped no-ops, actual sampler
  instrument/octave commands, validated tuning and engine rate preparation keep
  pitch current without resetting held phase/envelope/instrument identity.

## Comparative local microbenchmark

Run with:

```sh
cargo test --release dense_polyphony_pitch_cache_benchmark -- --ignored --nocapture
```

Environment: aarch64, rustc 1.98.0 (88d9e12ae, 2026-08-18), Cargo release profile.
Each trial renders 64 active voices, all three instruments, for 48,000 frames at
48 kHz. Both paths include the same envelope, oscillator and SVF work; construction
is outside the timed region. The reference evaluates the original pitch formula
per sample. Inputs are opaque to the optimizer on both paths, and complete output
checksums must be bit-identical. One warm-up runs before nine alternating-order
pairs. Timings are elapsed wall time around rendering, not a thread-CPU counter.

| Path | Median | Observed range |
| --- | --- | --- |
| Cached | 72.989 ms | 72.735–73.376 ms |
| Legacy per-sample calculation | 113.886 ms | 113.311–114.329 ms |

The cached median was 35.91% lower in this local fixture. This is not a whole-app
callback measurement, hardware stream test, deadline guarantee or XRUN claim.
No performance threshold is used as a timing-sensitive automated assertion.

## Validation result

- Full `cargo test`: 283 passed, zero failed, one ignored benchmark.
- Release deterministic pitch-cache tests: four passed.
- The ignored comparative release benchmark was run explicitly and passed.
- `cargo build --release` and `git diff --check` passed.
- Independent read-only review found no blocker. Physical hardware QA remains
  outside this local DSP validation.

The note field is private with a read-only accessor, enforcing cache invalidation
through the trigger/setter APIs. All external inspection sites use the accessor.
