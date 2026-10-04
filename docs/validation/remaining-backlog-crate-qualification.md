# Combined crate discovery qualification

Source `424e1419b5539963434103bc48b5b8311d2c1ce0`. Fresh Cargo test executable SHA256 `25e8148563c0b4be3896413914dd66b0ac5c3840c625ace10aafe6d776bc4115`. All retained source hashes remained unchanged during qualification; see [receipt](remaining-backlog-crate-qualification.json).

All **1505 ordinary checks passed**: 1504 in the main run and the isolated invalid-scene check. 32 opt-in checks were excluded from that invocation. Capability selectors match the actual executable. Crate discovery adds durable favorites, independent Unicode name search, direct manual/automatic memberships, viewport/selection/query restoration, native keyboard navigation and learned controller navigation with exact-ID admission and stale-list fencing. The initial full run exposed an outdated schema assertion; the corrected assertion checks the actual schema 12 and the complete suite passes.

The same executable captured all 32 routes through actual CPAL/ALSA playback/input and an owned private PipeWire loopback. Each run used 64 exact links, graph taps, record aliases and decoded WAVs; cross-channel overlap was zero and owned child processes exited. **Timing is not qualified.** The run concurrent with the exhaustive scene check reported 509 missing input frames and 26,941 input overflow frames. A second run without that scene check reported 1970 missing input frames and zero input overflow. Both complete receipts are retained. These are routing/capture results, not a zero-xrun or deadline pass.

The merged optimized gate remains pending. The prior source's failed wall-time receipt remains in [media-health qualification](remaining-backlog-media-health-qualification.md). [MIDI learn](remaining-backlog-midi-qualification.md), [routing and prepare](remaining-backlog-routing-qualification.md), [preparation](remaining-backlog-preparation-qualification.md) and [earlier baseline](remaining-backlog-baseline.md) retain their own source bindings.

Physical Pioneer/Numark/Akai/keyboard input, unplug/replug, converter timing, real suspend and listening remain pending. No release package or complete backlog acceptance is claimed.
