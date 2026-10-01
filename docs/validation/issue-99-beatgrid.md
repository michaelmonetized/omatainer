# Issue 99: persistent manual beatgrids

A grid records a downbeat in source seconds and a uniform period independently
of the decoded BPM hint. Beat coordinates before the downbeat are negative, so
pickups and leading silence do not redefine musical beat zero. Manual tempo is
validated from 20 to 400 BPM. The initial model uses four-beat bar labels; it does
not implement tempo maps, changing meter or automatic downbeat detection.

The deck editor previews Set at playhead, signed slip, stretch, half/double tempo
and Reset. Its waveform preview displays the symmetric mean magnitude of all
three analyzed bands, with at most 512 columns and 128 visible grid markers.
Applied and draft markers are distinct. Fields preserve f64 precision. Cancel,
Escape, closing the window and replacing the media discard unapplied drafts.
An explicit scroll area and fixed footer keep the final controls reachable in a
640-by-360 window. The controls have native names, roles, focus and contextual F1
help generated from the same catalogue as the offline manual.

Apply is one receipt-qualified undoable command. Its own atomic acknowledgement
is read before the coherent preparation snapshot. An applied command remains
acknowledged even if another grid edit or Undo changes the current preparation
before the GUI polls. A pending command is not completed by somebody else's
preparation revision, and receiver loss reports an unknown outcome. Stale media,
invalid input and protected-mode rejection cannot edit a replacement source.

The manual grid controls effective tempo, Match phase and quantized loop origin;
cue positions remain absolute source positions. Reset recovers the immutable
decoded BPM hint even if an older effective-tempo snapshot is still published.
Catalog schema 4 and native project State 3 retain grids. Older supported catalog
and project states default to no manual grid; invalid and future states fail
before installation. The recovery journal and container remain version 1 because
their existing generic state payload already carries the versioned project.

Grid-only preparation follows the same stable track identity and exact-byte
relocation rules as cues. Essential catalog saving continues while performance
protection defers optional source hashing. Both qualification admission and
worker hashing include grids without any hot cues; returning to Studio retries
the bounded backlog. Reanalysis can update its own metadata without replacing
manual geometry. Engine receipts publish preparation in 87 fixed atomic words.

## Evidence and limits

The actual App tests use real decoded 16 kHz PCM fixtures with pickups and leading
silence, while explicitly injecting ambiguous 60/240 BPM analysis hints. They
correct both to 120 BPM, edit the origin, persist, restart at 96 kHz output and
compare every cue's source position and beat coordinate. The injected hints
exercise correction of ambiguity; they do not qualify the tempo detector.

Further actual GUI/worker paths cover a grid with no hot cues saved under
protection, deferred hashing, Studio resumption, verified relocation and reload.
Recovery tests create durable records through the real worker, preview and
restore State 3, migrate legacy State 1/2, and reject future/invalid states while
preserving the current session. Restored transport remains stopped.

Renderer tests cover pickup arithmetic, sample-rate independence, Match and loop
phase, coherent publication, stale/no-op edits, Undo/Redo, reanalysis and catalog
identity. A warmed callback allocation fixture verifies both an accepted grid
edit and an already-queued command rejected after protection begins. The private
Linux AT-SPI workflow exercises opening, field focus, half/double, Set, slip,
Apply, draft Reset and Cancel through the actual GUI and renderer evidence.

All files and buses used by these fixtures are private. These checks do not
claim physical-controller, screen-reader listening, audio-device deadline or
vendor-algorithm parity. The fixed release timing gate is qualified separately
on the assembled stack.

On the normalized issue-98 prerequisite `442d7cd`, the complete local run passed
**784 ordinary tests, with 13 opt-in tests ignored**, using
`cargo test --locked --offline -- --test-threads=1` (152.43 seconds). This includes
the three added protection/relocation/recovery integration groups and the prior
grid model, renderer and GUI groups. The explicit offline-manual regeneration
test and `cargo build --locked --offline` also passed.

`scripts/check-accessibility.py` against that freshly compiled test executable
visited **240 native nodes** and performed **109 actions across 1,011 App frames**.
The union retains the original 128-action fixture cap and the existing cue,
project, recovery, help and performance workflows. The source/component license
inventory validated, and the production binary's embedded manifest/notices
matched the regenerated records exactly. No timed release-gate result is inferred
from these correctness runs.
