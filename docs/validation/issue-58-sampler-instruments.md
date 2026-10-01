# Issue 58: sampler choices match their named instruments

The UI, command and snapshot share `SamplerInstrument`: sample banks or a typed
`SynthInstrument` (Analog, Keys, Pad). DSP voices use the same synth identity with
exhaustive oscillator/envelope matches. There is no numeric clamp or catch-all
synth alias. The unsupported Drums synth choice is removed; Kit, Perc and Hits
sample banks remain available through Samples. Existing track/demo synth choices
are preserved during the type conversion.

| Menu choice | Source | Attack / decay / sustain / release | Base cutoff |
| --- | --- | --- | --- |
| samples | One-shot in the selected Kit/Perc/Hits bank | Sample's recorded/generated envelope | Sample source |
| analog | 70% saw + 30% square | 5 ms / 180 ms / 0.35 / 120 ms | 700 Hz |
| keys | 35% saw + 65% sine | 8 ms / 220 ms / 0.45 / 280 ms | 1800 Hz |
| pad | 60% sine + 40% saw at 0.997 frequency ratio | 40 ms / 400 ms / 0.7 / 800 ms | 1800 Hz |

The synth retains its existing linear ADSR and envelope-modulated low-pass. The
release value describes the existing full-scale decay coefficient; a voice
released below full level finishes sooner. Menu descriptions distinguish the
sound and articulation without presenting implementation details as controls.

Changing the selection affects future pad onsets. Held/releasing synth voices
retain their original instrument, envelope, base cutoff and captured mixer route.
Each re-used voice adopts the new identity when its next onset needs a different
instrument; this resets only that voice's envelope/filter. Samples continue their
one-shot tails across selection changes. Octave changes and original input-key
releases retain the established ownership behavior. Selection no longer destroys
and reallocates the synth pool in the audio command handler.

## Validation

Four new groups cover:

- Actual egui ComboBox selection of every displayed choice, including Samples;
  unsupported Drums is absent while all three sample banks remain present.
  The selected snapshot/engine identity and resulting voice ADSR are asserted.
- Real engine rendering at 44.1/48/96 kHz matches an independent reference built
  from explicit oscillator weights, literal envelope settings and the selected
  sample buffer. Pairwise sound fixtures are distinct, so Keys and Pad cannot
  silently alias.
- Held Analog, Keys and Pad voices coexist across menu/bank/selection/octave
  changes. Release targets each original gate; sample tails retain their source
  and mixer destination, and all voices finish after their appropriate release.
- Repeated instrument selection and synth-voice reuse allocate/free no heap
  storage in the audio handler.

Existing ownership, composition, clip-release, rate-reset and pad-routing tests
use explicit identities while retaining their original actual synth choices.
Physical hardware and listening QA remain pending.

Local validation: all 279 `cargo test` cases passed; `cargo build` and
`git diff --check` passed.
