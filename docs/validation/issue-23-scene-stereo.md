# Issue 23: preserve stereo at the scene FX input

The scene chain now receives the existing left/right session sum directly through `FxChain::process_stereo`. Adding a device no longer averages away track panning. Empty and all-bypassed chains preserve both channels exactly.

Spread is the intentional width control: zero selects a delayed mono sum; center preserves the original stereo input; values above center add Haas delay to the right channel. Its first slider is now labeled `width`, with those endpoints in its hover text. Other processors retain independent channel histories; compressor/gate may share their detector gain but do not crossfeed audio. Existing slot wet-law issues remain separately tracked by #54/#60.

Three regressions exercise full-engine processing with independently panned synth tones. Adding and bypassing all 13 processors produces sample-identical output to the empty chain. Each enabled processor at its default parameters leaves an empty right channel at exact zero when fed only a left-panned tone. Explicit zero width produces equal channels, while center width matches the original stereo output exactly.

Local `cargo test --locked`: 78 tests pass on the issue-13 stack. No physical-device or listening claim is made.
