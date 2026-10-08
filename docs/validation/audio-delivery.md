# Audio export and performance recording

Issues #179 and #181 share one native **Audio export & recording** window.
Offline export renders the actual mixer/effects graph from a coherent project
snapshot. Performance recording captures either a selected final physical-output
alias or a raw routed recording alias. Both use a bounded worker and native
float32, PCM16 or PCM24 WAV writing; FLAC16, FLAC24 and 320 kbps MP3 use FFmpeg.

Export selects a scene or currently launched session clips, saved deck cursors,
program output, range, repeats, source-release tail, sample rate, mono/stereo,
integer dither and optional normalization to −1 dBFS. Fresh DSP is reconstructed
by preroll. Every repeat copies the same rendered range. A new folder is
published only after encoding and durable metadata succeed. Physical input
routes require real-time recording. This is not an Arrangement/history replay;
those workflows remain separate checklist items. Enabled unavailable effects
and sounding unavailable instruments refuse export rather than losing audio.

Recording preserves the selected channel order and actual stream rate. Final
output includes the limiter, recovery fades and device sample conversion; a raw
alias preserves its selected tap. Final-output recording leaves armed Auto
monitoring unchanged. The native source meter, duration and destination are
available during Performance Mode. Native files split at 256 MiB. FLAC/MP3
encoding follows capture and retains the original finalized WAVs. Existing
folders are never replaced. Preflight checks free disk space; queue overflow,
missing channels, invalid samples, source/rate changes, stopped callbacks and
actual write errors produce explicit errors or warnings with retained prefixes.

Recovery first reviews a recording's manifest, exact file fingerprints, regular
file identity, native WAV format and complete finite frames. An explicit native
Recover action revalidates every reviewed segment under file locks before
repairing headers. A folder ownership lock spans capture, segment changes and
encoding, so closed earlier segments cannot be recovered during an active take. Active files, linked segments, foreign headers and stale
reviews are refused. Cancellation precedes repairs; after validation, repairs
and durable manifest completion finish as one worker operation. This proves
process-interruption recovery, not survival of power loss or failed storage.

Software checks exercise real encoders/decoders, actual callback sample
conversion, routed channel order, native effects, source/review failures,
protected-mode UI, bounded queues, suspended producers and zero callback heap
work. The two-hour controlled stream is accelerated software delivery at 12 kHz:
86,400,000 mono float frames, 345,600,000 PCM bytes, split into two WAV files and
verified in order against the submitted stream's SHA-256. It is not a two-hour
hardware endurance claim. An isolated writer is killed; its recovered 4,096
stereo frames decode exactly. An isolated OS file-size limit produces a real
write failure; only its 512 complete stereo frames survive and decode exactly.

The source-bound receipt records the focused results, complete regression gate,
original export-fixture failure and package/install provenance. The complete
unfiltered regression gate passes: 1,827 passed, 44 explicitly ignored cases, no
failures or filters. A forced fresh release of source
`278025cc1a3d06375ef66b975bdf1c65d72bf259` is installed as
`0.1.0+278025cc1a3d`; executable SHA-256
`c161c7dab9467eacf3bd21918bab5c07937ee9be7b79192333be4dc5efb5d1b4`.
The independent private controller app and old CLI follower retained their exact
processes and executable hashes. Playback stayed paused with master 1 before and
after installation. The default is for the next normal launch. No physical audio
or device capture is opened for this qualification.
