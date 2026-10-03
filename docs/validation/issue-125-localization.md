# Interface languages and international text: issue 125

Source freeze: `9159cb365765ac1d5cb80cfa74a142b6c823ab6c`. Base: PR #491,
`stack/version-close-folder`.

Preferences retain an interface language per profile. English messages and
numbered templates live in embedded catalogues; Spanish and German translate
primary navigation, project and preferences controls, with English fallback for
advanced help and technical errors. Applying a language does not change musical
values or saved user text. Decimal display/input and UTC calendar ordering follow
the language, including the native F2 numeric editor. Unicode search uses full
case folding and NFC only for transient search keys; source paths, names, tags
and notes retain their original bytes. Stable window IDs, wrapping and scrolling
support text expansion. `docs/localization.md` records translation coverage,
font requirements and bidirectional-text limits.

## Local qualification

Linux aarch64, locked Rust/Cargo 1.98.0:

- 1,365 ordinary tests pass: 1,364 serial (303.96 seconds)
  plus the exhaustive scene-boundary check (257.74 seconds); 30 opt-in ignored.
- The first final serial run passed 1,363 and failed the pre-existing typed-import
  test at its scan-idle assertion. It passed alone, then 20 isolated repeats and
  the complete serial rerun without concurrent compilation. The initial failure
  log is retained; its underlying timing cause is not established.
- 66 optimized focused tests pass: localization, preference migration and actual
  native Apply/Cancel/reopen, date-cache language changes, native decimal-comma
  F2 failure/cancel/apply, canonical annotation searches, Unicode project paths,
  version cancellation/close/restart, empty-folder publication and import bounds.
- The real native Save/Open/Cancel fixture uses combining characters, Japanese,
  Cyrillic and Arabic in a removable-drive-shaped temporary directory. Exact
  stored paths and project names roundtrip. This is a filesystem fixture, not
  proof of a physical removable device or OS input-method behavior.
- Eight license/package fixtures pass. Locales are included in retained source
  bindings; pinned ICU case-mapping dependencies and upstream notices validate.
- Controlled optimized gate passes eight workloads, three repeats each, reviewed
  audio hashes/checks and zero allocations/frees in measured callback workloads.
  CPU 6: 2026-10-03T15:52:51.766468+00:00 to 2026-10-03T15:55:43.481255+00:00.
- Native AT-SPI preflight: 158 actions, 264 nodes, 578 App frames.
- 516 retained source/build/gate files. Independent source, embedded-license
  and performance verification of the preserved production binary pass.

## Artifact bindings

`/home/michael/Projects/omatainer-work/issue-125-qualified-release` retains:

- omatainer: `5ef9998ecb0764363d0c32e60c3b44aa36b7244ba1ac1b447c864264b3449d73`
- release-tests: `fad3d5a0c26bf30c5413f836ad28d519c326737d2a4e9d1e36cff1664d6d6682`
- manifest.json: `0360950159fd8d5a5529035899019685ed67b4d71c2dc1cc23f11b53252458bf`
- notices.json: `14f1064a39065206a8c0a7fb086a7fcd63c7a4a4b041817443160c90bc24b459`
- policy.json: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`
- performance.json: `0b025caa16671fbcca253113224683e2566c72c0cdd1e218ceb2097808f8ce2f`
- Ordinary tests: `068b10911a6319b9c153151cbd050335f9d08a73d73ac6cfc9a3b5ecf7ee0ea5`

Adjacent final-qualified ordinary receipts, optimized logs, package log and
performance JSON/raw/log retain the results. Earlier stale-source/failing
qualification attempts remain separate. Artifacts were preserved before the
next layer compiled.

Actual App controls and renderer run without an OS window. Complete Spanish or
German translation, Arabic shaping/bidirectional cursor behavior, physical media,
installed-font coverage and OS input methods remain unverified or unsupported as
specified in the localization guide. The offline manual remains English.
