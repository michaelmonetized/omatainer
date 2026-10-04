# Discover named crates

The named-crate manager searches names independently of the track query, filters saved favorites, and reveals the selected track’s direct manual, annotation-rule and typed-rule memberships. Nested parent crates do not inherit child members. Favorite changes use the existing catalog owner, revision conflict check and atomic save. Catalog schema 12 migrates earlier valid records to unpinned crates; older headers containing the new field reject without modifying the file.

Choosing a search, favorite or membership result retains one original crate/whole-library context across multiple result selections. Return restores its track query, selected source and viewport anchor. Missing crates or tracks display a notice rather than inventing membership. Verified relocation follows retained selection and scroll anchors.

The filtered tree is limited to 4096 saved crates and 32 levels. Names accept 256 UTF-8 bytes, match Unicode search normalization, and show their full parent path in discovery results. Results and manual reverse membership are cached on exact publications; immutable manual membership lookup is built on the catalog worker. The UI virtualizes rows. Alt+Up/Down browses filtered results and Alt+Home/End selects the first/last. Automatic membership uses the same saved rules as the actual crate views.

Learned Browse crates requires an explicit relative encoder with one row per decoded step. Return to previous crate view uses a note button. Controller admission captures exact IDs into the existing 16-request mailbox, dispatches at most eight per GUI frame, rolls back cursor movement on full admission, and fences old list epochs. Raw renderer calls reject without heap work. Requests admitted before the GUI changes crates continue to reference their previously captured track sources.

All 97 focused checks pass; the additional native keyboard fixture passes after Alt navigation was added. Complete current-source qualification is in progress. Physical controller navigation remains pending attachment; no Pioneer or Numark mapping/driver compatibility is implied.
