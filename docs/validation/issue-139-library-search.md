# Field-aware library search: issue 139

Crate search combines terms with AND. Plain words search title, artist, key and track annotations. Text values preserve user metadata; matching uses Unicode normalization and full case folding. Quote spaces, for example `artist:"Nine Inch Nails" tag:"clean edit"`.

| Field | Meaning | Example |
| --- | --- | --- |
| title, artist | Contains text | `title:midnight` |
| key | Exact normalized text | `key:Am` |
| tag | Exact saved tag | `tag:"clean edit"` |
| group, note | Contains annotation text | `group:opening note:request` |
| color | Exact RGB annotation | `color:#FF6600` |
| bpm, length, rating | Exact, inclusive range, >= or <= | `bpm:120..128 rating>=4 length:3:00..4:00` |
| played | Confirmed current source/version membership | `played:no` |

Length accepts seconds or minutes:seconds. Unknown BPM/duration does not match numeric predicates. Rating is an integer from zero to five. Search is bounded to 4096 bytes and 64 terms. Unknown fields, malformed ranges or unterminated quotes display an error and return no rows. Search syntax is distinct from saved annotation crate rules.

Search all library broadens the current query without changing the selected named crate. Turning it off restores the crate query, source selection and scroll. Deliberately selecting another crate discards the old return context. Renderer-confirmed first playback invalidates played/unplayed results; repeated plays update display timestamps without another full filter pass.

## Current evidence

Two query/parser fixtures pass. Eight actual egui crate-view fixtures pass. The 100000-track synthetic Catalog contains title/artist/BPM/key/duration/played metadata and saved tags/ratings. Four query results match an independent arithmetic oracle: 856, 1, 429 and 16666 rows. The latest focused run took 17.8–59.5 ms per cold query on Linux aarch64; this is a local measurement, not a portable latency guarantee. At fixed 1440×108 viewport, repeated frames retain at most six cached rows and perform no filter rebuild or cell reformatting. Native checkbox actions prove restoration of crate query, selection and scroll. Confirmed-play and changed-file fixtures prove version-qualified played status.

The five-second cold-query guard catches catastrophic regressions; timings remain visible in the receipt. Sustained physical controller browsing and disk/media preparation workloads are tracked separately in issue 194.
