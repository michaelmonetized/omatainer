# Human drum beatgrid fixture

`11_rock_100_beat_4-4.wav` and its MIDI annotation are unchanged members of Google LLC’s **Groove MIDI Dataset 1.0.0**, performance `drummer3/session2/11`, test split, rock, 100 BPM. A person played a Roland TD-11 electronic drum kit; the WAV is its recorded synthesized output. This is a human electronic performance, not an acoustic drum recording or generated timing.

Source and license: https://magenta.tensorflow.org/datasets/groove. **CC BY 4.0**, https://creativecommons.org/licenses/by/4.0/. The retained license text applies; no endorsement is implied. The project MIT license does not replace these terms. Attribution: Google LLC; Jon Gillick, Adam Roberts, Jesse Engel, Douglas Eck and David Bamman, *Learning to Groove with Inverse Sequence Transformations*, ICML 2019.

`sources.json` pins complete member bytes and acquisition checks. The complete 4.76 GB archive was not downloaded or independently hashed. The fixture is source-tree test material, not a bundled production sound.

Manual anchors at beat 4 (2.3925 s), 8 (4.7925 s), 12 (7.20125 s) and 15 (8.99875 s) use the annotated quarter-note ride hits near those beats. Beat 0 is 0 s; the map’s tail retains its last interval. Other human note timing is retained. The renderer test exercises synced playback, source seeks, cross-anchor loops and source-rate changes, preserving the original decoded PCM. Rendered test output is a processed derivative and retains this attribution through this document.


## Consolidated additional performance

# Human drum performance for manual tempo maps

Unmodified WAV and MIDI from **Groove MIDI Dataset 1.0.0**, Google LLC, by Jon Gillick, Adam Roberts, Jesse Engel, Douglas Eck and David Bamman, *Learning to Groove with Inverse Sequence Transformations* (ICML 2019). Source: https://magenta.tensorflow.org/datasets/groove.

Licensed under **CC BY 4.0**: https://creativecommons.org/licenses/by/4.0/. The complete license and disclaimer are in `CC-BY-4.0.txt`. The project MIT license does not replace these terms. No endorsement is implied.

This is drummer3, session2, performance35: rock, 92 BPM, 4/4, test split. A human played a Roland TD-11 electronic kit; its recorded audio is synthesized by the kit. It is not an acoustic drum recording or a generated timing approximation.

Both archive members are unchanged. `sources.json` records exact hashes and ZIP CRC checks, plus quarter-beat annotations selected from nearby kick/snare MIDI onsets. Annotations map the performed timing and retain its local variation; they do not claim automatic beat detection. The WAV remains unchanged while test playback uses the manual map.

Only the required members and archive directory were retrieved with HTTP ranges. The members passed ZIP CRC checks; the full 4.76 GB archive checksum was not measured. These recordings are source-test material, not factory assets or production embedded audio. Rendered derivatives must retain attribution and disclose processing.

The additional performance uses [its original provenance receipt](additional-sources.json). The original source-time/BPM implementation is retained in Git history; combined tests use the canonical beat/time preparation format.
