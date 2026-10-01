# Issue 100: bounded two-deck keylock show workload

`benchmarks/keylock-show-policy.json` is the fixed policy for a separate opt-in
measurement. It leaves the matched 630-render/126-callback quality matrix and the
original issue 95 workload, policy and audio goldens unchanged.

The actual `OutputCallback<f32>` renders two simultaneously keylocked decks,
eight tracks with 1,024 notes each and three effects per track, live note
recording, track-gain changes and crossfader changes. Identical original
procedural media and origins make both decks perform their bounded alignment
search on the same callback. A small fixed DC bias prevents all-silent analysis
windows. This source is a timing stressor, not listening-quality evidence.

The matrix is 44.1/48/96 kHz, 128/256 frames, and fixed ratios 0.5/0.84/1.5. Each
of the 18 groups runs three times, with 128 warm-up and 2,048 measured callbacks.
Deck pitch/jog events from the original hybrid schedule are omitted to keep the
declared ratios and search hops fixed; the other event order is shared unchanged.

The verifier applies the existing issue 95 deadline ceilings, scaled by actual
frames/rate. It separately reports counts exceeding the actual one-block
deadline, so a ceiling pass does not imply zero offline exceedances. Render CPU,
complete callback wall time and wrapper thread CPU are separate measurements.
The latter includes clock/allocation-instrumentation overhead. Preparation,
control submission, waveform hashing and report I/O happen outside those timed
callbacks. This does not measure backend XRUNs, physical output latency or human
listening quality.

Every run checks finite/nonzero audio, exact original and recorded notes,
recording release, command queue/history status, both actual Locked modes and
rates, coincident alignment counts, and zero Rust callback allocations/frees.
The full-search counter increments only after the silent-reference early exit.
Every measured hop must perform a full search, and both decks' full-search
deltas must agree in every callback. The report retains hop counts, full-search
counts and the number of callbacks containing coincident full searches.
Repeated schedules must produce the same quantized audio hash. Raw per-block
timings are retained, including outliers; there is no trimming or automatic
budget relaxation.

Build locally after refreshing and validating the license/source inventory, then
use a fresh directory beneath an explicit evidence root:

```sh
python3 scripts/check-keylock-show-load.py self-test
OMATAINER_KEYLOCK_EVIDENCE_ROOT=/absolute/private/evidence \
  python3 scripts/check-keylock-show-load.py run \
  --test-binary /absolute/path/to/release/deps/omatainer-test-binary \
  --out /absolute/private/evidence/new-keylock-show-run
```

The executable must embed the exact current source inventory. The runner checks
that inventory and executable hash before and after execution and binds them to
`raw.json` in `verified-showload.json`. Output directories/files are never
overwritten. A failed policy retains its raw evidence and returns failure.
Measurements used for final qualification must come from the final assembled
source, during a coordinated quiet host window. An ordinary smoke regression
covers both endpoint modes and a complete 2,048-block recording/history run;
its debug timings are not qualification evidence.
