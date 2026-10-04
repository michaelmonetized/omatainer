# Combined MIDI learn qualification

Source `a9caa32ea76c64b7d84e5f1ca65e1490a80a64b4`. Fresh Cargo test executable SHA256 `642c95494da978be9084205c7605718979b704bf39d581baa565b9daf9df1c2d`. All retained source hashes remained unchanged during qualification; see [receipt](remaining-backlog-qualification.json).

All **1498 ordinary checks passed**: 1497 in the main run and the isolated invalid-scene check. 32 opt-in checks were excluded from that invocation. Capability selectors match the actual executable. Native egui and synthetic-wire checks cover MIDI capture, exact source/channel/address, conflict review, preview, edit/remove, cancellation, disconnect, ambiguous endpoints, durable preferences storage and file conflicts. Queued messages crossing capture/configuration revisions are fenced. The saturated callback test holds the learn editor, log and admission locks and still reports zero callback heap work.

The same executable passed actual CPAL/ALSA playback and capture through the owned private **32-channel PipeWire** loopback: 64 exact playback/capture links, graph taps, record aliases and decoded WAVs. Owned child processes exited. Four startup input underruns and zero input overflow were recorded. Physical converters and zero xruns are not qualified.

The merged optimized gate remains pending. The prior source's failed wall-time receipt remains in [media-health qualification](remaining-backlog-media-health-qualification.md). [Routing and prepare](remaining-backlog-routing-qualification.md), [preparation](remaining-backlog-preparation-qualification.md) and [earlier baseline](remaining-backlog-baseline.md) retain their own source bindings.

Physical Pioneer/Numark/Akai/keyboard input, unplug/replug, converter timing, real suspend and listening remain pending. No release package or complete backlog acceptance is claimed.
