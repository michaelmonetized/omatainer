# Combined backlog qualification

Frozen implementation `2ff1d5b078cf18f9d07dcfe071fac8f5660e26dd` passed 1,454 ordinary tests with four test threads and the separately executed exhaustive scene check: 1,455 total. All 563 reviewed source hashes stayed unchanged. Fresh debug test executable SHA256: `b5a3439a94b8117a5785032eeba9b40a67d4ab21465994e9060400bd21fe64ef`. Its 71 focused checks cover media validation, native UI, decoder admission, analysis, imports and help.

The optimized eight-workload gate **failed** on performance cores 2–9. Producer callback repeat 3 reached 19.24 ms against 10.67 ms. Its measured CPU maximum was 0.823 ms. Reviewed audio hashes, zero callback heap work and native accessibility preflight passed. The observed host load averaged 11.13 before and 12.24 after the workload on ten logical CPUs. Other builds were active. This does not establish the cause or erase the failure. Source, executable, policy, toolchain, affinity, measurements and hashes of complete local reports/raw samples remain in the [JSON receipt](remaining-backlog-qualification.json). No deadline or hardware qualification is claimed.

Real CPAL/ALSA streams against a private PipeWire null sink passed server restart, stream removal and a six-second process freeze while recording and performance protection were active. Recorded notes survived; explicit input acknowledgment and Play were required. All owned processes exited. These fixtures do not modify the desktop audio server or prove real system suspend behavior.

Live FreeToUse keyless `lofi` search and current-track lookup passed against this executable, returning 266 catalog matches. Only metadata was fetched; no music audio or paid license was configured. Five capability checks and eight license checks passed.

Thirty-one opt-in maintainer/external checks are explicitly ignored by the ordinary invocation. Named native, public-catalog, accessibility and performance checks above run separately; other checks remain unevidenced. Physical Pioneer/Numark/Akai/keyboard tests, USB faults, listening and converter loopback remain pending. The remaining planned capabilities are not qualified by these results. The combined PR remains draft.

The [preparation-lock baseline](remaining-backlog-preparation-qualification.md) retains its two unrestricted wall-time failures and its controlled passing attempt. The [older baseline](remaining-backlog-baseline.md) is also preserved. Neither baseline is promoted to this new source. [Media health evidence](issue-145-library-health.md) describes this implementation.
