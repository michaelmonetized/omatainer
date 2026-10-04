# Musical-key analysis qualification draft

Issue #142 software path: Library → analyze… → Analyze musical key → Analyze selected row / filtered crate → Inspect selected cache. Comparison accepts conventional or Camelot notation. Correction opens Audio metadata and uses its captured-target review/save workflow.

The same-descriptor worker hashes/decodes native-rate PCM, analyzes the whole first stereo pair (or mono), and discards resident PCM before Ready. Cancellation, foreground preemption and performance protection share the existing token/publication boundary. No media, gain, pitch, grid or saved key tags are modified by analysis.

Saved user sidecars (including clears), observed embedded keys and metadata locks take precedence. The estimate stays in a separately versioned source-qualified record. Library schema 15 refuses injected key records in older headers, including null, and migrates actual older data without inventing analyzed values.

Algorithm 1: Hann windows around half a second, native-rate radix-2 FFT; channel power summation rather than phase-sensitive mono averaging; local spectral peaks with log-power interpolation; cosine pitch weighting; per-window normalized chroma; Krumhansl-Kessler profile correlation against 24 major/minor candidates. No Essentia code/library or corpus-trained profile is included.

Unknown thresholds: at least 16 tonal windows; correlation at least 0.70; best/runner-up margin at least 0.08. Correlation is not a calibrated probability. Material below 8 kHz, short material, silence, weak or ambiguous evidence remains unknown. Non-finite PCM and invalid/oversized input are refused; cancellation is checked during PCM validation and between transforms.

Independent development corpus: all 48 complete Kimiko Ishizaka Open Well-Tempered Clavier piano recordings; labels are the artist track titles and cover all 24 pitch-class/mode combinations. CC0 public recordings, no account/provider access. MP3 byte hashes, full per-recording outcomes and the native worker receipt are retained. This is a development set, not a held-out cross-genre/DJ accuracy claim.

Initial production result (debug-2; rerun on final qualified source required): 19 exact / 48 (39.58%), 27 unknown / 48 (56.25%), 2 wrong / 48 (4.17%). Returned estimates: 21, 19 correct / 21 (90.48%), 2 wrong / 21 (9.52%). The two errors are C-minor Prelude No. 2 classified as C major, and C-sharp-minor Prelude No. 4 classified as G-sharp minor. Both errors remain published.

The first production corpus run refused a fully trimmed MP3 terminal packet after all declared audible frames. The corrected decoder accepts only explicit zero-duration padding/delay packets with zero returned audio and leaves the audible timeline end unchanged. The retained generated regression compares identical PCM with and without the extra fully trimmed packet; declared-duration, packet-damage and checksum tests still apply.

Limits: one whole-source major/minor estimate; no modulating-key timeline, automatic tuning correction, loudness-weighted salience or channels beyond the first stereo pair. Comparison uses the effective source key, does not account for deck pitch changes and cannot promise a musical transition. No external controller, converter, physical listening or cross-genre accuracy proof is claimed.

Primary references:
- https://kimikoishizaka.bandcamp.com/album/bach-well-tempered-clavier-book-1
- https://welltemperedclavier.org/kimiko-ishizaka-releases-open-well-tempered-clavier.html
- https://extras.humdrum.org/man/keycor/
- https://essentia.upf.edu/reference/std_HPCP.html
- https://essentia.upf.edu/reference/std_Key.html
