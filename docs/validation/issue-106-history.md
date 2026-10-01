# Performance session history: work in progress

This issue is not yet qualified or ready to close. This record tracks the
increment above final #105 (a06038b); no #106 PR has been published.

## Measurement foundation

The source observer uses fixed four-lane attribution of the existing deck
replacement/seek envelope. Process-local monotonic keys distinguish load
receipts from explicitly unresolved raw-media loads; these are not persisted
catalog identities. A bounded failed key registration returns unknown instead
of reusing an identity. A fifth overlapping source records incompleteness.

Four fallibly allocated stereo master-effect lanes retain old-source tails.
Their storage is checked against 96 MiB at the supported 8–384 kHz rates.
Ring-rotation peak envelopes bound future tails in constant time, with
floating-point margins and explicit error envelopes. Retirement below -110 dBFS
cannot change the actual audio, and pool overflow never relabels an old tail.
The error calculation qualifies an isolated linear master contribution removed
at the limiter input; it is not a claim that a differently rounded entire mix
would be bitwise equal to that counterfactual.

Classification uses 10 ms (rounded up to an output sample) RMS windows and a
-90 dBFS floor. Actual output and the source-dependent converted difference
must qualify on the same main channel. Cancellation nulls, silent integer
codes and zero-crossing samples cannot falsely create or truncate track time.
Partial windows retain actual frame counts. Numerical ambiguity, nonfinite
samples and sample-clock exhaustion are explicit outcomes.

The first compile exposed restricted re-export visibility and an untyped test
rate. A later compile exposed the same visibility boundary for the peak helper;
both were corrected. Twelve focused tests then passed in 0.96 seconds in
`issue-106-measurement-v6.log`. Earlier passed checkpoints were five groups
(v2), seven groups (v3) and nine groups (v5). Evidence includes independent
f64 fractional-delay/filter references, unchanged observed f32 processor output,
actual deck replacement/seek attribution, future wet/tempo changes, ring-tail
retirement/reuse, overflow and negotiated-rate storage bounds. The measured
tracker loop performed zero allocations and frees.

OutputCallback conversion/capture, session acknowledgment, persistence,
manual played/unplayed/external entries, export, UI, native checks and final
unchanged release/show budgets are still pending. These foundation tests alone
do not establish session history or physical audible/hardware behavior.
