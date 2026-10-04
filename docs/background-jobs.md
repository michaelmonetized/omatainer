# Background jobs

Open **Setup → Background jobs**. Each row has a session request ID, worker
state, measured progress, declared reservation and a Cancel button while queued
or running. Unknown totals stay unknown. Closing this window keeps work running.
A finished worker may still need a workflow choice, audio acknowledgment or a
catalog save. Its original window reports that result.

The current expensive workflows share one admission queue: deck decoding, track
analysis, library indexing/tag discovery/replacement search, provider search and
preview downloads, sampler preparation, video import/preview/render, MIDI file
interchange, library backup/verification/restore, embedded artwork and portable project inspection/export/import. This does not imply
plugin scanning or stem separation exist in this build.

The queue retains at most 64 records, never reuses request IDs and refuses a full
queue before starting more work. Equivalent pending/active deck and analysis
requests share their existing source-bound identity. Other workflow workers
refuse duplicate active operations. Superseded source generations cannot publish
late results. A deck eject, project replacement or media change also invalidates
a decode's captured deck generation at the renderer.

At most three expensive workers run: one deck decoder and two optional workers.
Active declared reservations total at most 3 GiB. Deck decoding reserves 1536
MiB, indexing 256 MiB, backup 1024 MiB, media health 64 MiB and provider/artwork work 128 MiB; operations retaining large
project/media state reserve the full budget and run alone. These are admission
reservations, not measured RSS or an operating-system memory cap. Existing
source, PCM, asset, catalog and output bounds still apply. Deck PCM is capped at
512 MiB before allocation when its length is declared, and while decoding when
its length is unknown.

Deck loads take priority over queued optional work and request cancellation of
running optional work when necessary to free a turn or memory reservation.
Performance protection cancels optional work through its existing publication
guard. Cancellation is cooperative at decode, traversal, network and child
process boundaries. It cannot interrupt an OS read already in progress or undo
a write that already committed. Queued/running reservations are released by the
worker owner; a UI publication choice does not hold its worker turn.

On Linux, only owned worker threads change to SCHED_OTHER, nice at least 10 and
IOPRIO_CLASS_IDLE. Child processes inherit these priorities. Failed priority
application refuses the worker turn. The GUI and audio threads do not consult
this queue or inherit its lowered priorities. Other platforms report that this
priority policy is not qualified. Disk scheduling behavior depends on the
kernel's I/O scheduler; setting a priority is not a disk latency guarantee.
Linux documents its [I/O priority
classes](https://docs.kernel.org/block/ioprio.html) and [per-thread nice
behavior](https://man7.org/linux/man-pages/man2/setpriority.2.html).

Job activity is transient and is not saved in projects. Existing workflow
publication receipts and project/catalog persistence remain authoritative.
