# Tempo automation and changing meters

The native Project menu opens **Tempo and meter**. Ordered tempo rows use
quarter-note positions, BPM and `step`/`ramp`; meter rows use positions and
numerator/denominator. A ramp changes BPM linearly in musical position until
the next point. Pickup length, click subdivisions, count-in bars and accent/
beat gains are editable. Applying creates one atomic History entry. A failed,
cancelled or stale operation retains the draft and the current music; closing
unapplied work requires an explicit discard.

Native state version 8 retains exact tempo ramps and click options. Earlier
versions still open with their original timing semantics; they cannot smuggle
new timing fields through a legacy version. Unchanged imported MIDI meter
clock metadata survives edits. Tempo, note/controller scheduling and recording
share the analytic beat/seconds conversion. The shared `sample_at` conversion
provides rounded sample coordinates for worker-side musical boundaries. This
layer does not add the later audio-warp editor or general master-audio export.

The piano-roll ruler labels pickups, odd meters and meter changes, including
partial bars. Count-in uses complete bars at the effective starting meter and
tempo, then starts transport at its exact output frame. Clips and recording
wait; monitored live inputs and DJ decks continue. Stopping, project replacement
or selecting a scalar tempo cancels the lead-in.

The editor accepts 1–4,096 strictly ordered points per list, beginning at zero,
on the project's PPQN grid. Tempo is 40–240 BPM; meter denominators are powers
of two through 128. Pickup is shorter than the first bar and precedes its first
meter change. Subdivisions are 1, 2 or 4; count-in is 0–4 bars; gains are 0–2.

Standard MIDI files cannot encode a BPM-linear ramp. Export integrates each
target MIDI tick and emits its average tempo, coalescing equal neighboring
values. Export refuses more than 131,072 sampled ramp ticks before publication;
users can reduce PPQN or shorten the export. Dense sampled tempo tracks can
exceed the native import conductor's 4,096-point bound; the import UI retains
the source metadata without offering that oversized map for transport. The
export dialog discloses these limits. Native project files retain exact ramps.

## Verification

Tests render a 120→180 BPM ramp with 7/8, 5/4 and 4/4 sections. A separate
Simpson-rule oracle integrates reciprocal BPM; it does not call the production
logarithmic integral or its inverse. Actual note gates, MIDI CC74 output,
recording clocks and metronome clicks match at 44.1, 48 and 96 kHz, within one
output sample, with zero renderer heap allocations/frees. Numerical tests also
cover increasing, decreasing, flat and nearly flat ramps.

One-bar 7/8 count-in at 120 BPM and 48 kHz holds the musical position for exactly
84,000 frames, emits fourteen subdivisions and starts the first note at frame
84,000. Click gains, pickup accents and half-open interval boundaries are
checked independently. MIDI exports at PPQN 480, 960 and 1,920 are encoded and
decoded, then integrated from their emitted events; representative timestamps
differ from the independent oracle by less than one 96 kHz sample.

Actual App/egui accessibility actions edit the map and click settings, apply
the worker transaction, preserve notes, save/reopen a native project and publish
a MIDI file through the native export dialog. Malformed rows, stale projects,
cancelled operations, dirty close handling, history inverses, schema migration
and export-capacity refusal are exercised. These are native headless workflows;
they do not claim desktop screen-reader, physical MIDI or listening QA.

The initial full-suite run exposed relocation/recovery failures. A subsequent
host namespace mount made media discovery fail reproducibly. That independent
parser defect is fixed in the preceding PR #470. Failed logs remain retained;
only the corrected qualification below is authoritative for this layer.

## Frozen qualification

Qualified source: `228ac5ece8d7b7d15ade74d1a446366263010ea4`.
Linux aarch64; all native builds and workloads ran locally.

- Corrected serial ordinary suite: 1,225 passed, zero failures, 27 opt-in
  ignored, 250.28 seconds. The separately run exhaustive scene-boundary test
  passed on that same executable in 255.35 seconds: **1,226 ordinary tests pass**.
- All 18 timing, metronome, export, controller, atomic-history, schema and native
  editor tests pass on the exact optimized release test executable as well.
- Unchanged standard performance gate: eight workloads, three repeats each,
  zero callback allocations/frees. Original audio hashes remain producer
  `26f84beea80f5ec5`, composer `e2f197b02633bd3d`, live DJ `d7711a2dc3b32a39`
  and hybrid `adc9540dab057fed`. The gate ran on CPU6 from
  2026-10-02 19:02:15 UTC through 19:11:26 UTC and its independent recheck passed.
- Required private native AT-SPI preflight: 158 actions, 253 visited nodes,
  571 frames. Eight license/package script fixtures also pass.

Release binary SHA-256:
`dda1a4d009f96911c18b1159886646f3e3eaa7b5d04dc6d1b29ff7dd4a084a62`.
Release test binary SHA-256:
`ee2b7585fba425703f4245c8a75c2d707d0fd249c7eab395e538a4925f5b9948`.
Unchanged policy SHA-256:
`fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`.
License manifest SHA-256:
`7f62264618240cabeea8d5cf0b415df25fb4312762ce4f185ade4d0ebb9277ed`.
The source inventory binds 451 files.

Preserved executables are in
`/home/michael/Projects/omatainer-work/issue-114-qualified-release/`.
The independently rechecked gate, raw measurements, optimized timing summary
and ordinary-suite logs are retained under `/home/michael/Projects/omatainer-work/`
as `issue-114-performance.json`, `issue-114-performance.raw.json`,
`issue-114-release-timing-tests.json`, `issue-114-release-timing-tests.log`,
`issue-114-corrected-suite.log` and `issue-114-scene-boundary.log`.
Earlier unsuccessful runs remain separately labeled in that directory.

The inherited issue107 supplemental quiet-host wall maxima remain unresolved;
this passing standard gate does not supersede physical producer/live-DJ QA.
No installation, merge or issue closure is performed by this layer.
