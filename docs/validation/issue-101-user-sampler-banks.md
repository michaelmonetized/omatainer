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

Editor/native interaction evidence and the final combined source inventory are
recorded after the UI union is verified. No physical controller, audio-device
latency, listening-quality or screen-reader user qualification follows from these
headless/native API fixtures. Filesystem/codec cancellation is cooperative between
bounded chunks/packets; an operating-system call is not forcibly interrupted.
