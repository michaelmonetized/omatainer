# Native import metadata admission

Base: PR #487, `stack/issue-123-project-import`.

The complete combined native envelope is serialized through the existing bounded
container encoder before import processors are prepared or a SessionEdit is
published. The UI preflight includes its actual view and captured deck identities.
A subsequent project-view change invalidates Review; engine revision changes retain
the existing renderer guard. No file is written by this admission check.

Linux aarch64 focused qualification: four import-model tests, three actual native
App import tests and nine native codec tests pass; one private codec subprocess
entry remains ignored and is exercised by its owning test. Eight package fixtures
pass. The regression fixture constructs two separately saveable documents with
nine unavailable devices carrying 1 MiB states each. Their combined metadata
exceeds the native 64 MiB envelope and is refused without changing the renderer.
The native UI fixture also refuses a changed project view after Review. Existing
native Apply/Undo/Redo/Save/New/Open and allocation-free callback fixtures still pass.

Test executable SHA-256:
`37c6ef8e192f7e1fb680f962688e3281e8191175feed27440a190d6de10d877a`.
Adjacent import-metadata-focused.json/log and package/build logs retain results.
The manifest is refreshed and git diff --check passes. This small follow-up has
focused qualification; it does not claim a separate complete release workload run.
