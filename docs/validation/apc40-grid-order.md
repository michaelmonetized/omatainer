# APC40 mkII grid row order

The APC40 mkII's top grid row sends notes 32–39. Subsequent rows send 24–31,
16–23, 8–15 and 0–7. Omatainer previously assigned the lowest notes to its first
scene. A native MIDI clip in scene 1, track 8 therefore sent loaded-color
feedback to note 7. Michael confirmed that the physical bottom-right pad,
scene 5, lit.

The correction applies this captured row order to the renderer's input route,
the outgoing LED addresses and the factory mapping's displayed targets. The
original APC40's channel-based grid addresses retain their existing behavior.

The regression uses the captured note table to check all forty factory targets
and actual input-worker launches and shifted selection. It repeats those checks
with track and scene banking, verifies that releases do not toggle launches,
and refuses pads outside the session. Feedback checks cover loaded, playing
and empty cells, including a partially filled bank at the session boundary.

## Physical capture before the correction

The complete grid pass captured 80 press/release messages for all forty pads
and 16 messages for all eight Clip Stop buttons. All 96 commands were admitted
and consumed, with no input drops, command refusals, underruns or callback
deadline overruns during that interval. The empty grid caused no loaded-clip
LED changes or playback. This pass proves input receipt, not loaded-clip
launch behavior.

The five Scene Launch buttons and Stop All Clips produced no raw input in the
requested pass. Earlier Pan, Sends and User checks also produced no physical
selector input in either host mode or isolated generic mode. Software-selected
Sends and User modes lit their corresponding physical LEDs. The cause of the
missing global button inputs remains unresolved; the row correction does not
claim to repair them.

These pre-correction captures belong to code
`7db2dc58faa4962f2eff824cde2a7374d2fbafea`, executable SHA-256
`617c8161972f638d5f83b25c52e975f8a119ffdb461f23c7c77067d52674add1`.
The GUI restarted during the requested top-right press check; no corresponding
physical press was captured. That check must be repeated on the corrected build.

## Correction qualification

The correction is isolated at source
`935d1ebe7a2e8aacbc7e1f1f996051434b97f161`, based on the separately qualified
deck-quantization release `f1ff6e5d09d8b5ca25eefd700db9e012477cec3a`.
The verified ARM64 release package has manifest SHA-256
`28c1ef2075a8b67640abcfb6234a583610a1dfd3b28a16352ba0bb236d7c4df1`.
The package, installed executable and running GUI match executable SHA-256
`81f3e6af03e5a37d8da168bef9cde8ee797c2f7e1b32647ca4c6d2c8a2057b16`.
The GUI uses a private copy of that verified install so another software
installation cannot replace its executable during the physical check. The
saved native project restored both loaded decks at their retained positions,
paused, with master zero.

The first MIDI run passed 134 tests and failed one older factory-profile test
that expected the reversed MkII row order. Both new grid regressions passed.
That existing test now uses the captured row table. Its corrected test source
is `725ff8990d9aa33adbc784927305a652f62a4779`; its production source matches
the installed release byte for byte. An interrupted normal-profile rebuild
and a runner attempt selecting the old executable remain recorded separately.
The final optimized run on the release-source executable passed **134 MIDI
tests with zero failures**, retaining two connected-capture ignores and
excluding only the changed factory-profile fixture. The freshly compiled
corrected fixture separately passed on the normal test-thread stack. Together
they cover all 135 non-ignored MIDI cases. The source comparison confirms that
only that `cfg(test)` fixture changed after the release build.

The corrected factory fixture uses reduced optimization and debug-information
flags for the test crate; dependencies retain their existing test profile. A
broader attempt with those reduced flags aborted on a stack overflow in an
unchanged routing fixture. That routing case passed in the optimized run.
The receipt retains these failed and interrupted attempts. No larger test
thread stack, callback stack or ordinary-suite performance claim was introduced.

## Physical check after the correction

A new one-note MIDI clip in scene 1, track 8 sent loaded feedback to note 39.
Michael confirmed that the top-right pad lit yellow, turned green when pressed,
and returned to yellow after track 8's Clip Stop. The independent capture
contains note 39 press/release and channel 7 note 52 press/release. Outgoing
note-39 colors changed to 21 and then 13. The native grid showed the stopped
test clip after Clip Stop. All four incoming messages were admitted and
consumed, with zero input drops or command refusals.

That interval also had 13 underruns and 12 callback deadline overruns while
other software builds were running. It does not qualify stage audio stability.
Transport was explicitly stopped after capture; master remained zero and both
decks remained paused. The unresolved global buttons retain their earlier
physical-input boundary.

The [row-order receipt](apc40-grid-order-receipt.json) records the source,
package, test and physical-capture identities separately.

Earlier Pioneer/APC checks retain their original provenance in the
[live controller receipt](live-controller-acceptance-receipt.json).
