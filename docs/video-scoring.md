# Native picture scoring

The audio owner integrates integer playing frames, preserving seconds across
pause, tempo changes and output-rate changes. One atomic scalar is refreshed per
callback; the video view does not wait for the full display snapshot. Seeking
converts seconds into the current conductor's beat, releases clip notes, ends
clip recording and rebuilds active schedules. Physical input ownership remains.
Project state schema 10 saves this clock. Older states derive their initial
position from saved beat and tempo/conductor; legacy headers reject new fields.

One external Clip stores exact rate, decoded frame count, file fingerprint,
source trim, project placement, signed timecode offset, drop-frame mode,
preview latency adjustment and up to 128 named project-frame locators. Native
Save/New/Open restores these values and verifies current source identity before
preview. Missing or changed sources remain explicit; reimport adopts new bytes.
The dependency inspector names the external picture. Portable export currently
refuses picture references; remove the reference in a saved copy first.

Installed FFmpeg/ffprobe run with argument-based child processes, one decoder
thread and allowlisted local file/container protocols. Import checks all decoded
frame timestamps, rate, dimensions, SDR transfer, square pixels and rotation.
Matroska's slight reported-rate rounding is normalized only to supported rates;
the full timestamp check still rejects discontinuities and variable-rate input.
Preview seeks use absolute timestamps, accurate decoding and container-timestamp
rounding tolerance. A bounded three-frame channel and bounded presentation queue
hold decoded RGBA; the UI presents only the exact due source frame or waiting/
black. Decode, pipe reads, file checks, cancellation and reaping stay on workers.
Closing the viewer cancels decoding; closing the editor retains admitted import
or export work. Native Cancel explicitly stops pending publication.

Supported rates: 24, 25, 30, 50, 60, 24000/1001, 30000/1001, 60000/1001.
Supported codecs: H.264, HEVC, ProRes, VP8, VP9, FFV1, MPEG-4, within the declared
SDR pixel/container limits. Sources are bounded to one million decoded frames,
4096×2160 and 128 MiB probe output, with a 120-second probe deadline. Individual
preview frame reads have a 15-second deadline. Preview is at most 1280×720.
Import failures preserve the source and current saved reference. Tagged PQ/HLG,
variable rate, rotation and nonsquare pixels require normalization first.

Offline rendering captures the real project coherently and owns a separately
prepared native renderer. It loops the selected Session scene from project zero,
with fresh DSP state, native mixer/effects, no count-in or metronome and no source
video audio. Rational frame/sample boundaries bound the 48 kHz stereo float WAV.
FFmpeg trims original video by source-frame index, resets timestamps and adds
black placement frames, then muxes lossless FFV1 picture with the score PCM.
A completed decode verifies output frame count and rate before publication.
Preview latency changes presentation only; score timing remains digital and
sample-aligned. Physical output latency requires user calibration.

`score.wav`, `picture.mkv` and `alignment.json` publish together through the
existing private staging-directory owner and Linux no-replace rename. The
performance commit guard qualifies publication. Cancellation before publication
removes staging; committed directory-sync failure remains a committed warning.
Existing destinations, source picture and live project remain intact. WAV is
bounded to 2 GiB, picture output to 12 GiB, and muxing to ten minutes; import's
normal timestamp/count checks also verify rendered picture. Unsupported codecs
or missing installed tools report actual child failures. FFmpeg is not bundled
and no external music or picture is downloaded.

Primary references: [ffprobe frame/stream inspection](https://ffmpeg.org/ffprobe.html),
[FFmpeg accurate seeking and absolute seek timestamps](https://ffmpeg.org/ffmpeg.html),
[trim, setpts and tpad filters](https://ffmpeg.org/ffmpeg-filters.html).

The validation record names tested codecs/rates and generated fixtures. Declared
support is broader than the codec combinations actually qualified on this host.
Detached OS-window behavior and physical audio/display synchronization require
separate desktop/device QA; headless native controls and digital samples do not
prove those external paths. This is Session scoring, alongside frame placement;
an Arrange timeline or persistent parameter automation lane remains separate.
