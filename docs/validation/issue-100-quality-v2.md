# Issue 100: additive quality corpus version 2

Version 2 appends one original musical-onset source. All five version-1 PCM
sources, including the alternating Nyquist burst, are unchanged. Recorded PCM
has portable regression hashes; procedural floating-point/trigonometric PCM is
checked on the actual qualification host, avoiding a cross-libm golden claim.
Pass `run --prior-corpus /path/to/retained-v1-report` to verify all five prior
source descriptors/PCM hashes and bind that prior report into the new receipt.
The old 630-render reports remain immutable and valid as
version-1 evidence; they cannot be compared to a version-2 candidate. A fresh
old-algorithm baseline uses the same version-2 harness as the candidate.

The added six-second stereo train contains eleven onsets at half-second
intervals. Widths cycle through 3, 7, 15 and 31 ms, with a 0.5 ms attack and
quadratic decay. Each onset mixes a descending 220–55 Hz kick component, one of
750/1500/3000/6000 Hz tones, and seeded deterministic noise through two cascaded
6 kHz one-pole low-pass filters. The right channel is an exact copy delayed
72 source frames (1.5 ms at 48 kHz). Values are bounded by 0.55. This is original
procedural test audio, not a recorded drum performance or a listening score.
Its half-sample interpolation regression retains over 80% of each event's
energy, ensuring that this supplement is distinct from the Nyquist stress.

The unchanged rates, ratios, locked/unlocked variants and block sizes now give
756 isolated production-renderer cases, 126 actual callback cases and nine
transition cases. The blind comparison has 42 pairs at 48 kHz/128 frames, each
with an unlocked reference and empty human score fields. Attribution files are
hash-checked against the embedded source inventory before copying. The same
source/executable binding and fresh-directory rules apply.

The original alternating burst is intentionally severe: at 48 kHz it alternates
at Nyquist. Linear half-sample interpolation cancels most alternating energy and
can leave one leading spike. A zero discrete 2.5–97.5% energy width can therefore
mean concentrated one-sample energy, not silence or a missing metric. Those
measurements must remain visible; the added musical source does not excuse a
failure in the original source.

Both onset trains retain source-time displacement and energy diagnostics.
Version 2 also reports actual output-time left/right onset and energy-centroid
gaps. Source time is output time multiplied by playback ratio: a preserved
1.5 ms output-channel delay can become 0.75 or 2.25 ms in source coordinates at
ratios 0.5 or 1.5. Neither coordinate alone establishes stereo quality, device
latency or perceptual transparency. Compare the rendered audio and both
coordinates, with the corpus and resampling limitations disclosed.
