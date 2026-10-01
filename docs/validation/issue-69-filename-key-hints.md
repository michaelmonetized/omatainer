# Issue 69: preserve complete filename key hints

Filename key parsing retains ASCII and Unicode sharp/flat accidentals and minor
qualities. For example A#, C#m, Bb, Db, C♯minor and E♭min become A#, C#m, Bb, Db,
C#m and Ebm. Major quality is the default; m/min/minor and M/maj/major suffixes
are accepted, with uppercase M meaning major. A separated minor/major word is
also supported. No enharmonic conversion or musical-key analysis is performed.

Only complete tokens count. Unknown attached symbols, unsupported accidentals,
combining marks, chord suffixes, repeated accidentals and malformed qualities
cannot be removed to invent a different pitch. Conflicting recognized key hints
produce unknown rather than silently choosing the last one.

To avoid reading ordinary title letters as keys, a bare natural key needs a
`key` marker, matching brackets, or a filename containing only that key. Explicit
qualities/accidentals identify other hints; unmarked lowercase natural words
such as `am` remain title text. For example `A beautiful day 128` has unknown key
and retains its independent 128 BPM filename hint. Unknown keys display a dash.
The crate visibly labels file keys as hints, with a tooltip explaining these
limits. Existing session/builtin key metadata is unchanged.

Validation: all 301 local Rust tests and production build pass. Two table-test
groups cover supported sharp/flat/minor notation, natural-key contexts, Unicode,
incidental title text/numbers, malformed symbols and ambiguity. Independent review
reproduced unsupported-symbol and mismatched-bracket cases; all now have passing
regressions. The changed module passes rustfmt and the diff passes whitespace
checks. No filename hint is presented as verified musical-key analysis.
