# Issue 80: readable and precise last-play times

Last-play cells no longer print epoch seconds modulo 100000. With one clock read
per crate draw, the formatter shows `Just now` under a minute, whole `N min ago`,
`N h ago`, or `N d ago`, then `YYYY-MM-DD UTC` after seven days. The row tooltip
retains BPM provenance and adds the exact UTC timestamp to nanosecond precision.
Unknown history stays `—` with an explicit unknown tooltip. Future timestamps are
labeled `Future time` and annotated relative to the computer clock.

The formatter accepts an explicit clock, floors signed fractional epoch offsets,
and uses caller-owned `gmtime_r` UTC calendar storage. No timezone/environment or
locale mutation occurs. Pre-epoch times stay meaningful; unsupported calendar
ranges retain the exact signed Unix-epoch nanosecond offset. Existing `libc` is
used, with no dependency or audio-path change.

Visible row cache entries retain their next age boundary and reference clock.
Only history replacement, an expired boundary, or backwards civil time reformats
a known timestamp. Unchanged rows reuse their formatted strings, unknown rows do
not reformat on a clock change, and offscreen rows remain unformatted. Repaint
requests use the nearest visible boundary, capped at one day before native
`Instant` conversion: a corrupt far-future civil time cannot overflow eframe's
native timer addition.

Six fixed-clock regression groups cover exact minute/hour/day thresholds, older
UTC dates, fractional timestamps, future expiry, unknown state, the instant before
1970, a pre-epoch leap day, a pre-epoch reference clock and both `time_t` extremes.
Real egui crate draws/hovering prove labels and precise tooltips are painted. At
100, 10000 and 50000 rows, 100 steady frames format zero cells; crossing a boundary
or moving the clock backwards formats exactly the changed visible cell without
another filter pass. Hidden history edits do not format hidden rows. An installed
repaint callback reproduces native timer addition and verifies far-future delays
remain bounded.

Validation on the issue 70 baseline: all 353 tests pass with `cargo test --offline`;
`cargo build --offline` and `git diff --check` pass using the private local Cargo
target. Independent read-only peer review found no remaining blocker. This is
deterministic headless UI and native build evidence, not a physical audio/controller
QA run.
