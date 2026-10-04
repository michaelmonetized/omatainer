# Library media health

The library's **health…** panel validates a selected row or up to 4096 rows in the current filtered crate. It captures their sources and versions before starting. Later navigation cannot substitute another crate. Validation never installs a sample, writes audio, updates preparation or replaces a live deck.

The existing decoder worker reads the complete stream, verifies packet continuity, declared length and available decoder checksums, and discards converted PCM after each packet. Validation shares the decoder with deck loads, sampler loading and analysis. Explicit loads preempt it; performance protection cancels it. Cancellation retains earlier completed observations. Closing the panel leaves admitted work running; application close requests cancellation.

Browser rows show the last condition for their exact published fingerprint: ready, length unverified, missing, unreadable, unsupported, corrupt, changed or validation limit. Read-only is independent of playback readiness. The panel and accessible row descriptions explain the next recovery action. Tag warnings remain separate when the strict audio decoder can still read the stream. Streams without trustworthy length evidence receive a distinct length-unverified condition, disclose that incomplete content cannot be ruled out, and count as needing attention.

Rescan and validate after replacing or reconnecting media. Checks are transient observations, not persisted promises about future filesystem access. They expire when the published row's fingerprint changes. At most 100000 source observations are retained; an evicted source returns to unchecked status. Source files above 8 GiB, packet conversion buffers above 64 MiB and unsupported project channel/rate limits are explicitly refused. Decoder library internals remain subject to their own format limits; an OS read already in progress cannot be forcibly interrupted.

Import failures remain visible in the existing per-input skipped-entry report. Supported-extension files enter the library even when validation later discovers damaged or unsupported bytes, so healthy files survive a partially failed import and each retained row can be checked separately.

Fixtures exercise actual read-only and permission-denied files, missing files, truncated audio, unrecognized containers, malformed ID3 metadata, an absent removable volume identity, recovery after repair, a complete stream whose decoded PCM exceeds the retained-buffer limit, cancellation between packets, optional-work protection and foreground admission. Native egui controls exercise captured crate validation during live playback, changed source versions and cancellation. Linux software fixtures do not substitute for physical drive, controller or listening tests.

Qualification is pending for this new source. The completed preparation-lock baseline remains separately bound in `remaining-backlog-qualification.md`; it is not promoted to this implementation.
