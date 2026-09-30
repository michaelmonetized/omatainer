# Issue 38: single-destination APC40 faders

The duplicate channel-zero CC7 registrations are removed. Both APC40 profiles
now emit one `TrackGain` command for one track fader. The original and mkII have
separate clip-grid definitions and names; the more specific mkII matcher takes
precedence over the original model's name.

## Address evidence

Akai's [APC40 protocol, revision 1, pages 16–18](https://cdn.inmusicbrands.com/akai/apc40/APC40_Communications_Protocol_rev_1.pdf_1db97c1fdba23bacf47df0f9bf64e913.pdf)
documents CC7 on channels 0–7 for track levels and five per-track clip notes
0x35–0x39. Akai's [APC40 Mk2 protocol, version 1.2, pages 30–34](https://cdn.inmusicbrands.com/akai/attachments/apc40II/APC40Mk2_Communications_Protocol_v1.2.pdf)
retains those fader addresses but uses forty distinct grid notes 0x00–0x27.
Global note controls do not use a track-channel discriminator. Channels here
are zero based, matching the status byte's low nibble.

| Input | Original profile | mkII profile |
| --- | --- | --- |
| Track fader `t`, 0–7 | CC7, channel `t` | CC7, channel `t` |
| Clip scene `s`, track `t` | Note `0x35+s`, channel `t` | Note `8*s+t`, any channel |
| Scene `s`, 0–4 | Note `0x52+s`, any channel | Note `0x52+s`, any channel |
| Master fader | CC14, channel 0 | CC14, channel 0 |

The previous record-arm→mute and track-select→deck-play guesses are removed.
The limited APC40 profiles explicitly ignore unmapped notes, including those
buttons, instead of falling through to live musical notes. Other controller and
keyboard profiles retain their prior live-note behavior. Mode negotiation,
output LEDs, arm/solo/activator/select/stop buttons, encoders and the other
unmapped controls are not implemented by this change.

## Validation

Every factory profile is validated before MIDI input callbacks are connected.
An invalid factory map returns an error through startup. Validation rejects
duplicate addresses even when the action is identical, conflicting actions,
wildcard/exact-channel overlap, CC/CcRel aliases, and pitch-bend aliases whose
data byte dispatch ignores. Invalid channel/data ranges are also rejected.

Applying this check to all eight factory profiles exposed an existing MPK CC1
conflict: it controlled both FX wet and deck filter. CC1 keeps its existing
filter assignment; the duplicate FX assignment is removed. Other MPK addresses
are unchanged. This correction is a profile consistency fix, not a new claim
about compatibility with every device named MPK/MPC/MPD/LPD.

`cargo test engine::midi::profile_tests` covers:

- Both generations, every MIDI channel and every CC address, all 128 values at
  the fader CC, and representative minimum/middle/maximum values elsewhere.
- Actual queued dispatch through `RtEngine::process`: only the intended track's
  gain changes, exactly one command is received, and master gain stays fixed.
- All forty clip addresses for each generation, note-off/velocity-zero release,
  incompatible grid addresses, global scene channels and ignored controls.
- Factory-wide validation, exact/conflicting/wildcard/decoder-alias negative
  cases, distinct valid addresses, and preserved generic keyboard input.

The local full suite passed 158/158 tests, including eight new profile tests.

These are synthetic software tests grounded in the published protocols. No
APC40 was connected during validation. Automatic name matching is only profile
selection; physical fader travel, grid orientation, device mode, feedback and
the user's APC40 mkII still require the final hardware QA run.
