# Bindings-only validation

Source freeze: `a590d52d12026ad198eb83f4a155bee1099ed94f`. Base: PR #494.

A valid shortcut bundle could fail import or export when another draft setting
was invalid. Bundle validation now uses known-valid defaults and only the bundle's
shortcut fields. Applying it changes only shortcuts and their enabled state.
The native export/import fixture retains an unrelated relative folder entry,
refuses an existing destination without modifying it, and preserves cancellation
and invalid-document behavior. The unit fixture also checks unchanged non-shortcut
fields in an invalid profile.

Locked Linux aarch64 debug qualification: 34 preference/model/storage/native UI
tests and eight license/package fixtures pass. The preserved test executable is
`/home/michael/Projects/omatainer-work/bindings-only-final-qualified-tests`, SHA-256
`3d3df6bb6f85ada40a9dac169c26d1ed4d21953ffec5eab7a1012d0ef003ed60`.
Adjacent final build, manifest, test and package logs retain results. Preliminary
fixture failures used the wrong model buffer and an off-screen text control;
the frozen regression uses the editor's roots buffer and actual native import/export.
No separate full suite or optimized performance qualification is claimed for this
small correction; the parent qualification is retained and the next layer reruns it.
