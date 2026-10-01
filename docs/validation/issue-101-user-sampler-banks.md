# Issue 101 — reusable user sampler banks

## Ownership and format boundaries

Reusable `sampler-banks/banks.json` is separate from native projects and the DJ
catalog. Its strict schema 1 permits 64 named definitions of 16 slots, bounded to
4 MiB. Names are presentation: overwrite requires a captured definition ID;
imports and copies generate distinct working IDs. Startup corruption, future
schemas and external replacement are reported without replacing the file.
Precommit cancellation preserves old bytes. After rename, a sync failure remains
a committed-with-warning result; late cancellation cannot relabel a save as lost.

A working project has at most 16 banks. Engine State 4 stores sparse slots,
source-second ranges, gain, identities and embedded available PCM. State 1–3
migration treats every old bank as embedded, including a bank named `Kit`.
Only an explicit live startup factory marker permits output-rate regeneration.
Generic project container/recovery formats do not change. User/project PCM keeps
its source rate; held voices retain the onset's audio, range, gain and track.

The shared decoder has two deck lanes and one sampler lane, one active operation,
and one pending/result per lane. It does not impersonate a deck for bank loading.
A source is opened once, bounded and fingerprint checked, SHA256 hashed and decoded
through that descriptor, then rechecked against both descriptor and current path.
Only fresh successful measurements carried by an Applied bank request can qualify
the catalog. Copied or serialized source claims cannot qualify it. Missing or
changed reusable media is an explicit unavailable slot, while valid embedded
project PCM remains playable without its original file.

A single bounded background asset owner pins bank data, settings and samples
independently. Limits are 1 GiB PCM, 8 MiB metadata, 512 sample allocations and
1,024 bank/settings allocations, including retained voices/history/captures and
pending reservations. Saturation rejects work before excess decode. Final release
runs off the callback after the last real owner disappears; a surviving project
capture joins the same owner in a new graph. Poison refuses new preparation until
restart while preserving callback pins. Project installation keeps the existing
renderer client rather than dropping its final client allocation on the callback.
Existing independent 256 MiB incoming-owned-payload and Undo budgets still apply:
successful preparation is not a promise that an oversized edit will be admitted.
Queue/renderer rejection remains visible and does not change the working bank.

## Checked implementation paths

- Real WAV, FLAC and Ogg preparation preserves source rates and validated trim
  ranges. Simultaneous imports have distinct IDs. Same-content catalog relocation
  followed by deleting the old pathname restores the original sample bytes.
- Declared decode overflow rejects before decoded frames; cumulative PCM credit
  rejects an oversized mixed bank. A path replacement during same-descriptor
  preparation is detected. Missing/damaged definitions and failed single-slot
  assignments preserve their distinct outcomes.
- Worker store fixtures cover restart, duplicate names, exact overwrite IDs,
  admission cancellation, protection entered and exited before completion,
  completed-read publication cancellation and late committed-save cancellation.
- Actual renderer tests cover immutable held voices, source ranges/gain/destination,
  Undo/Redo, stale targets, producer and consumer protection, atomic edit claim,
  short audition completion and reserved Stop under a saturated ordinary queue.
  Warmed callback allocation/free counters are zero for these edit/release paths.
- Actual native project capture/install/replace verifies embedded source retention,
  State 1–3 migration, preserved renderer owner and final voice/history/capture
  release away from the callback. Original generated factory PCM, names and peak
  arrays are compared at 44.1, 48 and 96 kHz.
- The normalized core and catalog/recovery prerequisites passed 851 ordinary tests
with 15 opt-in tests ignored (four test threads; 51.15 seconds). The retained log
  is `issue-101-normalized-full.log` in the local work evidence directory.

## Final editor and combined verification

The actual App suite includes 14 editor groups covering all slots, mixed formats,
per-slot gain/trim, prepared audition, renderer Apply/Undo, private-store restart,
missing media and verified relocation. Controlled boundaries include cancellation
before and after renderer claim, no proof persistence before Applied, replaced
targets, snapshot lag followed by Undo, asynchronous native action identity,
pending-store close, same-frame Apply/Cancel and engine disconnect. Small-window
scroll/focus, F1 and Escape are exercised through real egui controls.

The private Linux AT-SPI workflow passed 123 actions across 242 native nodes and
1,062 actual App frames, within the existing 128-action/70-second child limits.
It assigns real FLAC PCM, changes gain with native SetValue, auditions/stops,
checks a confirmed durable reusable Save separately from renderer Apply, then
clears a preview and cancels without changing the applied bank. Evidence is
retained locally as `issue-101-native-ui.json`; the final source-bound release
gate repeats this preflight on the assembled package source.

After combining the core, editor and catalog/recovery/audio-action prerequisites,
`cargo test --locked --offline -- --test-threads=4` passed **867 tests**, with
**15 opt-in tests ignored**, in 44.49 seconds. The real offline-manual generator
also passed. Log: `issue-101-final-full.log`. These are local development checks;
the controlled performance/package gate is a separate qualification step.
`cargo build --locked --offline` passed, and the resulting production executable's
embedded manifest/notices exactly matched the refreshed source-bound inventory.

No physical controller, audio-device latency, listening-quality or screen-reader
user qualification follows from these headless/native API fixtures.
Filesystem/codec cancellation is cooperative between bounded chunks/packets; an
operating-system call is not forcibly interrupted.

## Final assembled release qualification

The final source `ffc5a13` includes the captured audio-route confirmation fix and
its regression. The complete ordinary suite passed **868 tests**, with **15
opt-in tests ignored**, in **44.54 seconds** (four test threads); retained log:
`issue-101-consent-final-tests.log` in the local work evidence directory.

The unchanged local performance gate passed from
`2026-10-01T10:16:40.106409+00:00` to
`2026-10-01T10:20:17.176673+00:00`, with other agents' builds/tests paused during
measurement. It freshly built the locked offline release and release tests,
verified the embedded source inventory, and ran all eight fixed workloads in
three sessions. Native preflight passed **123 actions, 242 nodes and 1,015 App
frames**, persisting the note and 1.25 preference scale. The release binary SHA-256
is `8c25c9ac8e199d3355e6a4f7f3e2c61b7676f6c2c6c82bbcf969d37a6b128209`.

Worst callback wall p99 / maximum across the three sessions, in milliseconds:
producer **0.830550 / 1.073428**, composer **0.820591 / 1.337347**, live DJ
**0.107002 / 0.174085**, hybrid **0.857508 / 0.964801**. These are local fixture
callback measurements, not device latency. Measured callback and recording heap
allocations/frees, rejected commands and MIDI drops were zero. Worst large-crate
frame p99 was **3.237031 ms**; multi-controller frame p99 **4.841921 ms**, IPC
round-trip p99 **8.891794 ms**, and MIDI dispatch p99 **4.260291 ms**. All fixed
thresholds passed; no threshold or workload was relaxed.

An independent report check passed. Packaging and package verification passed
with the exact release executable, followed by all **7 CLI**, **6 follow-protocol**
and **5 runtime-isolation** groups. The latter protocol fixtures deliberately do
not contact the user's application socket or audio device. The verified package
is `issue-101-final-package`; reports, raw measurements and detailed log are
retained as `issue-101-final-performance.{json,raw.json,log}`, with gate output
in `issue-101-stack-gate.log`, under `/home/michael/Projects/omatainer-work`.

An earlier gate attempt was stopped during compilation, before measurement, to
include the route-confirmation fix; it is not counted as passing evidence.
Physical Pioneer/Numark/Akai/MIDI-keyboard, hardware latency, human listening and
screen-reader acceptance remain pending the user's final producer/composer/live
DJ runs. No issue is closed and no PR is merged by this qualification.
