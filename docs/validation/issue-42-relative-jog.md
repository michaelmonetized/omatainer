# Issue 42 — explicit relative jog encodings

The previous relative decoder produced only nonnegative values. Each relative
binding now carries its encoding and sensitivity. The two supported encodings
have different neutral values and never infer a format from the incoming byte:

| Encoding | Reverse | Neutral | Forward | Full signed range |
| --- | --- | --- | --- | --- |
| Offset binary | `0x3f` = -1 | `0x40` | `0x41` = +1 | -64 through +63 |
| Seven-bit two's complement | `0x7f` = -1 | `0x00` | `0x01` = +1 | -64 through +63 |

All six existing Pioneer jog bindings (channels 0/1, CC `0x21`/`0x22`/`0x23`)
explicitly select offset binary with 0.35 engine delta per count. Sensitivity is
an application setting retained from the old positive path, not a measured
physical rotation calibration. Neutral produces no command, so it cannot cancel
an existing scratch velocity or pitch nudge. Profile validation rejects missing,
misplaced, nonpositive, or nonfinite relative metadata. Invalid data bytes cannot
reach jog commands through the relative decoder.

## Protocol evidence

- Pioneer/AlphaTheta's [DDJ-FLX4 MIDI message list](https://www.pioneerdj.com/-/media/pioneerdj/software-info/controller/ddj-flx4/ddj-flx4_midi_message_list_e1.pdf),
  [DDJ-400 MIDI message list](https://downloads.support.alphatheta.com/software_info/dj-controllers/DDJ-400/DDJ-400_MIDI_Message_List_E1.pdf)
  and [DDJ-SB3 MIDI message list](https://downloads.support.alphatheta.com/software_info/dj-controllers/DDJ-SB3/ddj-sb3_midi_message_list_e2.pdf)
  specify centered difference counts for the wheel side and platter, with
  clockwise counts above 64 and counterclockwise counts below 64. These support
  the three existing unshifted jog addresses for the first two decks. They do
  not prove every controller matched by the broad legacy Pioneer name pattern,
  every shifted control, or every hardware/firmware mode.
- Akai's [APC40 MkII communications protocol v1.2](https://cdn.inmusicbrands.com/akai/attachments/apc40II/APC40Mk2_Communications_Protocol_v1.2.pdf),
  page 37, defines the other signed encoding. It is implemented and exhaustively
  tested as explicit binding metadata. This change does not assign new actions
  to APC relative controls; the limited APC profiles from issue 38 remain intact.

## NS7 scope and remaining work

The original NS7 and NS7FX profiles previously guessed that CC `0x21` on channels
0/1 and pitch-bend were jog messages. Those bindings are removed, rather than
given an invented relative encoding. The displayed profile names explicitly say
that wheels are unmapped. Other legacy NS7 controls are unchanged.

The [Numark reference manual](https://www.numark.com/images/product_downloads/ns7_reference_manual___v1.1.pdf)
does not specify the wheel byte format. The primary open-source Mixxx mapping
was also inspected at revision `53ada03f500f8c7f2842c1ab59befa9f32efd8bd`:
its [NS7 XML](https://github.com/mixxxdj/mixxx/blob/53ada03f500f8c7f2842c1ab59befa9f32efd8bd/res/controllers/Numark%20NS7.midi.xml)
routes wheel position on channel 0, CC `0x00` (left) and `0x02` (right), and its
[NS7 script](https://github.com/mixxxdj/mixxx/blob/53ada03f500f8c7f2842c1ab59befa9f32efd8bd/res/controllers/Numark-NS7-scripts.js)
derives movement from successive position reports with wraparound. This is a
stateful absolute-position protocol, not the relative decoder fixed here. No
Mixxx implementation was copied into Omatainer.

NS7 motor/touch initialization, absolute-position decoding and NS7II's distinct
protocol remain unimplemented. Issue 301 covers motorized platter control and
calibration; issue 252 covers qualified hardware profiles. Those later layers
must provide a separate NS7II profile with captured device vectors and hardware
validation. Device-name matching alone must not be
reported as NS7II support. User hardware QA remains pending for every profile,
including exact Pioneer model, mode, wheel direction, sensitivity, scratching
and motor behavior; no physical wheel proof is claimed by this change.

## Local validation

Five new tests cover all 128 wire values for both encodings at three sensitivity
settings, all six factory relative bindings, rejection of invalid metadata and
non-data bytes, and the deliberately unmapped NS7 guesses. The original
`handle_msg` path drives the real command queue and engine for both decks:
stopped seeking, touch scratching, untouch pitch nudging, forward/reverse output
playhead movement and neutral preservation. The opposite deck stays unchanged.

All 163 tests and the production build passed. Validation commands use a private
reused Cargo target and no MIDI/audio devices:

```sh
CARGO_TARGET_DIR=../issue-38/target cargo test relative -- --nocapture
CARGO_TARGET_DIR=../issue-38/target cargo test
CARGO_TARGET_DIR=../issue-38/target cargo build
git diff --check
```
