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

## Output callback prototype

Only the actual OutputCallback's final converted destination buffer promotes
observations. A fixed 16,384-frame sidecar captures corresponding isolated
source-removal values; larger blocks explicitly lose measurement coverage while
audio continues. Source observers still advance before session Start, retaining
real pre-existing tails. Direct renderer export and zero-frame offline project
service cannot create output time. Rate changes close partial rational-rate
segments without replacing the observation receiver, and actual DSP/project
resets clear matching observer histories. The callback wall timer includes
observation; the existing renderer-CPU scope still excludes final conversion.

Sixteen focused groups passed in 11.12 seconds, with one opt-in timing probe
ignored (`issue-106-measurement-v9.log`). This includes 250 combinations across
all ten supported sample formats, mono/stereo/4/6/8 channels and signal, zero
gain, crossfader-zero PFL at cue mix 0/1 and anti-phase mono cancellation.
Independent assertions use actual converted output buffers, including zero
additional channels. Warmed measured callbacks had no allocations or frees.
Other cases cover starting while playing, ending partial windows, 48/44.1 kHz
segments, sidecar overflow and disconnected observation consumers.

An opt-in release-profile active-history probe passed 2,048 measured blocks in
each of three existing two-keylock show fixtures. No policy ceiling changed:

| Rate / frames / ratio | Wall p99 / max, ms | Full callback CPU p99, ms |
| --- | --- | --- |
| 44,100 / 128 / 0.50 | 0.733339 / 0.829465 | 0.726416 |
| 48,000 / 128 / 0.84 | 0.766297 / 0.838215 | 0.763167 |
| 96,000 / 128 / 1.50 | 0.756423 / 0.843007 | 0.751834 |

All original workload audio/state assertions passed, with zero measured heap
operations and command rejections. The observation receiver drains between
timed callbacks. This one-repeat, three-case prototype is not the final full
source-bound #95/#100 gate. Raw samples and log are preserved as
`issue-106-history-prototype-timing-v1.*` in the local work evidence directory.

Session acknowledgment, timestamps, persistence, manual played/unplayed/external
entries, export, UI, native checks and final release/show qualification remain
pending. Start/End currently run only as direct renderer-owned test operations;
there is no product session control yet. These software captures are not
physical audible-duration, driver delivery, hardware, listening or XRUN proof.
