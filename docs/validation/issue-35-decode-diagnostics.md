# Issue 35: report damaged and incomplete decoding

Audio decoding returns `DecodedAudio { sample, diagnostics }` or a structured
`DecodeFailure`; a failure never carries a loadable prefix. Failures retain
their operation stage, reason, decoded/expected frame counts, packet count,
EOF state and any checksum result. The GUI worker preserves these types until
completion. A failed result updates the visible load status without sending a
deck replacement command. The main UI now renders that status in a bottom panel
in every mode; previously the status string was assigned but never painted.

The policy rejects detected packet damage (even codec errors that could be
skipped), fatal reader/codec failures, missing packet timestamps, short declared
duration, failed checksum verification, empty media, non-finite PCM, and changes
in sample rate/channel layout. Actual decoded format determines sample rate and
channels. Codec verification and available gapless trimming are enabled.

`UnexpectedEof` from the reader is only a candidate completion: declared
duration, decoded frame count and packet coverage are checked before success.
Container durations are converted through their timebase. Reset-required or
chained streams are explicitly unsupported and rejected in their entirety;
silently loading the pre-reset section is no longer possible. Symphonia's
[decoder contract](https://docs.rs/symphonia-core/0.5.5/symphonia_core/codecs/trait.Decoder.html)
distinguishes recoverable packet errors from resets and fatal errors; its
[codec parameters](https://docs.rs/symphonia-core/0.5.5/symphonia_core/codecs/struct.CodecParameters.html)
describe duration in the stream's timebase.

Symphonia 0.5's MPEG reader may estimate `n_frames` from bitrate and does not
expose the evidence source. MPEG durations are therefore classified as estimated
or unavailable, not enforced as exact. Successful media lacking a reliable
declared duration or verified checksum shows `length unverified; incomplete
media cannot be ruled out` in the GUI load status. This also explicitly labels
undetectable truncation in headerless/unknown-length streams. Arbitrary PCM bit
changes without a checksum cannot be detected; no universal corruption proof is
claimed. Codec priming/padding behavior remains bounded by the decoder, not a
claim of universal sample-exact gapless playback.

`decode_audio_with_cancel` supplies cooperative checks around open/probe, packet
reads, packet decode and waveform/BPM analysis boundaries. Cancellation returns
a distinct failure and discards the prefix. It does not interrupt blocking file
I/O. Issue 37 owns scheduling, request identities, overload and stale-result
handling; this is only its compatible decode hook.

## Validation

- `cargo test --offline`: 162 tests pass on issue 25 plus the issue 34 prerequisite.
- `cargo build --offline`: passes locally.
- Ten decoder tests use real WAV, FLAC, Ogg/Vorbis, MPEG and AAC/M4A bytes. They
  assert exact PCM frame counts/content; compressed stereo tone content and
  bounded duration; truncated WAV/FLAC rejection; recoverable FLAC subframe
  errors after 4,096 decoded frames; FLAC checksum mismatch; internally skipped
  Vorbis damage; genuine chained Ogg reset after a decoded prefix; unknown-length
  truncation error/warning policy; missing/malformed/empty files; and cancellation
  at every cooperative boundary.
- Two actual `App::poll_loads` regressions show corrupt loads preserve deck
  audio/emit no replacement, and unverified-length warnings reach the status
  while valid media queues normally. Egui output assertions verify both failure
  and warning text are painted inside the visible status panel. Existing
  built-in/file loading tests pass with typed results.
- `git diff --check`: passes.

All source media is synthetic and documented in `tests/fixtures/audio/README.md`.
Tests require no external encoder, installed application, audio/MIDI devices or
user media. Hardware playback and interactive desktop QA are not claimed here.
