# Omatainer AIFF bounds patch

This is the pinned `symphonia-format-riff` 0.5.5 source, with its MPL-2.0
license and original file notices retained. `UPSTREAM.json` records the
upstream archive digest and unmodified source hashes.

Only `src/aiff/chunks.rs` and `src/aiff/mod.rs` change behavior: subtract the
SSND offset/block-size header from the audio-data extent, reject a shorter
chunk, and require the resulting frame count to agree with COMM. Upstream
0.5.5 counts the eight header bytes as audio: an ordinary 12,000-frame stereo
16-bit AIFF advertises 12,002 frames, and a following tag can enter the audio
packet range. Source media is never patched or rewritten by the demuxer.

SSND must also contain a whole number of audio frames. Its length excludes the
optional outer even-byte chunk pad, so valid odd-length 8-bit mono audio remains
supported while an incomplete interleaved sample frame is rejected.

Wave handling and codecs are unchanged. Offset/block-aligned AIFF remains
explicitly unsupported, as in the pinned upstream release. The integration
regressions use actual FFmpeg-generated AIFF, Unicode tags after SSND, exact
production-decoded PCM, and malformed chunk/frame-count refusal.

Modified source is retained here and published as part of Omatainer. The
release provenance inventory must bind all of these files and distinguish this
modified component from the upstream archive before packaging.
