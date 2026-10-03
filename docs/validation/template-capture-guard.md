# Template capture and project replacement guard

Source: `6defebe2443040184cd6e82e2fecf1f9a79709da`. Base: PR #476, `6a589f2b400f346df8d286a0ca2368d10740ac29`.

Template capture is unavailable throughout project preparation and installation. New/Open/Recover/template replacement also waits for an existing template operation to finish or be cancelled. This keeps worker captures paired with the UI selection and view from the same session, whichever operation starts first.

A deterministic native GUI regression pauses each worker in turn, tries the conflicting action through AccessKit, verifies disabled capture controls / rejected replacement, resumes the worker, and confirms completion. The changed-template regression clears the earlier duplicate error, rewrites a valid template after inspection, and requires the new identity rejection in the project result.

## Local validation

Linux aarch64, Rust/egui App and actual renderer fixture; no external devices, desktop window or human screen-reader claim.

- 55 focused tests passed: templates 7, projects 35, timing 5, dependency editor 5, portability 3. The archive tests invoke their private fresh-process paths.
- Eight license/package fixtures passed; retained provenance source inventory checked.
- `git diff --check` passed.
- Immutable test executable SHA-256: `9e37bf4881f35ee178de30522aa082d0e7909c83262300e54d9dad0aac44a272`.
- Receipts: `/home/michael/Projects/omatainer-work/template-capture-guard-focused-final.json`, matching `.log`, `template-capture-guard-license-check.log`, `template-capture-guard-license-tests.log`.

This small GUI coordination change does not claim a fresh complete suite or controlled performance gate. PR #476 retains its source-bound full qualification; #118 will qualify the combined stack.
