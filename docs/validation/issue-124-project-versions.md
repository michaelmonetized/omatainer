# Named native project versions: issue 124

Source freeze: `eb3fb97e418cc9856d1c015b696fbbd2f0290b37`. Base: PR #488,
`stack/project-import-metadata-limit`.

Project → Named versions snapshots a coherent native document with a name and
notes. Immutable decoded audio blobs are shared by content across revisions;
per-version aliases, analysis and musical metadata remain separate. Compare
reports named musical changes; Restore opens an unsaved copy through the ordinary
project replacement handshake; Branch writes a new native project without
changing the original. Pruning previews exact revisions and unused audio, checks
that the review is current, commits the index before deleting reviewed files,
and preserves every retained audio reference. Work runs on cancellable workers.

## Local qualification

Linux aarch64, locked Rust/Cargo 1.98.0:

- 1,350 ordinary tests pass: 1,349 serial (279.80 seconds) plus the exhaustive
  scene-boundary check (259.29 seconds); zero failures, 30 opt-in ignored.
- 18 optimized focused tests pass: seven version model/native UI/comparison
  fixtures, five import model fixtures, five native import UI fixtures, and
  the existing pending-template/project boundary regression.
- Version fixtures cover shared PCM, exact reopening and aliases, cancelled
  publication and orphan cleanup, retained audio, changed records/index,
  foreign files and symlinks. Actual App controls exercise Snapshot, Refresh,
  Compare, Branch, Restore, New/Open guards, notes/reload, original-file
  preservation, prune review/application, cancellation and changed sources.
- Eight license/package fixtures pass. T3's interpreter executable alias uses
  the explicit sys.executable=/usr/bin/python3.14 launcher for private Python
  subprocesses and the empty-PATH fixture, as in the preceding layers.
- Controlled optimized gate passes eight workloads with three repeats each,
  all reviewed checks/audio hashes and zero allocations/frees in workloads
  measuring callback heap activity. CPU 6: 2026-10-03T13:57:12.756604+00:00 to
  2026-10-03T14:00:06.229346+00:00.
- Native AT-SPI preflight: 158 actions, 264 visited nodes, 592 App frames.
- 511 retained source/build/gate files. Independent source, embedded-license
  and performance verification of the preserved production binary pass.

## Artifact bindings

`/home/michael/Projects/omatainer-work/issue-124-qualified-release` retains:

- Production: `8fb3c72f052f88fbfdef4f3ec5c6f1d7a5016faa92989d7aacb0fb4341975aa0`
- Release tests: `be9675d2146f5977a8c51c765c71589884557e02d487d68b57a5d3ec4027a1ec`
- Manifest: `daf97c36a9245aed4340e7f900772c4573632980023242beda42f914c2b0280e`
- Ordinary tests: `e0a60af6bd3da7cf353af94a0b87bdd176f0fabe43910f6f423adcc7831b9656`
- Policy: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`
- Performance receipt: `61b89e1a3f3de94dddb4f0cc418b2d9d368fa7a78999309227c3a3f206f327a6`

Adjacent final-qualified ordinary receipts, focused release logs, package logs
and performance JSON/raw/log retain the actual results. Binaries were preserved
before any next-layer compilation.

The actual native App and renderer are exercised without an OS window. Physical
MIDI/audio hardware and display behavior remain unverified. Versions obey the
native 64 MiB metadata, 256 media-reference and 1 GiB decoded-audio bounds, with
128 indexed revisions. Comparisons summarize up to 12 changed rows and eight
fields per category while retaining total counts. External video and hardware
resources retain the existing native project's requirements. Interrupted
unindexed files can be reclaimed by a reviewed prune; arbitrary files are refused.
