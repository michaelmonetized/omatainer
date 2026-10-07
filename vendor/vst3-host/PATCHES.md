# VST3 host timing patch

Pinned crates.io 0.9.0, MIT. The unmodified crate and all original file hashes are in UPSTREAM.json.

Omatainer adds an explicit in-process transport seek used only inside its own isolated child. The dependency helper transport returns an error rather than ignoring the seek. Musical time advances from the current quarter-note position instead of recalculating all prior time at the current tempo. This preserves seeks and tempo changes. All other published code is unchanged.
