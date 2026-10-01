# Track analysis queue and inspector

The actual App exposes selected-row and captured filtered-crate analysis with
BPM, duration and waveform selection, forced reanalysis, cooperative cancellation,
and explicit Retry/Skip after foreground-media preemption or a source/storage
failure. The immutable library and filtered indices are retained by Arc; at most
4,096 rows are accepted, and only one source is inspected/prepared/persisted at a
time. Later selection/filter changes cannot change that source identity.

The catalog owner's actual receipt separates prepared progress, durable saves,
and committed results with uncertain durability. A cancel that loses the
publication claim still displays that commit. The cache inspector displays
partial saved values and a verified bounded waveform; it never decodes a source
by itself. Manual preparation and unselected analysis fields are preserved by
the catalog layer. Optional automatic key detection remains unavailable.

Eight actual-App groups pass on ec911fc+d4aae95+e0cffbc:

- Real FLAC, Ogg and MP3 source queue while renderer frames continue; changing the
  filter retains the admitted Arc identities and order. A fresh App/catalog open
  reuses the verified cache and waveform with no deck load.
- Read-only cache inspection, duration-only preparation and forced BPM-only
  update retain the other fields.
- A 4,097-row crate and an empty field selection refuse admission explicitly.
- A bounded test hook pauses the production analysis worker before its real
  source verification/decode. Actual foreground admission or UI Cancel produces
  a terminal result; Retry keeps the original captured source after selection
  changes. The ordinary foreground decoder remains the production show-aware
  path.
- Changed source bytes pause without altering the archived version; Skip leaves
  those bytes and prior analysis untouched.

Additional groups verify worker-disconnection ownership, protection/centralized
close cancellation, and a retained native Cancel action after preemption. The
last case reproduced Cancel turning into Retry in 0.10 seconds before the fix.
Always-present independent action slots preserve Cancel identity across phases.
Queue generation and frozen row position prevent another source becoming its
target. The fixed full UI subset passes eight groups in 0.60 seconds.

The full ordinary suite passes 909 tests with 16 opt-in tests ignored in 96.12
seconds (two test threads). Production build and diff checks pass. The fresh
private Linux AT-SPI fixture passes with 243 visited nodes, 129 actions and 1,293
App frames. Added native actions analyze the actual local FLAC fixture, await
catalog persistence, reuse the cache, inspect its 2,048-bin/0.25-second waveform,
and close the panel. The first expanded run hit the fixture's prior 128-action
cap; that failed attempt remains `/tmp/issue104-native.log`. Its bounded script
capacity is explicitly expanded to 160; the 70-second timeout and performance
workloads/policies remain unchanged. Passing evidence is
`/tmp/issue104-native-final.log`; other logs are `issue104-ui-tests.log`,
`issue104-cancel-red.log`, `issue104-ui-full.log`, and `issue104-ui-build.log`.

The ignored `ui::library_analysis::tests::long::long_files_under_callback_analysis`
compiles for the parent's final assembled qualification. It requires a fresh
`OMATAINER_ANALYSIS_LONG_DIR` containing the launcher's exact three-file source
manifest and 180-second WAV/FLAC/Ogg inputs. It observes actual decode/analysis
progress, cancels before publication using a bounded metadata hook, restarts,
persists, reopens and verifies cache reuse without source re-decoding. A separate
thread runs the real four-channel OutputCallback against an independent seeded
reference and counts warmed callback Rust allocation/free, finite output and
sample continuity; pacing is outside the measured callback. It records actual
frames/durations, source hashes, stable track IDs, manual preparation and creative
revision preservation. The large corpus run is not claimed by this UI delta;
root executes it only on the final assembled branch. A bounded off-GUI Store
open/drop probe synchronizes old-owner shutdown before in-process reopen.

No physical controller, backend deadline or human listening claim is made by
these functional fixtures. Root owns separate cache/publication and long-file
qualification evidence.
