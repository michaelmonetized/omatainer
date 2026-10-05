# Combined routing and prepare qualification

Source `6bd195a5eaef0f77693648ffaa070493cffbe8ba`. Fresh Cargo test executable SHA256 `eaf17e2a86217faf68f756be881340ab8e4ffedb0fba31372725930fb3b7cc8d`. Retained source hashes were unchanged across qualification; see [receipt](remaining-backlog-qualification.json).

All **1484 ordinary checks passed**: 1483 in the main run and the isolated invalid-scene check. 32 opt-in tests were excluded from the ordinary run. Capability selectors match the actual compiled executable. The earlier merged executable additionally passed 88 focused routing, prepare, project, Undo and controller checks; its distinct SHA is retained in [prepare evidence](issue-146-prepare-queue.md).

The current executable passed actual CPAL/ALSA playback and capture through an owned private **32-channel PipeWire** loopback: all 64 exact playback/capture links, graph taps, record aliases and decoded WAV outputs. Owned child processes exited. The fixture reports four input startup underruns and zero overflow. This does not qualify physical converters or prove zero xruns.

Changed library content now rejects before deck installation; retained-deck and stale-metadata fixtures pass. Custom graphs cannot borrow legacy stereo attribution for automatic prepare removal, and the native queue explains why tracks are retained.

Current merged optimized qualification remains pending. The prior source's optimized wall-time failure remains recorded in [media-health qualification](remaining-backlog-media-health-qualification.md); its passing unit/native results do not establish a passing release gate. [Preparation qualification](remaining-backlog-preparation-qualification.md) and [earlier baseline](remaining-backlog-baseline.md) remain bound to their own sources.

Physical Pioneer/Numark/Akai/keyboard input, unplug/replug, converter timing, real suspend and listening remain pending. No release package or complete backlog acceptance is claimed.
