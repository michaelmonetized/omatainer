# Audio preview confirmation identity follow-up

Review after the asynchronous Cancel identity fix found a different consent
boundary. Previewing again left the prior Switch/Calibrate confirmation alive.
Even with unchanged saved settings, default-device discovery can resolve to a
different output or input route. Clearing that confirmation alone also allows an
old native action identity to be recycled when a new confirmation is displayed.

Each successfully admitted preview now advances a checked, process-local
preview generation and immediately retires the prior preview/confirmation.
Publication clears confirmation again. Preview actions and route confirmations
include that generation in their semantic identity; stale or absent-preview
confirmations are rejected. Failed/cancelled discovery cannot revive earlier
consent. Saved intent, actual backend validation, exclusive audio admission,
rollback, callback ownership, and the stopped DSP reset contract are unchanged.

The real egui regression uses the controlled existing audio owner/backend. It
captures a native confirmation action, changes default-device discovery without
editing saved preferences, previews again, opens a new confirmation, and sends
the old action. No output open or calibration is admitted. A freshly exposed
action then successfully applies the new output or measures the new input in
the controlled backend. This covers both Switch and Calibrate.

The previous production source fails deterministically in 0.09 seconds:
`refresh retained consent to the old route`. The fixed source passes all ten
audio-settings UI groups in 0.71 seconds with two test threads, including the
prior asynchronous Cancel, rollback and cancellation regressions. Logs:
`/tmp/issue101-audio-preview-red.log` and
`/tmp/issue101-audio-preview-green.log`. The ten owner-focused groups also pass in 1.10 seconds, recorded in
`/tmp/issue101-audio-preview-owner.log`.

This is application/backend-fixture evidence, not a physical loopback or device
switching claim. Final ordinary suite, production build and source-bound release
gate run on the assembled parent stack.
