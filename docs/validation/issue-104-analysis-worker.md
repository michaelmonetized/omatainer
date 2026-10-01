# Issue 104 analysis worker foundation

This delta supplies the source analysis lane. Catalog/cache persistence, batch selection and GUI publication are separate parts of issue 104; a worker `Ready` state is not a saved result.

## Ownership and limits

Deck A, deck B and sampler preparation retain their existing round-robin lanes on one decoder thread. One replaceable analysis request/result is selected only when those foreground queues are empty. A successfully admitted explicit deck or sampler request preempts pending/active analysis. Analysis never cancels foreground work or creates another decoder. Request IDs are allocated in serialized admission order. Replaced results and all analysis PCM are destroyed off the callback.

Analysis is optional work. Its `WorkPermit` and `AnalysisToken` accompany the small result through the metadata writer. Immediately before irreversible publication, the writer must obtain `work.commit()` and then `token.claim_publication()`, retaining the permit through commit. Cancellation/replacement before the atomic token claim wins; after the claim, the writer must report the real commit/durability outcome even if a newer request changes current generation. Taking a decoder result is not a persistence acknowledgment.

Only captured local `SourceRef` identities are admitted. The worker opens one regular, non-symlink final file, checks the captured fingerprint, hashes at 64 KiB boundaries, and decodes the same open description. It checks both the retained descriptor and pathname fingerprints after hashing, decoding and optional analysis. An existing expected hash must match. The source-file ceiling is 8 GiB; strict decoded PCM capacity remains 1 GiB. Declared oversized geometry is rejected before growing the PCM buffer. The result contains a freshly measured source hash, actual frame-count duration, requested optional tempo and at most 2048 three-band bins. PCM is not returned to the GUI or metadata writer.

Progress is a coherent scalar stage plus a fraction only when a real denominator exists. Hash progress counts source bytes; decode progress counts frames against available declared geometry. Tempo/waveform phases are indeterminate. Cancellation checks occur between reads/packets, tempo envelope/scoring steps, and at most 4096 waveform source frames apart. No claim is made that an already blocked OS read or codec operation can be interrupted immediately.

## Analysis semantics

Persistent tempo uses the existing 70–180 BPM heuristic, with no invented confidence score. Empty, short or no-score input produces completed `Unknown`, not a measured 120 BPM fallback. Existing deck callers retain their prior fallback behavior. Key detection is not provided by this worker.

The waveform uses the existing bounded low/mid/high mean-magnitude calculation over the full source duration and first mono/stereo pair. It is not raw waveform min/max, does not represent additional multichannel channels, and can lose opposed stereo content through mono averaging. Source channel count and rate remain accurate geometry. Selective duration-only work skips tempo and waveform calculation.

## Validation

Focused source tests cover real WAV/FLAC/Ogg decoding, source hash and native geometry, selective fields, silent/short/no-score Unknown, legacy fallback, source replacement during hash/decode, real packet-boundary cancellation and waveform equivalence. Loader tests cover both foreground targets, same decoder thread, foreground priority, concurrent bounded replacement, cancellation after result retrieval, quick protection cycles and nonblocking teardown.

The playback regression analyzes an eight-second stereo source while exercising the actual `OutputCallback`. Its 256 warmed render blocks remain bit-identical to an independent renderer, with zero callback allocations/frees. This is an output/state/heap check, not a timing budget, device XRUN, physical deadline or listening claim.

Focused checks: six analysis groups and seven new loader groups passed. The existing seven loader groups also passed. Full-suite/build results will be recorded after the coordinated release timing window.
