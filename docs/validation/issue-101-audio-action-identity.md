# Audio cancellation identity during sampler qualification

An assembled helper run passed 820 tests but timed out while an existing real
calibration UI test waited for cancellation. Twenty-four unchanged isolated
repeats passed, so a passing rerun alone did not establish that the failure was
safe to ignore.

The audio panel placed asynchronous status text before automatic button IDs.
Running to Calibrating removes a callback-observation label. An accessibility
Cancel request exposed in the previous frame could consequently miss the new
button and leave the real calibration worker blocked.

A deterministic regression blocks the existing calibration backend fixture,
publishes the Running and Calibrating status transition between exposing and
consuming the actual egui action, and checks the real cancellation token and
worker completion. It failed on the previous implementation in 0.08 seconds.
The test controls status publication timing; it does not measure physical
loopback hardware or claim a timing cause from the original timeout alone.

Dynamic live status and notice actions now occupy fixed parent scopes. Each
audio action also has its own semantic salt, so Cancel cannot reuse a different
action identity. A salt alone is insufficient in the pinned egui version because
automatic child IDs also include the parent insertion counter.

All nine actual audio-settings UI groups pass in 0.49 seconds with four test
threads, including cancellation, rollback, saved settings, confirmation and
canonical help. Red and green logs are retained as
`issue-101-audio-cancel-red.log` and `issue-101-audio-cancel-green.log`.
The complete editor union and production gate are validated separately.
