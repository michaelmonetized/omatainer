# Smart crates: issue 205

Open Named crates, select an empty manual crate, then choose Smart crate rules. Typed conditions support title, artist, key, tag, group, notes, BPM, duration, rating and whether the current version has confirmed playback. Text supports Contains or Equals with Unicode normalization and full case folding. Numeric ranges include both ends; unknown BPM or duration does not match. All requires every condition; Any requires one. Each rule contains 1–16 conditions and the catalog permits 64 typed smart crates.

Preview reports the exact match count and up to eight track titles. Save binds to that successful preview's rule, crate identity, revision and immutable library/catalog publication. Editing, closing, replacing the catalog or replacing the row publication invalidates approval. Preview runs on a bounded optional worker and can be cancelled. Its result never changes the catalog on its own. Existing manual members and annotation rules are preserved when conversion is refused. Removing a typed rule leaves an empty manual crate.

The catalog worker prepares membership with each exact metadata publication. Unchanged rules reuse results by stable track identity; only changed metadata/annotations are evaluated again. Row reordering preserves identity and requires no new predicate evaluations. An unavailable or different publication cannot expose stale membership. Refresh smart membership deliberately recomputes current saved rules. The native library view reads prepared membership and keeps its existing search and virtualized viewport.

Library schema 10 persists typed rules. Schema 9 and earlier records migrate without inventing rules; typed fields disguised in older schemas fail closed and preserve the original bytes. Malformed conditions, unknown fields/actions, invalid ranges, oversized rules and attempts to mix manual/automatic membership are refused before publication. Metadata import/export uses the catalog's existing whole-document validation and conflict checks.

## Acceptance fixtures

- `library::smart_crates::tests`: every supported field, All/Any semantics, inclusive ranges, Unicode text, unknown metadata, malformed imported rules, persistent save/reopen, schema migration and preservation of manual members.
- `ui::library_metadata::collection_rows::tests`: a real catalog-owner annotation transaction updates membership incrementally; explicit Refresh recomputes it. The 100,000-track fixture checks exact membership, a one-track edit, row reversal and concurrent zero-heap software renderer callbacks.
- `ui::library_smart_crates::tests`: actual native text and numeric controls, All/Any actions, preview/save/refresh/remove, durable catalog contents, cancellation/stale preview rejection and automatic membership updates while a loaded deck keeps playing.

The first development measurement on this Linux aarch64 host was 70.5 ms to build 100,000 memberships and 91.9 ms to rebuild the identity publication with one predicate evaluation after a single annotation change. It ran beside 1,390 finite software callbacks with zero callback allocations/frees. These are development observations, not a physical audio deadline or sustained hardware soak. Final qualification records are retained with PR #500.

Genre, added-date and playcount fields are not yet part of the authoritative catalog and are not advertised as supported rule fields. Played membership uses confirmed current-version catalog history; it follows the catalog worker's publication rather than assuming a GUI highlight is playback. Rules are evaluated off the native UI and audio callback; constructing an exact changed publication still visits its bounded row identity list.
