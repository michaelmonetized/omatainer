# Issue 71: retain crate history by verified identity

The existing asynchronous scan already carries matching row metadata and restores
selection by typed source. The playback-history layer records renderer-confirmed
history independently of scan results. This layer closes the remaining first-scan
hole: the worker's previous-fingerprint map may be empty, so that map cannot prove
that a baseline timestamp belongs to the file currently found at the same path.
The row merge now requires equal source and a matching known file fingerprint;
built-in stems retain their stable typed identity.

Removed entries disappear from the visible crate. Session-confirmed history stays
in its existing identity-indexed in-memory store; removing a root and restoring it
can therefore recover an unchanged file's timestamp. Moved paths and replacement
content do not inherit old timestamps. No move detection or cross-restart persistence
is claimed. Filesystem metadata fingerprints are cache identity, not content hashes.

Two new actual-worker/App regression groups cover the first-scan stale baseline,
unknown fingerprints, unchanged files, built-in history, moved/deleted/replaced
files, reordered results, filtered selection, repeated rescans and temporary root
removal/restoration. They assign known timestamps and check the displayed-history
accessor against explicit expected values. Existing concurrent-playback-during-scan
and worker-held UI responsiveness tests remain active. All fixtures use private
temporary directories; no user's media, live socket or hardware is touched.

Local preparation validation: all 349 tests and production build passed; all ten
scan regression groups also passed separately. An earlier full-suite invocation
ended with an OS input/output error while writing the test listing, before a test
failure was reported; an unchanged rerun completed successfully. `git diff --check`
passed and read-only peer review found no blocker.
