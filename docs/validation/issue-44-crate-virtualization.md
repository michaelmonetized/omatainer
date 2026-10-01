# Issue 44: virtualized crate and cached browsing

The crate uses egui `show_rows` with fixed 18-pixel rows, rendering only its
viewport and bounded overscan. BPM, duration and last-play cells are cached only
for that range; title, artist and key borrow the immutable library directly.
Rows leaving the viewport release their cells, so formatting memory follows the
viewport rather than the full library.

A filtered index vector is keyed by the query and immutable library identity.
The scan worker still owns sorting and old-library retirement. The cache holds
only a weak library reference: it detects `Arc::make_mut` metadata changes and
prevents allocation-address reuse without keeping old media metadata alive.
Controller selection publication uses the same cache, removing the extra full
filter pass that previously happened after rendering.

Selection and scroll anchors use typed sources across replacement/reordering.
Filtering retains a still-visible selected source and reveals the result;
removed sources clamp to valid rows. A focused crate accepts Up/Down, Page
Up/Down and Home/End and reveals offscreen selections. The real search field
keeps its text editing behavior.

## Evidence

The fixed **1440 × 108** headless egui fixture includes crate rendering and
controller selection publication. After ten warmup frames, 100 frames measure
elapsed CPU-side headless frame time. These timings are not desktop FPS, GPU
presentation latency, or physical-controller measurements.

| Entries | Steady median | Steady p95 | Rows rendered/frame | Rows formatted/frame | Entries filtered/frame |
| --- | --- | --- | --- | --- | --- |
| 100 | 23.833 µs | 26.916 µs | 5 | 0 | 0 |
| 10,000 | 23.834 µs | 29.959 µs | 5 | 0 | 0 |
| 50,000 | 23.916 µs | 28.542 µs | 5 | 0 | 0 |

Cold frames and query/library changes still perform one full filter pass. A
scan's sorted publication can require a source lookup across cached indices;
this change does not claim constant work for changed queries or publications.

Five new regression groups exercise the actual egui frame: the size matrix,
filtering/history/metadata invalidation, source and scroll-anchor preservation,
focused keyboard navigation/search editing, and real wheel scrolling. Existing
crate buttons/double-clicks, controller capture, and asynchronous scan-selection
regressions also remain in the full suite.

Cold measured frames were 1.012 ms, 1.058 ms and 1.447 ms respectively.
`cargo test` passed **209 tests**; `cargo build`, new-module `rustfmt --check`
and `git diff --check` passed. The focused timing run used dev opt-level 1 and
`--test-threads=1`. A read-only peer review found no blocker.
