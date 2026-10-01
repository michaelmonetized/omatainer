# Issue 47: nonfatal UTF-8 theme colors

The color parser operates on an exact six-byte ASCII pattern and validates
hex nibbles. It never slices a UTF-8 string by byte offset. Accepted formats
are RRGGBB and #RRGGBB, case-insensitive, with surrounding whitespace ignored.
Shorthand, alpha channels, repeated #, embedded whitespace, nonhex characters
and trailing characters are rejected instead of silently truncated.

The production startup/reload color-file path preserves each invalid entry's
previous color, applies other valid entries and writes a nonfatal diagnostic
to stderr with the file path, key, required format and fallback behavior.
Missing optional keys remain silent. Diagnostics do not print arbitrary raw
color values. A corrected later reload recovers normally.

Validation on assembled issue39 prerequisite:

- 232 Rust tests and a production build pass locally.
- Three new test groups cover every channel value/case, documented whitespace,
  invalid formats, every non-ASCII Unicode scalar at every fitting six-byte
  boundary, and 20,000 seeded arbitrary UTF-8 strings.
- A private real TOML file is loaded, changed to mixed valid/invalid colors,
  and corrected. Assertions verify unchanged fallback colors, unaffected
  valid colors, diagnostic keys/messages and recovery. The same reload_colors
  function is called from production reload; no user theme file is modified.
- `python3 scripts/check-theme-colors.py` compiles the actual production parser
  with rustc -O -C panic=abort and repeats exhaustive Unicode boundary checks.
  It passes without unwinding or launching the desktop.
- `git diff --check` passes. This fixes parser safety; it does not claim live
  desktop theme switching was exercised.
