# Decoder fixtures

These files contain generated tones, not user media. Each short source is
12,000 stereo PCM frames at 48 kHz: left is a 400 Hz sine at integer amplitude
12,000; right is an 800 Hz sine at amplitude 8,000. Samples are rounded to signed
16-bit PCM. The source is exactly 0.25 seconds. `tone-long.ogg` repeats it eight
times (2 seconds); `tone-next.ogg` is a separate encode with a distinct Ogg
stream serial for constructing a genuine chained stream.

Fixtures were encoded locally with FFmpeg n9.0.1. Commands, with the generated
PCM source at `source.wav`:

```sh
ffmpeg -i source.wav -map_metadata -1 -c:a flac tone.flac
ffmpeg -i source.wav -map_metadata -1 -c:a libvorbis -q:a 3 tone.ogg
ffmpeg -i source.wav -map_metadata -1 -c:a libvorbis -q:a 3 tone-next.ogg
ffmpeg -stream_loop 7 -i source.wav -map_metadata -1 -c:a libvorbis -q:a 3 tone-long.ogg
ffmpeg -i source.wav -map_metadata -1 -c:a libmp3lame -q:a 3 tone.mp3
ffmpeg -i source.wav -map_metadata -1 -c:a libmp3lame -q:a 3 -write_xing 0 tone-estimated.mp3
ffmpeg -i source.wav -map_metadata -1 -c:a aac -b:a 96k tone.m4a
```

Tests consume committed fixture bytes and require no FFmpeg executable. WAV
fixtures are generated in Rust so exact frame/channel values are asserted.
Corrupt/truncated variants are derived in isolated temporary files at runtime:

- FLAC MD5 mutation preserves all decoded frames but fails final verification.
- A reserved subframe type in FLAC frame 2, with its frame CRC recomputed,
  reaches the real decoder and fails after 4,096 valid frames. Its byte offsets
  are pinned to `tone.flac` (frame 2: 10,540 through 12,869).
- Vorbis audio/header payload damage in the final Ogg page has a recomputed
  page CRC. The demuxer skips this damage internally; the production duration
  check rejects the resulting shortened stream.
- Chaining the long and second Ogg streams triggers the real `ResetRequired`
  path after audio has already decoded.

Lossless/WAV frame counts and content are exact. Lossy fixtures retain codec
priming/padding where Symphonia does not trim it; tests assert the bounded
decoded lengths plus distinct left/right tone content. This change does not
claim sample-exact gapless support for every codec.

Tag inspection/edit fixtures `tone-tags.wav` and `tone-tags.aiff` decode the
original generated `tone.flac` losslessly, with metadata stripped. Generated
locally with FFmpeg; tests require no encoder executable:

```sh
ffmpeg -i tone.flac -map_metadata -1 -c:a pcm_s16le tone-tags.wav
ffmpeg -i tone.flac -map_metadata -1 -c:a pcm_s16be tone-tags.aiff
```

The tag tests copy fixtures into isolated temporary directories before writing
Unicode title/artist, BPM and key tags, and compare production-decoded PCM.

`tone-padding.mp3` is a generated 440 Hz mono sine, 44,100 Hz, 1,323 audible
frames. FFmpeg encodes 0.03 seconds with libmp3lame at 128 kbit/s. Its last
418-byte MPEG frame is repeated once, the Info frame count/byte count increase
by one frame/418 bytes, and the Lavc end-padding field increases by 1,152
samples. This retains the original audible samples and adds a fully trimmed
terminal packet. Tests assert identical PCM with and without that extra packet;
real declared-duration and damaged-packet checks remain active.
