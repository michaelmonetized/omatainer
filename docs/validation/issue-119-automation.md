# Versioned automation and loopback OSC: issue 119

Source freeze: `5ebec0e60ba7231a685d61b6f18b4e79992ad831`. Base: PR #479, `cf2e62d89dc239737276a7a10ca36b90a99669df`.

API v1 discovers typed operations, stable native object IDs, bounded state pages,
read-only subscriptions and structured errors. IDs and counters use strings.
Commands, musical schedules and single atomic undoable rename/color/move edits
return reconnectable jobs with pending/applied/rejected/cancelled completion.
Accepted admission is separate from renderer completion. The renderer rechecks
identity, safety and edit revision; concurrent stale edits cannot both commit.

The Linux Unix socket has mode 0600 and an effective-UID peer check. OSC is
optional, disabled by default, and binds only 127.0.0.1 UDP. Its random 128-bit
token stays in memory and rotates on restart or disable/re-enable. Strict OSC
message parsing precedes the shared API; bundles and OSC subscriptions reject.
The [protocol guide](../automation-api.md) documents the exact formats, limits,
errors, access boundaries and original specification references.

A 64-entry fixed schedule dispatches at musical sample boundaries without
network I/O or heap ownership release on the callback. Scheduled gain/pan and
crossfader changes update mixer caches at that sample. Stop, safety changes,
original-target deletion and every successful project installation invalidate
obsolete requests. Reopening the same project or a saved copy also advances the
transport epoch, despite retaining its native namespace and IDs.

The native Automation panel exercises state, atomic track naming, gain scheduling
and job refresh/cancel through the same API. Preferences version 9 migrates older
profiles with OSC disabled. Preview/Apply persists listener intent; Cancel and
external-file save conflicts preserve the actual listener and token. A failed
port change preserves the earlier listener and supports retrying saved settings.

## Final local qualification

Linux aarch64, locked Rust/Cargo 1.98.0 builds, actual App/renderer and Unix/UDP
services. Final-source checks:

- **1,300 ordinary tests pass**: 1,299 serial, zero failures, 29 opt-in ignored,
  262.65 seconds; exhaustive scene-index test passes separately in 256.62 seconds.
  Independent processes use the same immutable executable on separate CPUs.
- **186 optimized checks pass**, covering the API/OSC/native panel, CLI exchange
  and subscription functions, scheduling, project installation, preferences,
  storage, legacy IPC, accessibility, projects, templates, timing and portability.
- **57 focused debug checks pass**. Real clients exercise disconnect/reconnect,
  malformed input, unsupported future-version requests, UID guard failures,
  bounded jobs/pages, cancel conflicts and concurrent atomic edits with native Undo.
  Malformed subscription success fields reject before reaching CLI output.
- Actual nonzero scheduled audio in one 64-frame block is bit-for-bit equal to
  native gain commands issued at the corresponding reference sample; measured
  callback allocations and frees are zero. The real project worker reinstalls
  identical namespace/IDs, rejects the old job, resumes playback without that
  mutation, and performs no callback heap work, including counter wraparound.
- Controlled gate: **eight workloads, three repeats, zero callback allocations
  and frees**. Original hashes remain producer `26f84beea80f5ec5`, composer
  `e2f197b02633bd3d`, live DJ `d7711a2dc3b32a39`, hybrid `adc9540dab057fed`.
  CPU 6 run: `2026-10-03T04:00:17.597139+00:00` to `2026-10-03T04:03:09.440851+00:00`,
  after other builds/tests finish. Policy is unchanged.
- Native AT-SPI preflight: **158 actions, 256 visited nodes, 577 App frames**.
  Eight license/package fixtures pass; inventory retains 479 source/build/gate
  files and 330 license entries. `git diff --check` passes.
- Independent checking of the preserved production artifact passes before and
  after the validation commit.

Review regressions are covered explicitly: unsupported versions are checked
before version-specific parsing, and installing an identical project invalidates
old musical jobs. Earlier candidate and intermediate qualification logs remain
separate; the results above bind the final source freeze.

## Artifact bindings

Preserved in `/home/michael/Projects/omatainer-work/issue-119-qualified-release`:

- Production: `1bac307903e6d23ad439286322090544beb1c8b1675728c832853bfb48d2b44a`
- Release tests: `03d242864b9b2bb4b1d26e94eb5b51e6ae699da31fcc2b716fa6be6d7ba454ce`
- License manifest: `66138f727f16a4c4ab36cced26efa0fa25b91f650c2b66dc54c3dcbe352df3bb`
- Ordinary tests: `35b86834b47550a9e337ecffe3bb926efc17651713887140b97d7296a4e9c8f3`
- Policy: `fa1fb85c8f5c9aeac076920eaaf7eec5131a8ebe7019b3f81d6fa1e07ac9dd6c`

Local receipts share `/home/michael/Projects/omatainer-work/issue-119-`:
`qualified-ordinary-tests.json`, `qualified-suite.log`,
`qualified-scene-boundary.log`, `qualified-focused.json`, `release-focused.json`,
`performance.json`, `performance.raw.json`, `performance.log`,
`license-tests-qualified-final.log`, `preserved-check-before-qa.log` and
`preserved-check-after-qa.log`.

No OS window, physical audio/MIDI device, foreign-user process or external
network-control QA is claimed. The peer test uses real kernel credentials and a
mismatching permitted UID. The inherited issue-107 supplemental quiet-host wall
limit remains separate; this standard gate does not replace it.
