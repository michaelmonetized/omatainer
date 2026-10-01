# Issue #45 — visible, truthful deck-load status

The shared GUI/controller/drop loading path now keeps a status record for each
deck. The record retains the requested typed source and displayed title, so a
failure on A remains visible while B loads, and a later crate selection cannot
change what Retry loads. File rows also render the full path. Loading, waiting
for the audio engine, loaded, failed, and replaced/unloaded are separate states.
There is no invented percentage: decoding does not expose byte-level progress.

Errors do not expire. Retry resubmits the captured source through `load_source`;
Dismiss removes that deck's row. These are real egui buttons, including for an
unavailable built-in. Decoder warnings remain visible before and after loading.
Other application status messages continue to render beside the deck records.

Queue admission is not completion. GUI media loads use `DeckLoadRequested` with
an atomic application receipt. The renderer checks the receipt and, for decoded
files, the existing #37 generation token before replacing media. An atomic
Pending→Applying claim resolves cancellation against application: cancellation
that wins prevents media mutation; a claim that wins may finish before the next
queued replacement/unload. Applying remains visibly queued. It marks the
receipt current only after replacement. Each deck retains its current receipt
and retires it on any later media replacement or unload, including raw engine
commands. Cancelling a pending request does not retire already-applied media:
a rejected or not-yet-executed unload retains its truthful current status. The
GUI checks current state immediately before rendering; identical
titles and reloading an already-current built-in cannot manufacture completion.
A disappeared renderer produces failure rather than an indefinitely queued or
obsolete successful state. No renderer mutex or synchronous decode was added.

Validation (local Linux, isolated private Cargo target):

- `cargo test --offline`: **211 passed**, including existing #34/#35/#37/#40
  loading, cancellation, controller bridge and contract coverage.
- UI coverage: **32 tests passed** within the full suite.
- `cargo build --offline`: passed.
- `git diff --check`: passed.
- Seven added regression tests run the real App frame/layout and inspect painted
  egui text inside a 1440×900 viewport. Actual pointer events activate Retry and
  Dismiss. Production decoder fixtures cover missing, corrupt and valid WAV
  loads; the resulting media is checked in the real renderer. Tests also cover
  persistent errors with simulated frame time advanced by hours, independent
  deck rows, captured-source retry after browsing, missing built-in recovery,
  application delayed in the command queue, rejected/queued unloads, same-title reload, controlled before/after-claim cancellation races, renderer
  disconnect, unload, and a stale completion delivered after its replacement.
- Existing warning tests continue to assert the actual painted warning text,
  and controller tests continue to enter through the synthetic Pioneer message.

These are headless egui/application/decoder/renderer checks, not desktop FPS,
physical-controller or audio-device QA. The receipt adds only a fixed atomic
state transition to media replacement; this issue makes no new claim that media
replacement or command-payload retirement is allocation-free. Those broader
callback lifetime concerns remain separate from visible load feedback.
