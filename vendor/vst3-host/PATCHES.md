# VST3 host timing patch

Pinned crates.io 0.9.0, MIT. The unmodified crate and all original file hashes are in UPSTREAM.json.

Omatainer adds an explicit in-process transport seek used only inside its own isolated child. The dependency helper transport returns an error rather than ignoring the seek. Musical time advances from the current quarter-note position instead of recalculating all prior time at the current tempo. This preserves seeks and tempo changes.

The public `encode_project_state` wrapper exposes the existing component/controller state encoder for reviewed migration. Its format, validation and decoder are unchanged. This allows retained Live VST3 state to use the same bounded host-state contract as an ordinary native checkpoint. Only the files declared in UPSTREAM.json's patch list are modified; all other published source hashes remain checked by the license inventory.
