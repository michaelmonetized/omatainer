# Issue 78 — reload every applied theme/font source

The old GUI check watched only `colors.toml` and synchronously invoked font
selection while ignoring that selection during installation. The new loader
runs one worker and publishes one latest complete theme/font snapshot. The GUI
only polls ready updates, applies text styles, and installs prepared font data.

Each pass reads the configured color path and its sibling `shell.toml`, then
resolves `fc-match monospace` afresh. This observes the canonical fontconfig
selection, including included configuration, without guessing which particular
configuration file changed. The installed local `omarchy-font-current` command
uses that same fontconfig selection; `omarchy-font-set` writes fontconfig. No
Omarchy command, filesystem probe, or font-byte read runs in a GUI/audio callback.

Checks wait 800 ms after the preceding pass. Results coalesce into one complete
latest snapshot; an intermediate font update cannot be lost behind a later
color-only update. Repeated explicit test requests also coalesce to one pending
slot. Font identity includes device/inode, length, mtime and ctime nanoseconds;
changed bytes reload even when the selection path/name stays the same. Reads
verify identity before/after and fonts are checked with egui's `ab_glyph` parser.

Syntactically incomplete files retain their preceding valid settings. Invalid
individual color fields retain their own preceding values. Invalid fontconfig
output, font bytes/indices, or unavailable files retain the preceding font.
Later passes retry, so recovery does not require touching colors. Shell
`font.base-size` accepts finite numeric values from 4 through 96 points; standard
text styles derive from it, and existing fitted graphical labels keep their
explicit geometry. Font definitions are replaced only when font data changes.

Resource limits are 64 KiB per text input and 32 MiB per font file. Fontconfig
stdout/stderr each have 4096-byte bounds, with a 500 ms process deadline. Failed
or timed-out children are killed/reaped; stderr diagnostics invalidate that
attempt. File access itself cannot be forcibly cancelled, so dropping the UI
releases its worker handle without waiting; the worker retains its data until
the operation returns and then exits. Changed diagnostics are logged off-thread.

Validation uses private directories and bundled test font assets:

- Color bytes and timestamp stay unchanged while shell-only edits change the
  actual egui Body/Button/Monospace size and rendered label dimensions.
- Font-only edits replace selected font bytes in both text families and change
  actual egui text geometry. Changed bytes at the same selected path also reload.
- Real `fc-match` uses a private `FONTCONFIG_FILE`, private font directory and
  private cache; selection changes and malformed XML recovery are verified.
- Damaged colors, shell, font bytes, invalid/nonfinite sizes, missing files and
  oversized files retain the last valid values, then recover on the next pass.
- A thousand requests coalesce while resolution is held; later complete state
  includes the selected font and final color/size together.
- Actual application frames and master control commands continue while the
  resolver is held; worker teardown does not wait for it.
- Private subprocess fixtures verify timeout, output-cap and malformed-result
  handling. No live endpoint, desktop configuration or installed font is changed.

Issue 79 layers the selected-font fallback/glyph contract and its specific
coverage on this shared implementation; this change supplies the font reload
mechanism required by issue 78.

Results: `cargo test` passed all 312 tests, including ten theme-focused tests;
`cargo build` passed. These are local headless egui/process fixtures, not a claim
of external desktop-session or physical-device QA.
