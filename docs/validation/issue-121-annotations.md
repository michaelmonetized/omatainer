# Track ratings, colors and performance annotations: issue 121

Source freeze: `2325e6554a6e168c11c4f07e85adcf16d6bdd748`. Base: PR #482, `stack/provider-commit-guard`.

Library schema 9 stores rating, color, performance group, tags and multiline notes
against stable track IDs. Schema 8 and earlier migrate with empty annotations;
legacy headers cannot hide new fields. Source replacement, verified relocation
and catalog export/import preserve track-level annotations.

The native editor captures stable selected/filtered IDs, replaces only checked
fields after Review/Apply and publishes through the existing single-writer save
receipt. Invalid targets or fields reject the whole batch. No embedded audio-tag
operation is part of this path. The crate table exposes five annotation columns,
ordinary text search covers annotation text, and explicit predicates combine.
Saved annotation rules evaluate current published library membership. Enabling a
rule on a populated manual crate fails without removing its members. Selecting a
saved rule loads its current values; failed captures clear earlier targets.

## Local qualification

Linux aarch64, locked Rust/Cargo 1.98.0; final combined source above PR #482:

- **1,323 ordinary tests pass**: 1,322 serial, zero failures, 30 opt-in ignored,
  268.75 seconds; exhaustive invalid-scene boundary separately passes in 255.76
  seconds. Immutable executable partitions run on disjoint CPU sets.
- Five model and two actual native GUI checks pass, including 10,000 stable
  records edited/reopened/filtered/exported, atomic invalid batches, replacement,
  migration, saved rules, reviewed edits, real durable worker publication,
  Unicode notes, unchanged loaded deck ownership and unchanged audio-file bytes.
- **105 focused optimized checks pass** across annotations, native editing,
  crates, tags, virtualized views, catalog owner, provider contracts/preview,
  projects and manual synchronization.
- Controlled release gate: **eight workloads, three repeats, zero callback
  allocations and frees**, unchanged expected audio hashes and unchanged policy.
  CPU 6 run: `2026-10-03T05:41:09.273618+00:00` to `2026-10-03T05:44:01.292542+00:00`.
- Native AT-SPI preflight: **158 actions,
  263 visited nodes, 588 App frames**.
- Eight license/package fixtures pass; 492 retained source/build/gate files
  and 346 license entries. `git diff --check` and independent checking of the
  preserved production artifact pass.

## Artifact bindings

Preserved in `/home/michael/Projects/omatainer-work/issue-121-qualified-release`:

- Production: `ee98f4983c589b1117ef48486cb3be2c2b8c45618a55dc494c1b6863f4422f85`
- Release tests: `3b0bdd58a49b5c7718c337f8d624385621d5ea3fbafae7ee7f30dd58b54b48b2`
- License manifest: `41d112690fdd55f1cb640e18d19bd032ac74086d1c1a9c4942f16b939eabacdc`
- Ordinary tests: `e5862dac2467f85bf5035fcd7ba18a8e59e7ddb31e1d97310c5759bd98a7e04f`
- Policy: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`

Local receipts share `/home/michael/Projects/omatainer-work/issue-121-`:
`qualified-ordinary-tests.json`, `qualified-suite.log`,
`qualified-scene-boundary.log`, `release-focused.json`, `release-focused.log`,
`performance.json`, `performance.raw.json`, `performance.log`,
`license-tests.log`, `preserved-check-before-qa.log` and
`preserved-check-after-qa.log`.

No OS-window or physical audio/MIDI-device QA is claimed. Annotation rules cover
these user fields; issue #205's broader smart-crate feature remains separate.
The inherited issue-107 supplemental quiet-host wall limit remains separate.
