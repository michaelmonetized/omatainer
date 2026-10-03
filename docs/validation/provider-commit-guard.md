# Provider project-commit guard and offline guidance

Base: PR #481, `766bb9e66e95f92b420a5185cbace26816d7bef1`.

Search/Preview widgets now remain disabled while the native project commit or
view-confirmation fence is active. Worker admission rechecks that fence after
widget handling. Existing jobs still cancel through the normal poll path.
README, in-app Offline help and its generated manual distinguish supported
explicit online previews from unsupported provider offline storage.

Seventeen focused checks pass: eleven provider/GUI contracts, five preview
renderer checks and manual synchronization. The regression uses the actual
startup project-view installation fence, submits native Search/Preview actions,
confirms no provider worker starts, then confirms normal access resumes after
the real App confirms the project snapshot. Eight license/package fixtures pass.
`git diff --check` passes.

Debug executable SHA-256: `40a052d65c24ac6f69ffed241e42969e8c6bff136387e421d0d9c0383619184b`. Local receipts:
`/home/michael/Projects/omatainer-work/provider-commit-guard-focused.log`,
`provider-commit-guard-license-tests.log` and `provider-commit-guard-debug-tests`.

These are focused follow-up checks. PR #481's prior complete release qualification
remains bound to its own source; the next issue qualifies the combined stack.
No new full-suite, release-gate, desktop-window or physical-device QA is claimed.
