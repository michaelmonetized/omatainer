**Professional backlog scope and provenance**

Prepared 2026-09-30 for `michaelmonetized/omatainer`, baseline commit `cae546c7bc840ec1a643e4befe18f91e86d0e4c7`.

The user authorized individual GitHub issues for every audited gap and a broad producer/live-DJ maturity inventory. The manifests separate existing defects (`fixes.json`) from shared foundations (`platform.json`), production workflows (`production.json`) and DJ/library/hardware workflows (`dj.json`). Titles use the requested `[fix]/` and `[feat]/` prefixes. This inventory proposes work; it is not a claim that the product implements it.

Stable references verified on official pages are [Ableton Live 12.4.6](https://www.ableton.com/en/release-notes/live-12/) and [Serato DJ Pro 4.0.10](https://serato.com/dj/pro/downloads). Suite/optional Packs, Serato expansions and previews are identified separately. Live 12.4.15b5 and Serato DJ 5.0 are preview baselines rather than released-feature claims.

Each issue contains its own acceptance criteria and validation. Source facts use immutable commit links; benchmark capabilities use official manuals, release notes or standards. Beyond-benchmark professional requirements are described as proposals. Physical compatibility, streaming permissions, SDK access, licensing, and unsupported operating-system formats require actual verification before implementation claims.

Priority definitions: P0 protects creative work/audio continuity or enables fundamental operation; P1 is essential professional workflow; P2 adds workflow depth; P3 is specialist, optional or platform-dependent. Priority is sequencing, not deletion of scope.

`publish_backlog.py` validates IDs, titles, dependency references and cycles; creates each issue serially; records durable receipts; reconciles remote issues after ambiguous errors; and publishes a linked tracking index. Publication is only performed with `--publish`. The final `created-issues.json` and `published-issues.md` are receipts, not speculative links. Dependencies are linked to actual issue numbers during publication. Product code is unchanged.

Published and read back successfully: **356 child issues** (83 fixes and 273 features), plus [tracking issue #1](https://github.com/michaelmonetized/omatainer/issues/1). Every child title, full body, label, dependency and index entry matched the reviewed inventory. The [self-contained audit probes and outputs](https://github.com/michaelmonetized/omatainer/issues/1#issuecomment-5919708009) are attached to the tracker and were read back byte-for-byte. See `published-issues.md` and `publication-verification.json` for the full receipt.
