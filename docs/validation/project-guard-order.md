# Project draft guard order

Source freeze: `e75b9ddf40736d425efbac6368de51543b0894c6`.

A queued preparation checks that an uncancelled Prepare operation still exists
before opening draft review. Cancelled or unrelated operations cannot reopen a
retained timing editor or replace the cancellation message. An active replacement
evaluates both source and timing guards before combining their results, so both
pending editors expose their explicit Keep/Discard choices after one project action.

62 focused native checks pass on Linux aarch64: project workflows 35, dependency
GUI 5, timing GUI 5, portability GUI 3 and Help/manual 14. The two added regression
checks also pass individually. They exercise real GUI text edits, source search
and replacement selection, both Keep controls, a worker-paused preparation whose
Ready event is queued before cancellation, and unchanged project identities.
The fresh-process portable import/playback probe runs within its GUI round trip.

Test executable SHA-256: `de881783f1945fb835e713e7c33db8c1c83140a25dd9be9898b63d2766139d2b`.
License manifest SHA-256: `d6aaf40e82fa7c538c1c21fe57ca6b2d85238a6a84b9122fec6c2092cae7dd47`.
License inventory/check and diff checks pass. Receipts:
`/home/michael/Projects/omatainer-work/project-guard-order-tests.json` and
`project-guard-order-tests.log`. The prior issue 116 full release qualification
remains bound to its own source freeze; this small control-flow layer is qualified
by the focused native checks above. Physical devices and desktop screen readers
are not exercised.
