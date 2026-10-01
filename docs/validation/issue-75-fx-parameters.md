# Issue 75 — implemented FX controls

The FX panel now draws controls from `FxId::controls()`. Each entry states its
stored index (or common wet mix), visible name, units, physical range, DSP
consumer and help text. UI values convert to the existing normalized storage;
no processor algorithm or preset value changed. Engine commands use the same
metadata to reject hidden indices, Arp wet mix, and nonfinite values.

| Effect | Exposed controls besides wet mix | Actual consumer |
| --- | --- | --- |
| Comp | Threshold 5–45% peak; Compression 0–100% | Linked envelope threshold; blend between unity and threshold reduction |
| Spread | Width 0–200%, neutral 100% | Existing mono/Haas width and delay law |
| Balance | Balance −100–100%, neutral 0% | Square-root left/right channel attenuation |
| Reverb | None | Fixed room network; common wet mix only |
| Chorus | None | Fixed 0.7 Hz, 2–14 ms modulation; common wet mix only |
| Delay | Feedback 0–100% | `Delay::fb`; time remains fixed at 250 ms |
| Gate | Threshold 0–100% peak | Linked envelope comparison; below threshold gain is 5% |
| Arp | On/off only, no wet mix | Upstream MIDI sixteenth-note stepping |
| Drive | Drive 1–9× | Input gain before `tanh` |
| Filter | Mode 0–2 (LP/BP/HP); Cutoff 120–8120 Hz | SVF morph and cutoff |
| EQ3 / EQ5 / EQ8 | Low/Mid/High gain 0.25–1.75× | Interpolated lower/middle/upper gains over 3/5/8 bands |

Arp cannot be added to a scene audio bus, where no MIDI event consumer exists.
Its track panel explains its MIDI-only role. Fixed settings are visible below
Reverb, Chorus, Delay and Arp. Removed sliders are not disguised as working
controls; in particular, no delay-time implementation is claimed.

## Validation

- A table iterates all 18 exposed parameter indices and 12 wet controls at
  normalized 0, 0.25, 0.5, 0.75 and 1. Each runs 48,000 heterogeneous stereo
  frames at 48 kHz against independently constructed primitive references.
  Every adjacent sweep differs by summed absolute output error >0.01; every
  reference sample agrees within absolute 1e-6. Neutral processors use a
  nonneutral parameter setting when their wet control is swept.
- Delay p0 endpoints produce identical complete impulse output at 44.1, 48 and
  96 kHz. The first echo is exactly 250 ms (sample 12,000 at 48 kHz), and five
  feedback settings produce the expected second-echo amplitude.
- All 256 parameter indices are checked against the declared controls for each
  effect. Hidden indices and nonfinite values leave state unchanged.
- The actual renderer produces the same ascending Arp steps with legacy mix 0
  and 1. Its supported on/off control switches between steps and the held chord.
- Actual egui pointer edits exercise all 30 sliders through `App::fx_row`, real
  command admission and `RtEngine::apply`. AccessKit nodes expose the declared
  label, minimum, maximum and current physical value. Passive redraws submit
  no commands. There are exactly as many sliders as declared controls.
- Painted fixed-setting explanations and the absent scene Arp button are
  checked, as is renderer rejection of a direct scene Arp add command. Existing
  scene ownership and exact stereo bypass tests use supported scene controls.

These are deterministic software checks. They do not establish subjective
sound quality, physical controller operation, or XRUN performance.

Final local validation on this issue branch: `cargo test` passed 356 tests
(2 existing ignored benchmarks); `cargo build` and `git diff --check` passed.
A separate read-only peer review found no blocker. No hardware or installed
application was changed for these checks.
