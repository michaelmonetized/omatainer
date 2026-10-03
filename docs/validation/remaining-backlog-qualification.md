# Combined backlog qualification

Frozen implementation `ccbebbfcb52ef3922cb30b61cf8a41edf60c8676` passed 1,435 ordinary tests with four test threads, plus the separately executed exhaustive scene check: 1,436 total. All reviewed source hashes stayed unchanged during both runs. Thirty-one opt-in maintainer/external checks remain explicitly ignored.

The controlled optimized gate passed eight workloads with three repeats, native accessibility preflight, reviewed audio hashes and zero heap work in measured callback workloads. Its binary and source bindings are retained in the [machine-readable receipt](remaining-backlog-qualification.json).

Actual CPAL/ALSA streams against a private PipeWire 1.6.8 null sink passed server restart, stream removal and a six-second process freeze while recording and performance protection were active. All recorded notes survived, explicit input acknowledgment and Play were required, and all owned processes exited.

The complete suite had previously reproduced a race between recovery discovery and an exact support-link restore. Closed sessions now allow shared readers; writers and deletion retain exclusive locks. Deterministic regressions cover simultaneous discovery/lookup/restore and deferred retirement cleanup. Twenty repeated native support-link restores also passed. The output owner separately checks physical identity after opening the still-muted stream, before enabling it; a simulated replacement during open retains the original target and project without advancing audio callbacks.

This is a qualification baseline for the implemented subset. The combined PR remains draft. Physical USB unplug/replug, listening, converter loopback, real system suspend and the remaining planned capabilities are not covered by these results. New source changes require renewed evidence.
