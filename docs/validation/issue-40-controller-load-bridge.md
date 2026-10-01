# Issue 40: controller load bridge

Controller `DeckLoadSelected` now enters a dedicated GUI mailbox before audio
command admission. The dispatch worker captures an existing `Arc` containing the
GUI-published typed source and title. The GUI drains up to eight of 16 requests
per frame and invokes the same `load_source` helper as crate buttons. A request
for file A therefore still loads A after browsing to B or publishing a rescan.
The current GUI selection remains B. A captured empty selection remains an error.

Typed built-in/file source definitions moved into a small shared engine module.
Queue admission and GUI dispatch counters are separate from audio consumption.
Full, disconnected and invalid-target requests report explicit submission errors
through the existing persistent error surface. Legacy requests applied directly
to the renderer lack a source capture and report an explicit error with no
allocation, Arc destruction, decoding or GUI lock acquisition.

## Validation

- `cargo test`: **204 passed**, including nine new test groups; `cargo build` passed.
- New modules pass `rustfmt --check`; `git diff --check` passes. Whole-project
  `cargo fmt --check` still reports pre-existing formatting differences.
- Synthetic Pioneer DDJ-FLX4 load messages travel through the actual MIDI input
  ring, dispatch worker, factory mapping, CommandPort mailbox and real egui App
  frame. The raw callback has zero measured allocations and frees.
- Both built-in stems load into both target decks; release and zero-velocity
  messages do not trigger loads. File requests retain source/deck through browse
  and real asynchronous rescan. A private WAV also travels through the production
  decoder worker and becomes the renderer's current deck audio.
- Real egui output contains loading and failure text. Missing files, empty
  selections, full/unavailable GUI queues, saturated audio admission and raw
  uncaptured renderer requests are explicit failures without media replacement.
- Saturating the mailbox proves its 16-request bound and eight-per-frame FIFO
  drain. A held audio producer-admission mutex does not block GUI handoff.

Selection capture occurs at dispatch-worker admission using the latest selection
published by the GUI, not at the hardware event timestamp. These are local
headless egui, worker and renderer checks with private files; no physical MIDI
controller, audio device or desktop FPS claim is made. Per-deck progress,
retry/dismiss controls and renderer-confirmed success are tracked by issue 45.
