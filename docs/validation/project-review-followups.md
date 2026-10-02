# Project review follow-ups: issues 114 and 115

Source freeze: `0f645c95fe07fd45c3b2f6ac14174535689fa027`.

New, Open and Recover refuse pending source choices and timing drafts before
starting, after an unsaved-save continuation, and immediately before prepared
replacement commits. The editor opens its existing explicit Keep/Discard
controls; the user repeats the project action afterward. A settled timing editor
is cleared on installation so reopening captures the new session namespace.
Applied music and aliases remain intact when replacement is refused.

Relink verification returns the complete captured asset-key set. Applying a
reviewed batch prunes obsolete aliases before merging the selected replacements,
so deleted assets cannot exhaust the 256-entry source-reference limit.

Source searches charge actual decoded PCM for successful files. Failed decodes
retain conservative work credit; unsupported/non-audio failures before decoding
release that credit and are counted separately. Damaged recognized audio,
permissions, changed files/mounts, bounds and other read failures remain explicit
partial searches. Header checks do not seek or suppress successful decoding of
renamed files. Skip decisions recheck the retained file and mount identities.

Pickup-aligned meter changes tolerate the shared beat epsilon before rounding
an integral bar count. The 960-PPQN, 3/4, 1.025-pickup, 4.025-change regression
reports bar 2. Genuine fractional prior bars still count.

## Local verification

Linux aarch64, Rust 1.98.0, native optimized debug test profile, locked dependencies.
The shipped egui handlers are exercised through their actual App and renderer,
including menu actions, explicit discard, relink application, native persistence,
and a timing edit made while project preparation is paused before commit.

66 checks pass: dependency core 3, dependency GUI 4, timing GUI 5, timeline 6,
project workflows 34, Help/manual 14. The manual regeneration entry is opt-in.
The mixed-folder fixture contains sidecars, renamed audio, damaged WAV and 81
matching small sources alongside a real 128-MiB unresolved PCM identity.

Test executable SHA-256:
`54aa379f6a9459d10fa6baeb5f4a322754974d09bfc8a1876b0f9cee147f0aa0`.
License manifest SHA-256:
`d4f134b65c05c08f8109b429cae95cf140ba405c75159c75240be137a5bf281a`.
License inventory/check and `git diff --check` pass. Logs are retained under
`/home/michael/Projects/omatainer-work/project-review-followups-*`.

These are native software/GUI fixtures, not physical device or listening proof.
This layer does not claim a new full release gate; issue 116 will qualify the
combined stack after rebasing. The prior issue 115 receipt remains bound to its
own earlier source freeze.
