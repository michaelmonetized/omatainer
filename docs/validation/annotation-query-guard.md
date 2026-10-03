# Annotation predicate syntax follow-up

Source: `5d945f6b2d3f08f847d420fbe63a2c9f6b4f606a`. Base: PR #483.

Reserved color and rating operator forms now validate their complete syntax.
`color:red`, empty colors and unsupported rating operators return visible errors.
Plain words such as `rating`, `color` and `colorful track` remain ordinary text.
Valid combined rating/color/tag predicates retain their existing behavior.

Thirteen focused checks pass: six annotation model/query checks, two actual
native editing checks and five virtualized library-view checks. Eight
license/package fixtures and `git diff --check` pass.

Debug artifact SHA-256:
`858d17c7e89d11ae3fcabb461a0849e84b69af63ad5c9c5c666fea8a3c6ce970`.
Local receipts: `/home/michael/Projects/omatainer-work/annotation-query-guard-focused.log`,
`annotation-query-guard-license-tests.log` and `annotation-query-guard-debug-tests`.

These are focused follow-up checks. PR #483's complete qualification remains
bound to its own source; issue #122 qualifies the combined stack. No new full
release gate, OS-window or physical-device QA is claimed.
