# Preparation locks and replacement previews

Issue #140 is implemented in the combined backlog branch. Physical controller qualification is separate.

Tracks have independent grid, BPM and metadata locks. These choices persist in library schema 11; older catalogs migrate without silently accepting new fields. Replacing source bytes retains the track's lock choices and starts a fresh preparation version. A replacement does not inherit unverified grids or cues.

The library's **locks…** panel captures the selected row or up to 4096 filtered current versions. Select lock changes, review the exact versions, then save. The catalog owner rechecks source identity, metadata, grid and previous locks before changing any target. A stale or duplicate target rejects the complete batch. Closing an unsubmitted review discards it; a submitted save reports its actual durability result.

Grid locks block the grid editor, queued renderer edits and grid-changing Undo/Redo. A coherent receipt carries the saved grid into the renderer and restores it if a grid command raced the catalog save. Other deck controls, cues, loops and playback history remain editable. The callback check is bounded and does not allocate.

BPM and metadata locks apply to scans, loader observations, tag refresh and persistent analysis, including Force. A locked BPM is used by the actual file-load path. User BPM retains its higher priority even when unlocked. A reviewed manual tag edit can intentionally change a selected locked field; it does not change unselected locked fields.

In **analyze…**, Preview selected/filtered analysis changes captures the same bounded row set as analysis and asks the catalog owner which selected fields need refresh. The list shows each row's saved BPM, duration and waveform summary, marking locked, unselected and verified-cache fields. Preview neither decodes nor saves. Changing versions, locks or displayed values makes its row stale; analysis always rechecks the source and current locks. The tag editor's existing actual-tag inspection shows the exact saved-to-refreshed field values after user sidecars and locks are applied. This does not approve media writes.

Locks and analysis are sidecar data. A real read-only FLAC fixture is loaded through the shipped deck-load control after saving BPM and grid locks; the loaded sample retains the saved BPM, rejects another grid and leaves source bytes intact. The existing tag transaction path uses a sidecar when a file cannot be rewritten.

Validation selectors:

- `library::protection::tests`: automatic metadata/grid protection; atomic stale/duplicate batches; schema migration and reopen.
- `library::analysis::tests`: late analysis cannot overwrite protected fields; permitted waveform analysis preserves preparation.
- `library::tags::tests`: automatic observations and failures retain locks; intentional reviewed edits preserve unselected fields; refresh previews match publication without changing the catalog.
- `engine::beatgrid::renderer_tests`: grid locks reject queued edits and Undo with zero callback heap work.
- `ui::library_protection::tests`: native captured review/save/unlock and read-only deck load.
- `ui::library_analysis::tests`: native filtered replacement preview captures rows and leaves catalog/analysis unchanged.
- `ui::library_tags::tests`: native actual-tag refresh preview displays saved protection on read-only media.
- `ui::library_metadata::tests`, `ui::grid_editor::tests`: existing owner and editor regressions.

The preparation-lock source `adea3009e06899d266bbc5831bbc8c9bfdf0b95d` passes the complete 1,448-test ordinary qualification, 118 focused checks, three private native audio fault modes and the optimized eight-workload gate. Source and executable bindings are retained in `remaining-backlog-qualification.md` and its JSON receipt. This qualifies the implemented software subset, not every remaining feature or physical device.
