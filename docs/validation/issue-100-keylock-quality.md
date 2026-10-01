# Issue 100: key-lock implementation and quality evidence

## Scope and corpus

Qualification uses the production deck renderer and real OutputCallback in a
private offline process. It does not open an audio device. CPU, software content
displacement and startup behavior are separate observations; none establishes
converter latency, backend XRUN performance or a human transparency judgment.

The corpus contains two complete recorded VocalSet 1.2 singing-vowel files,
plus original analytical stereo bass, stereo transient pulses and the complete
procedural Omatainer drums/harmony mix. The recorded files cover one soprano
and one baritone performing the same slow forte arpeggio vowel; they are a
limited musical sample. The source license is CC BY 4.0, with author/DOI
attribution, exact member names, member CRC32/SHA-256 and complete terms in
`tests/fixtures/keylock`. Selected bytes were retrieved from the authoritative
archive and verified individually; the full 6 GB archive MD5 was not checked.
The recordings are test fixtures, not factory-library or production-embedded
media. Original generated corpus material follows the application's MIT license.

The fixed matrix uses forward ratios 0.50, 0.84, 0.92, 1.00, 1.08, 1.16 and
1.50 at 44.1/48/96 kHz output, blocks 64/128/512, with lock on/off. Each run
contains 630 isolated renderer cases, 126 actual callback cases and nine
transition traces. Decode, analysis and file writes are outside timed blocks.
Timers and Rust allocator instrumentation add measured overhead. Canonical
Float32 WAVs use block 128; the other block sizes must produce exact matching
PCM. Every sample must be finite and timed sections heap-free.

`scripts/check-keylock-quality.py` accepts an explicit evidence root through
`OMATAINER_KEYLOCK_EVIDENCE_ROOT` (default: repository target/keylock-quality).
It requires fresh descendant output directories and a 2 GiB export/pack bound.
Native exports embed the compiled source/license inventory. Before and after
execution, the script validates that inventory against the reviewed tree and
checks source, executable, manifest and notices stability. A verified-run receipt
binds these checks to the report hash; comparisons require matching workload
and corpus identities. Runtime checkout fields are explicitly informational.

## Implementation and declared bounds

The replacement is original MIT Rust waveform-similarity overlap-add (WSOLA),
with one joint stereo correlation decision for both channels. It introduces no
foreign allocator, DSP dependency or proprietary algorithm. Normalized
correlation sums channel energies separately, so opposite-phase stereo and a
silent channel do not cancel the alignment reference.

At 48 kHz the window is 2,048 frames and the hop is 1,024 frames; their durations
scale with the output sample rate, with integer rounding exposed in each report.
The search radius is 25 ms. Each non-silent search has at most 100 candidate
scores, each using 128 deterministic stratified reference points. Weights are
allocated during preparation; processing and reset use fixed storage.

The first 15 ms search candidate attenuated a 20 Hz fundamental even while its
total RMS remained strong. Uniform reference/candidate spacing also missed phase
at some upper-band frequencies. The final search covers a full 20 Hz cycle and
uses stratified positions plus six bounded refinements. Regressions measure
fundamental amplitude/coherence for sub-bass and upper-band tones, alongside
mixed sample rates, stereo relationships and callback heap behavior. These
focused probes supplement the unchanged before/after musical corpus.

Supported forward ratios are 0.50–1.50. Unity uses exact direct playback;
scratch touch and unsupported rates use direct resampling with an explicit
renderer-confirmed UI mode. Release, seek, cue and load reset overlap history
through the existing bounded 2 ms transition. Valid natural loop wraps retain
overlap with cyclic source reads. Only exact unity/range-endpoint targets receive
a sample-rate-derived floating-point convergence snap; arbitrary rates and
unlocked playback retain their previous smoothing behavior.

Resident source PCM supplies look-ahead without adding an output FIFO. At
48 kHz, conservative source-time configuration bounds are 67.667 ms of look-ahead
and 46.333 ms of content displacement. These are geometry bounds, not measured
device latency. The render report separately measures transient onset/envelope
displacement and unity alignment; output-driver/converter latency remains
unmeasured by this offline protocol.

## Original implementation baseline

The baseline ran from 2026-10-01 07:49:25 to 07:50:09 UTC with other agent CPU
jobs held, on the same local M1 Pro/Linux host used by the release gate. The
preexisting arbitrary-origin overlap-add implementation was compiled with the
new qualification harness. Source identity is embedded in the retained report;
the corresponding clean checkout was `cee1f96`.

- Workload SHA-256: `fd4682051d4a83a1d900b04e205886e3ab9b1c15dca3c9076acd9fd99a5b521b`.
- Executable SHA-256: `d2d66e0a08516bd439bc7fd80baeedcb8b14bf832c41986366905f77716db498`.
- Report SHA-256: `a14686bdf3ec9d470e3b12885d3a95e545395570b5ed520551a37a2618759aaf`.
- Evidence directory: `issue-100-baseline-cee1f96` under the local work directory.

At 48 kHz/block 128, lock enabled, the analytical bass revealed failures that
the prior zero-crossing contract did not catch:

| Playback ratio | 55 Hz source dominant peak (Hz) | 93.75 Hz source dominant peak (Hz) | Projection amplitude at intended 93.75 Hz |
| --- | ---: | ---: | ---: |
| 0.50 | 27.50 | 46.90 | 0.000442 |
| 0.84 | 46.20 | 78.75 | 0.002389 |
| 0.92 | 50.60 | 86.25 | 0.006172 |
| 1.00 | 55.00 | 93.75 | 0.349959 |
| 1.08 | 59.40 | 101.25 | 0.010344 |
| 1.16 | 63.80 | 108.75 | 0.005171 |
| 1.50 | 82.50 | 46.85 | 0.001090 |

The intended tone's source amplitude is approximately 0.35. These are objective
spectral/projection results, not listener ratings. The ordinary one-deck callback
at 48 kHz/block 128 had worst CPU p99 67.08 microseconds and wall p99
72.88 microseconds across these locked ratios. All 765 measured groups had zero
Rust allocations/frees, all rendered samples were finite, all 420 cross-block
comparisons were exact, and no callback deadline exceedance was observed in
this neutral offline workload. Those results do not rescue the musical defect
or qualify a loaded show/hardware configuration.

## First assembled candidate: retained findings, not musical acceptance

Candidate `5667ac0` ran the identical v1 matrix from 2026-10-01
08:29:31.898900 to 08:30:19.730646 UTC, with other agent CPU jobs held. The
source-bound test executable SHA-256 was
`a517a18283c16fa3079198edd69f00e7520e1109d22d47a5a648c3b1c142030e`;
the report SHA-256 was
`82acc8a9811b0d0a11f46121502764fc8e3a917e78d1db473d1b685840d298bb`.
Evidence is retained in `issue-100-candidate-5667ac0`.

All 765 measured groups had zero Rust allocations/frees, every rendered sample
and transition observation was finite, and all 420 cross-block comparisons
were exact. All 315 unlocked render PCM hashes matched the original baseline;
all 45 locked-unity renders exactly matched their unlocked counterparts.
At 48 kHz/block 128, both analytical bass peaks remained at their intended
55/93.75 Hz across all seven ratios. The intended 93.75 Hz projection amplitude
was 0.349917–0.350586, compared with approximately 0.35 in the source.

The 48 kHz/block-128 one-deck callback had worst CPU p99 132.917 microseconds
and wall p99 134.877 microseconds across the locked ratios. The full neutral
callback matrix nevertheless retained **one wall deadline exceedance**:
96 kHz/block 64, ratio 1.16, lock off, block 73 took 2.055612 ms wall and
0.039500 ms thread CPU against a 0.666666 ms deadline. The difference indicates
time outside this thread's CPU execution; no specific external cause was
established. No locked callback or isolated-render deadline exceedance occurred.
This outlier was not discarded or represented as a hardware XRUN.

The separate dense two-deck show gate passed all 18 groups × three runs with
zero actual deadline exceedances, zero callback Rust heap operations/rejected
commands, exact recorded state and coincident full searches. At 96 kHz/128
frames its worst wall p99 was 0.6277 ms and maximum 0.7145 ms against a
1.3333 ms deadline. Across the complete matrix, worst wall p99/maximum were
1.0056/1.3535 ms. Source-bound evidence is retained in
`issue-100-show-5667ac0`; raw SHA-256
`dd23baf9449bb4caa3eb7850b50d028905aa9745728534ec0aa8645cc5ff9d09`.

The ordinary suite passed 803 tests (15 explicit fixtures ignored), and the
original issue 95 release gate passed all eight groups × three runs, including
109 native accessibility actions, from 08:32:13 to 08:34:59 UTC. That gate's
production binary SHA-256 was
`eb58187112bdb902b96716018547d0f9d29c1bd686a467c6e09391e37f06d9db`.
Its report/raw/log are retained as `issue-100-initial-performance.*`.

**Musical qualification stopped here.** The v1 transient source is a 5 ms
alternating-sign burst at the 48 kHz source Nyquist frequency. The rendered
loss is real: at ratio 0.5, pulse energy relative to unlocked playback fell to
0.00386–0.0992; at 1.5 it fell to 0.00539–0.0351. Some pulses concentrated
almost all remaining energy into a single sample. Wide observation windows and
whole-file energy confirm this is not a missed-window diagnostic. Fractional
source interpolation can cancel neighboring opposite-sign samples; this extreme
stress input is not representative evidence of ordinary drum fidelity.

The implementation and musical corpus require further work before publication.
The next corpus revision retains this stress source and adds an audible-band
onset train, with a freshly matched original-algorithm baseline. The v1 data
remain unchanged; the extra case must not erase the discovered failure or be
compared against a different workload as if it were the same experiment.

## Listening acceptance

The comparison script creates 35 deterministic blinded A/B pairs at 48 kHz,
with a separate implementation key and an explicitly unlocked rate-shifted
reference. It carries the corpus attribution and supplies an empty score sheet.
No level normalization, alignment correction or fabricated listening score is
applied. Human blind listening remains part of the user's final QA; objective
render measurements alone cannot mark that listening criterion passed.
