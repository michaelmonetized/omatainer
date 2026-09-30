# Issue 24: independent scene effects

`scene_fx` is now an array of eight independent chains. FX commands and published panel contents address the explicitly selected scene. Editing, bypassing or opening one scene never mutates another.

A fixed stereo input array routes every track to its last started clip scene, beginning with scene 1 in a new session. Pending launches retain the prior route until their actual first sample; stops keep that route for release/live/track-effect output. Added scenes may coexist on different tracks. A newly started clip moves the complete track output, including any residual track-level tail, to its new bus. Old scene-effect histories remain in their own chains and continue ticking with zero input if no tracks remain. Their own feedback governs decay; bypass continues the existing freeze-history policy. Scene outputs sum before decks and master processing.

Four new regressions verify all eight panel settings/snapshots independently, two simultaneous added scenes using opposite panned tones, route changes only on actual launch (not pending/panel selection/stop), and exact old-scene delay-tail samples after a track changes scenes. The new scene retains a silent independent history. Existing scene-stereo and allocation regressions pass after adapting to the array.

Local `cargo test --locked`: 112 passed on the issue-18 stack plus issue-23 prerequisite. These are synthetic routing/DSP checks without physical-device or listening evidence.
