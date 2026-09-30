# Issue 19: clip-local recording positions

Live MIDI and pad capture now share `recording_position`. For the selected playing clip, it subtracts that clip's launch beat and wraps by its actual length (`bars * 4`, minimum one beat). Second and later bars retain their positions, and a nonzero or fractional launch origin is respected.

A matching quantized pending clip is monitor-only until its scheduled start: early input still sounds and releases normally but does not write an event into the end of the clip. At the launch beat, capture begins at zero. A stopped or unlaunched target has an explicit compose cursor at local zero, including when a different scene is playing on the track. Capture does not implicitly launch that target. Pads still compose while transport is stopped, preserving contract C5; MIDI recording retains its running-transport requirement.

Three regressions cover live/pad capture in one-, two- and four-bar clips at a fractional nonzero launch beat, multiple wraps, velocity preservation, pending monitoring, start-boundary input, stopped targets and another scene playing. A replay fixture verifies that captured late-bar notes trigger within one output sample of their original local position. Local `cargo test --locked`: 67 tests passed on the issue-16 prerequisite branch.

Note durations are a separate issue (#20). These are synthetic engine checks without physical controllers.
