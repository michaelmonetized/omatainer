# Combined backlog qualification

Frozen implementation `adea3009e06899d266bbc5831bbc8c9bfdf0b95d` passed 1,447 ordinary tests with four test threads, plus the separately executed exhaustive scene check: 1,448 total. All 559 reviewed source hashes stayed unchanged. The fresh debug test executable has SHA256 `508df1738266bd3097f8cbcbc3bcb1b4d522f04e712a557ab7dc8c3cee67799f`. Its 118 focused checks include Undo, preparation protection, actual read-only deck loading, analysis/tag replacement previews, catalog publication, grid editing and smart-crate regressions.

The controlled optimized gate passed eight workloads with three repeats, native accessibility preflight, reviewed audio hashes and zero heap work in measured callback workloads. This attempt ran with process affinity restricted to performance cores 2–9. Binary, policy, toolchain, affinity and source bindings are retained in the [machine-readable receipt](remaining-backlog-qualification.json).

Two unrestricted optimized attempts failed wall-time ceilings: producer callback repeat 2 reached 10.98 ms against 10.67 ms; composer callback repeat 3 reached 14.79 ms against the same ceiling. Golden and allocation checks passed in both attempts. Both source-bound failed results and timing distributions remain in the JSON record, with hashes of the complete local reports and raw samples. The unchanged source passed with performance-core affinity without relaxing the policy. That pass does not erase the observed spikes or certify unrestricted scheduling or physical audio deadlines.

Real CPAL/ALSA streams against a private PipeWire null sink passed server restart, stream removal and a six-second process freeze while recording and performance protection were active. Recorded notes survived; explicit input acknowledgment and Play were required. All owned processes exited. These fixtures do not change the desktop audio server.

The public FreeToUse keyless API passed a live `lofi` search and current-track lookup, returning 266 catalog matches. The test fetched metadata, not music audio, and configured no paid license. Five capability-matrix checks and eight license-manifest checks passed.

The ordinary invocation explicitly ignores 31 opt-in maintainer/external checks; named backend, public-catalog, accessibility and performance checks above run separately. Other physical and maintainer checks are not silently promoted. Physical USB unplug/replug, listening, converter loopback, real system suspend and the remaining planned capabilities remain outside these results. The combined PR remains draft.

The previous completed baseline is preserved in [baseline evidence](remaining-backlog-baseline.md) and its JSON receipt. Preparation locking and replacement previews are described in [issue 140 evidence](issue-140-preparation-locks.md). A new source change requires renewed evidence.
