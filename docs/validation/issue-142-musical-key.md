# Musical-key analysis (#142)

Issue #142 software path: Library → analyze… → Analyze musical key → Analyze selected row / filtered crate → Inspect selected cache. Comparison accepts conventional or Camelot notation. Correction opens Audio metadata and uses its captured-target review/save workflow.

The same-descriptor worker hashes/decodes native-rate PCM, analyzes the whole first stereo pair (or mono), and discards resident PCM before Ready. Cancellation, foreground preemption and performance protection share the existing token/publication boundary. No media, gain, pitch, grid or saved key tags are modified by analysis.

Saved user sidecars (including clears), observed embedded keys and metadata locks take precedence. The estimate stays in a separately versioned source-qualified record. Library schema 15 refuses injected key records in older headers, including null, and migrates actual older data without inventing analyzed values.

Algorithm 1: Hann windows around half a second, native-rate radix-2 FFT; channel power summation rather than phase-sensitive mono averaging; local spectral peaks with log-power interpolation; cosine pitch weighting; per-window normalized chroma; Krumhansl-Kessler profile correlation against 24 major/minor candidates. No Essentia code/library or corpus-trained profile is included.

Unknown thresholds: at least 16 tonal windows; correlation at least 0.70; best/runner-up margin at least 0.08. Correlation is not a calibrated probability. Material below 8 kHz, short material, silence, weak or ambiguous evidence remains unknown. Non-finite PCM and invalid/oversized input are refused; cancellation is checked during PCM validation and between transforms.

Independent development corpus: all 48 complete Kimiko Ishizaka Open Well-Tempered Clavier piano recordings; labels are the artist track titles and cover all 24 pitch-class/mode combinations. CC0 public recordings, no account/provider access. MP3 byte hashes, full per-recording outcomes and the native worker receipt are retained. This is a development set, not a held-out cross-genre/DJ accuracy claim.

The actual production worker on frozen source `cf8702ecc71c59a612521c8314a005b6682056b8` returns 19 exact / 48 (39.58%), 27 unknown / 48 (56.25%), 2 wrong / 48 (4.17%). Returned estimates: 21, 19 correct / 21 (90.48%), 2 wrong / 21 (9.52%). The two errors are C-minor Prelude No. 2 classified as C major, and C-sharp-minor Prelude No. 4 classified as G-sharp minor. Both errors remain published. This receipt uses the same-descriptor native-rate MP3 decoder and background analysis worker, not the earlier standalone resampled prototype.

The first production corpus run refused a fully trimmed MP3 terminal packet after all declared audible frames. The corrected decoder accepts only explicit zero-duration padding/delay packets with zero returned audio and leaves the audible timeline end unchanged. The retained generated regression compares identical PCM with and without the extra fully trimmed packet; declared-duration, packet-damage and checksum tests still apply.

Frozen source passes the complete ordinary suite: **1,621 passed, zero failed, 37 ignored**, with all 1,658 listed names audited. The run uses two test threads and the default test-stack limit. All 623 qualification input hashes remain unchanged. [The structured receipt](issue-142-musical-key.json) binds the source, three retained binaries, corpus outcomes, logs, failed attempts and package records.

The actual native review workflow analyzes a selected source, filters by its measured key, inspects the cache, enters compatible and incompatible Camelot values, opens the captured-target Audio metadata editor and applies a manual F-sharp-minor correction. Forced analysis preserves that correction; reopen retains the manual effective key and independent measured C-major record. The suite also checks notation parsing, opposed-phase stereo, scaling, silence/ambiguity, damaged input, cache versioning, older-header refusal, key-only analysis, metadata locks and incremental smart-crate invalidation.

Long WAV/FLAC/Ogg sources pass the actual App/decoder/background workflow while the four-channel production callback renders 553,088 frames in 4,321 blocks. Measured callback Rust allocations/frees are zero, output is finite, reference error is zero and the deck remains playing. Cancellation occurs before the first save; restarting preserves manual preparation and creative revision, and reopening uses the cache. Pacing sleeps outside the callback. This is functional continuity evidence, not deadline or physical-xrun qualification.

The unchanged optimized gate passes all eight workloads over three trials each on normal CPU affinity 0–9. Its actual App/renderer/AT-SPI preflight reports 280 visited nodes, 166 actions and 1,625 frames. This exercises native software accessibility paths without a physical window or screen-reader test.

Four fresh optimized private-server checks pass: JACK and PipeWire graph recovery/routing, plus settings Preview, Apply, Cancel and reopen for each backend. Graph checks report zero Rust callback allocations/frees during initial operation, quantum change and reconnect; settings checks do not measure heap activity, and C/C++ allocations are not measured. These dummy/private servers do not qualify physical interfaces or audibility.

The local ELF64 AArch64 package verifies all 34 packaged file records, frozen source revision and embedded licenses. It was neither published nor installed. Earlier gain and waveform qualification retain their own source bindings and measurements.

Failed attempts are retained: the first corpus run exposed terminal MP3 padding; focused tests exposed an absent formatted translation and an overconfident synthetic assertion. New native/smart-crate tests initially used unavailable APIs, the wrong AccessKit text property and an assertion that also counted factory rows. Those fixtures were corrected against actual behavior. The first long-analysis driver attempt failed offline metadata resolution before starting the child because CARGO_HOME was absent; the retry passed without source changes. No performance budget or test-stack override was introduced.

Limits: one whole-source major/minor estimate; no modulating-key timeline, automatic tuning correction, loudness-weighted salience or channels beyond the first stereo pair. Comparison uses the effective source key, does not account for deck pitch changes and cannot promise a musical transition. No external controller, converter, physical listening or cross-genre accuracy proof is claimed.

Primary references:
- https://kimikoishizaka.bandcamp.com/album/bach-well-tempered-clavier-book-1
- https://welltemperedclavier.org/kimiko-ishizaka-releases-open-well-tempered-clavier.html
- https://extras.humdrum.org/man/keycor/
- https://essentia.upf.edu/reference/std_HPCP.html
- https://essentia.upf.edu/reference/std_Key.html
