# Light contrast text cursor

Source: `2946b8b52d64997e6e3e6cbc85fb5bbea4d83b08`. Base: PR #478, `f81832f1ed913c6a4149df3c328a46146ff1f41a`.

Light contrast previously inherited egui's dark text cursor, text coverage and shadows. The pale blue cursor failed the 3:1 non-text contrast requirement on the near-white text edit background. The light preset now starts from `Visuals::light()` before applying Omatainer's explicit palette. Dark and desktop presets retain their existing base.

A real egui focused TextEdit fixture paints both presets and verifies the cursor stroke against the actual painted input background, with matching native shadows and text coverage.

- 24 focused tests passed: theme/display 3, native display 4, preferences 17.
- Eight license/package fixtures passed; source inventory validation and `git diff --check` passed.
- Immutable debug test SHA-256: `05dbfcb5be2c1e269d8629f2a590b56e4c30c06e60b99aae7881610d42f41849`.
- Receipts: `/home/michael/Projects/omatainer-work/light-theme-cursor-focused.log`, `light-theme-cursor-license-tests.log`, `light-theme-cursor-license-check.log`.

Linux aarch64 App/renderer fixtures; no OS window or human vision claim. This small style correction does not claim a new full suite or controlled performance gate; #119 will qualify the combined stack.
