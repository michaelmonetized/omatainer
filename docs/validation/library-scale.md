# Large-library browsing and preparation — #194

Library search and sorting now use full Unicode keys prepared by the existing metadata worker. Titles, artists, effective musical keys, groups, notes and tags keep their complete text; field predicates, missing values and stable ties retain their original meanings. All eleven sort columns use the prepared values. Search no longer repeatedly normalizes every row on the GUI thread.

Small changes of up to 256 metadata rows update an unchanged whole-library query directly, including sorted views. Unchanged records are shared between worker publications. Reordering, source replacement, larger changes and selected-crate changes use exact full membership. Selection and scroll anchors follow source identity. Live confirmed play history remains separate and invalidates relevant queries and sorts.

The supported software qualification covers 10,000 and 100,000 catalog rows. Search p95 must stay below 100 ms; actual wheel scrolling and GUI application of prepared small metadata updates must stay below one 16.7 ms frame. The additional normalized search index has a 128 MiB retained limit. Oversized metadata receives an explicit refusal without shortening text or returning incomplete matches. Existing catalog, identity tables, audio and temporary preparation allocations are separate from that index budget.

The native scale fixture runs 120 sorted field searches, 480 real wheel frames and 32 small metadata publications at each size while two decks render at least 300 seconds of finite, audible software PCM with no callback allocations. The scale fixture uses the actual index builder through a test publication adapter. It measures GUI application after preparation; worker index build time and process RSS/high-water memory are reported separately. The five-minute audio quantity is rendered faster than real time and does not establish five minutes of wall-clock stage playback.

A separate native workflow uses the actual persistent metadata owner, decoder, background analysis and filesystem scan. Its catalog contains 10,000 synthetic entries, three real FLAC/Ogg/MP3 files and two built-ins. With one file already loaded and looping, it analyzes another file, replaces the loaded file on disk and imports one additional real file. The two-path import/analysis publication must finish within five seconds while preserving the selected source, loaded audio Arc, media identity and ongoing playback. Bulk import of 1,000 files is not claimed by this receipt.

No physical audio or MIDI device, live capture or new GUI is opened. The independent stage app and session stay preserved. Software qualification does not claim physical display/audio acceptance.

Measured on Apple MacBook Pro (16-inch, M1 Pro, 2021), 10 logical CPUs, MemTotal:       15763184 kB, kernel `7.1.13-3-2-ARCH`:

| Tracks | Search p95 | Scroll p95 | Prepared GUI update p95 | Index build | Retained index | Process high-water |
| --- | --- | --- | --- | --- | --- | --- |
| 10,000 | 1.794 ms | 0.092 ms | 0.796 ms | 16.181 ms | 2.35 MiB | 387904 kB |
| 100,000 | 23.927 ms | 0.093 ms | 1.980 ms | 195.269 ms | 23.51 MiB | 1238400 kB |

The actual two-path replacement/import and all-fields analysis publication took 752.660 ms. The concurrent renderer produced 14400000 frames at 48000 Hz with zero callback allocations.

The complete unfiltered Rust suite passed 2,006 checks, zero failures and 44 existing explicit qualifications ignored (554.47 s). Three search checks, three index checks and the actual incremental view, scale and worker-owner workflows passed. All fifteen package/dependency/checklist checks passed. The [source-bound receipt](library-scale-receipt.json) retains 770 source files, executable SHA256 `82d10faa9dd3a1b594f251266d2491406fec5153242f5100b2738e3e43011f8f`, platform, memory, latency distributions, audio and real import measurements, and preliminary fixture corrections.
