# Issue 100: key-lock implementation and quality evidence

## Scope and corpus

Qualification uses the production deck renderer and real OutputCallback in a
private offline process. It does not open an audio device. CPU, software content
displacement and startup behavior are separate observations; none establishes
converter latency, backend XRUN performance or a human transparency judgment.

The corpus contains two complete recorded VocalSet 1.2 singing-vowel files,
plus original analytical stereo bass, stereo transient pulses and the complete
procedural Omatainer drums/harmony mix. Version 2 retains those five sources and
adds an audible-band kick/tone/filtered-noise onset train. The recorded files cover one soprano
and one baritone performing the same slow forte arpeggio vowel; they are a
limited musical sample. The source license is CC BY 4.0, with author/DOI
attribution, exact member names, member CRC32/SHA-256 and complete terms in
`tests/fixtures/keylock`. Selected bytes were retrieved from the authoritative
archive and verified individually; the full 6 GB archive MD5 was not checked.
The recordings are test fixtures, not factory-library or production-embedded
media. Original generated corpus material follows the application's MIT license.

The fixed matrix uses forward ratios 0.50, 0.84, 0.92, 1.00, 1.08, 1.16 and
1.50 at 44.1/48/96 kHz output, blocks 64/128/512, with lock on/off. Each run
contains 630 isolated renderer cases in version 1, or 756 in version 2, alongside
126 actual callback cases and nine transition traces. Decode, analysis and file writes are outside timed blocks.
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
The search radius is 25 ms. Each non-silent search has at most 101 candidate
scores, each using 128 reference points. A dense, bounded scan of the overlap
retains both channels' peaks among the stratified points so isolated attacks
cannot disappear between probes. The score penalizes amplitude loss as well as
phase mismatch, and the exact continuation is one bounded candidate. Weights are
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
unlocked playback retain their previous smoothing behavior. Initial and silent
grain origins use a phase-safe sampling lattice, fenced at an explicit seek,
with at most one source frame and one output frame of quantization. Both initial
overlap contributions share that origin; the logical transport remains exact.

Resident source PCM supplies look-ahead without adding an output FIFO. At
48 kHz, conservative source-time configuration bounds are 67.688 ms of look-ahead
and 46.354 ms of content displacement, including the quantization margin. These are geometry bounds, not measured
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

These findings required further implementation and corpus work before publication.
Corpus version 2 retains this stress source and adds an audible-band
onset train, with a freshly matched original-algorithm baseline. The v1 data
remain unchanged; the extra case must not erase the discovered failure or be
compared against a different workload as if it were the same experiment.

## Revised candidate and matched version 2 evidence

The final assembled implementation at `60c414a` includes the issue 99
acknowledgement correction. Its ordinary suite passed **807 tests**, with 15
explicit opt-in fixtures ignored, in 48.15 seconds. The revised corpus ran with
other agent CPU jobs held, from 2026-10-01 09:07:23.951182 to
09:08:17.036079 UTC. All sources and the executable stayed bound to the compiled
inventory throughout measurement.

| Binding | SHA-256 |
| --- | --- |
| Shared version 2 workload | `44964770acf6f8c73ca133c05187bcc1465f6659fd0deb6ef298befc106bbf1d` |
| Original-algorithm baseline executable | `8ca4c3cb0635cf6b2ccc335701f93a55ba89d3b8988715f214eb9a4ae4875066` |
| Original-algorithm baseline report | `4bc831a5d6dbeff77e85386a83a313b37d15dbbf0197794a26e0ffc577ed2934` |
| Revised candidate executable | `329ab0f5c27a111e80b03577ac37dad1c5d58ce19fa6f08c24707b2dd3e6aa8e` |
| Revised candidate report | `fa35719826d2fdb37c4c6527b91332e8fc9b833793e0df5fe9b278424da94faf` |

The baseline checkout `6029254` adds only the version 2 harness to the original
algorithm/harness baseline. It ran from 08:53:32.759605 to 08:54:20.354977 UTC.
Both runs verified the original five source descriptors and PCM hashes against
the retained version 1 report on this host. Direct comparison verifies identical
complete version 2 workload identities. Local evidence directories are
`issue-100-baseline-v2-6029254` and `issue-100-candidate-v2-60c414a`.

All **891** measured groups (756 renders, 126 callbacks, nine transition traces)
had zero Rust allocations/frees; every output sample and transition window was
finite. All **504** cross-block PCM comparisons were exact. All **378** unlocked
renders retained their original-algorithm PCM hashes, and all **54** locked-unity
renders were exactly equal to unlocked playback. The 18 central locked-unity alignment
observations measured zero output-frame lag. No wall deadline exceedance occurred
in this corpus run. At 48 kHz/block 128 the worst locked one-deck callback CPU
p99 was **144.834 microseconds**, with wall p99 **146.667 microseconds**.

At 48 kHz/block 128, both analytical bass peaks stayed at their intended
55/93.75 Hz across all seven ratios. The left-channel 93.75 Hz projection amplitude was
0.349464–0.350300 against a source amplitude of approximately 0.35. This is a
measured improvement over the retained baseline, not a perceptual rating.

The revised transient diagnostics retain substantial limitations. The following
ranges include both channels and every pulse at 48 kHz/block 128. Energy is
relative to that pulse's **unity** rendering at the same output rate; it is not
silently multiplied by the tempo ratio. This measures attenuation/repetition,
not an ideal time-stretch target.

| Source | Ratio | Raw pulse energy / unity | Source-time onset displacement (ms) | Output 95% energy width (ms) |
| --- | ---: | ---: | ---: | ---: |
| Nyquist stress | 0.50 | 1.0388–1.8500 | −22.094 to −16.677 | 23.479–34.167 |
| Nyquist stress | 1.50 | 0.4279–1.0000 | 5.938–20.688 | 2.583–2.771 |
| Audible-band onsets | 0.50 | 1.0034–1.8497 | −21.385 to −15.917 | 3.875–54.354 |
| Audible-band onsets | 1.50 | 0.5927–1.0000 | 6.094–20.844 | 1.521–15.396 |

Across all three output rates and seven ratios, raw pulse-energy ranges are
0.3743–1.8506 for the Nyquist stress and 0.5291–1.8508 for the audible-band
onsets. The latter's source-time onset displacement spans −21.579 to 20.875 ms,
and output energy width spans 1.510–57.710 ms. These show real envelope changes;
no transparency claim follows from fixing the severe interpolation loss.
The separate narrow single-frame regression retains as little as 0.1814 of
unity energy at 44.1 kHz, an explicitly retained combined WSOLA/interpolation
limitation; that probe does not isolate sample-rate conversion as its sole cause. That focused probe is additional to the six-source listening corpus.

Joint stereo alignment preserves an output onset gap near the source's 1.5 ms:
1.4966–1.5193 ms across these transient cases (exactly 1.5 ms at 48 kHz).
This differs from the unlocked source-time-scaled gap at non-unity ratios.
Audible-onset energy-centroid gaps still vary from 1.3008 to 3.7551 ms; onset-gap
preservation does not imply identical channel envelopes.

The separate dense two-deck gate passed all **18 groups × three runs** under the
unchanged deadline ceilings, with zero heap operations/rejected commands, exact
recorded state, and coincident actual full searches. It retained **one actual
wall deadline exceedance**: 48 kHz/128 frames, ratio 0.50, second run, measured
block index 90 took **3.863382 ms wall**, **0.381210 ms complete thread CPU**
and **0.373501 ms render CPU**, against a **2.666666 ms** block deadline.
All measured CPU deadline counts were zero. Time outside measured thread CPU
explains the difference, but no specific external cause was established. The
fixed gate allows maximum wall time up to two block deadlines; passing it is
not equivalent to zero actual overruns or physical-backend qualification.

At 96 kHz/128 frames, worst wall p99/maximum were 0.688334/0.788459 ms against
a 1.333333 ms block deadline; complete callback CPU p99 was at most 0.684876 ms
against the unchanged 1.000000 ms p99 ceiling. Across all rates/blocks, worst
wall p99 was 1.042669 ms. Evidence is retained in
`issue-100-show-v2-60c414a`, with raw SHA-256
`b6e03ab1907a0b4b3b36a47f4bc17d5b716d30fb85e8c333a9752529df624f3b`
and verified-report SHA-256
`b04c57c6fcaf23638179eb84cb935e8dba27e4a1560226e4ce20127a4a6ff383`.
The earlier run's outlier and failed transient evidence remain above; no run
was discarded to manufacture a clean result.

## Final release and package validation

The unchanged issue 95 release gate passed all eight workload groups × three
sessions from **2026-10-01 09:10:01.228379 to 09:12:47.280976 UTC**. It built
and measured the assembled source locally on this Linux aarch64 M1 Pro host
(10 logical CPUs, 16 GB RAM, kernel 7.1.13-3-2-ARCH, schedutil,
SCHED_OTHER/nice 0, affinity 0–9). Other agent CPU jobs were held; unrelated
applications were not modified. One-minute load was 1.6792 before and 1.3525
after the workload.

The production binary SHA-256 is
`646f3c6d412cfcf1e51d5d724a0d0f220c2ae9c67f27c97fac30c64a5607b0c9`.
The mandatory native AT-SPI fixture visited 240 nodes, performed 109 actions and
ran 1,046 actual App frames; project reopen retained one note and preference
scale 1.25. This private fixture does not establish human screen-reader or
physical desktop/device acceptance.

Worst wall p99/maximum across the three runs, in milliseconds:

| Workload | p99 | Maximum |
| --- | ---: | ---: |
| Producer callback | 0.7956 | 0.9065 |
| Composer callback | 0.8549 | 1.0495 |
| Live DJ callback | 0.0985 | 0.5159 |
| Hybrid callback | 0.8730 | 2.2547 |
| Large crate UI frame | 3.3581 | 4.2339 |
| Multiple-controller UI frame | 5.3490 | 9.2792 |
| IPC round trip | 6.6013 | 9.8825 |
| MIDI dispatch | 4.3258 | 6.4021 |
| Project round-trip UI frame | 5.2288 | 5.2288 |
| Long recording UI frame | 4.8105 | 4.8105 |
| Long recording renderer | 2.5770 | 3.6775 |

Original audio goldens, exact state/admission checks and measured callback heap
requirements passed. Independent source-bound report checking and package
verification passed, followed by all seven CLI, six follow-protocol and five
runtime-isolation groups. The retained package is `issue-100-final-package`;
release evidence is `issue-100-final-performance.{json,raw.json,log}` in the
local work directory. The separate keylock show workload above remains the
specific two-keylocked-deck timing evidence.

## Listening acceptance

The comparison script creates 35 deterministic blinded A/B pairs for version 1,
or 42 for version 2, at 48 kHz,
with a separate implementation key and an explicitly unlocked rate-shifted
reference. It carries the corpus attribution and supplies an empty score sheet.
No level normalization, alignment correction or fabricated listening score is
applied. Human blind listening remains part of the user's final QA; objective
render measurements alone cannot mark that listening criterion passed.

The matched version 2 comparison produced **42 blinded pairs** in
`issue-100-blind-v2-final`. The 42 score rows were checked and every human
rating cell is empty. `listen/` holds the listening files, attribution and score
sheet; `operator/` holds the concealed implementation mapping and objective
diagnostics. The issue's human listening acceptance remains pending the user's
reserved final QA. No proprietary expansion parity is asserted.
