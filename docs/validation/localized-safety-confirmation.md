# Localized safety confirmation controls

Source freeze: `76711c3f69208273ab7365113ea813ed55c355d1`. Base: PR #492 (`stack/issue-125-localization`).

The native safety dialog now looks up its selected confirmation action and
explanation in the current catalogue. Existing Spanish/German confirmation
translations become visible without changing their commands, acknowledgment
requirements, cancellation or latched emergency mute.

Linux aarch64, locked Rust/Cargo 1.98.0: five actual App safety tests and three
catalogue tests pass, plus eight license/package fixtures. The new native fixture
runs both Spanish and German through protected-mode cancellation/leaving, safe
stop, input-release acknowledgment, emergency cancellation/silence and recovery
that preserves the output mute. Actual AccessKit actions reach the renderer;
physical input release is deliberately a fixture acknowledgment.

Qualified ordinary test binary: `a3886afc52ff851d0a16fa01c3702302df706010badd7806c5e69555873a6ffc`,
retained at `/home/michael/Projects/omatainer-work/localized-safety-qualified-tests`.
Adjacent `localized-safety-{native,catalog,package}.log` files retain results.
No separate complete suite or performance gate is claimed for this display-only
layer; PR #492's frozen qualification remains its parent receipt.
