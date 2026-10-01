# Issue 82 — native project file codec layer

This document covers the codec prerequisite. Project state capture, worker
orchestration, dirty-state rules and the New/Open/Recent/Save UI belong to the
integrating issue 82 change; this codec alone does not claim that workflow.

## Worker API and ownership

`project_file::Bundle<T>` contains `state: T` and `media: Vec<Arc<Sample>>`.
The engine model references media by ordinal and owns validation of those
references and its strict generic `T` fields (including finite/ranged controls).

- `save(path, &bundle, Overwrite::{Never, Replace}, &limits, &cancel)` returns
  `Result<SaveOutcome, Error>`. Every `Err` is precommit. `SaveOutcome::Durable`
  means file and directory sync succeeded;
  `CommittedButDirectorySyncFailed(String)` means the complete new destination
  already exists, but directory durability could not be confirmed. The UI must
  report saved-with-warning instead of a failed save or automatic retry.
- `load::<T>(path, &limits, &cancel)` returns a complete `Bundle<T>` or an error.
  It performs no writes, replacement, recovery or migration, even when the
  existing file is corrupt or has a newer format version.
- `Error::{Cancelled, Invalid(String), Io { stage, source }}` identifies
  cancellation, validation, and the exact failing file-operation stage.

All codec allocations, metadata serialization, PCM validation/checksums, file
reads/writes and resulting media destruction belong on an I/O worker. These
functions never run on the GUI or audio callback. Cancellation is checked
between bounded 16 KiB chunks and before publication. An OS call already in
progress may finish before cancellation is observed.

## Version 1 format

| Offset | Encoding | Meaning |
| --- | --- | --- |
| 0 | 8 bytes `OMATPRJ\0` | Magic |
| 8 | little-endian u32 | Format version, currently 1 |
| 12 | little-endian u64 | JSON metadata byte length |
| 20 | little-endian u64 | Total PCM byte length |
| 28 | declared UTF-8 JSON bytes | Strict envelope: format_version, state, media |
| after JSON | concatenated little-endian f32 bytes | Each media entry's declared interleaved values, in order |
| final 4 bytes | little-endian CRC32 | IEEE checksum of every preceding byte |

Each strict media record stores its name, original path, sample rate, channels,
value count, BPM float bits and waveform peak float bits. PCM, BPM and peaks
preserve all finite f32 representations, including signed zero and subnormals;
no source decoding or analysis is repeated. Referenced source paths need not
exist when reopening embedded media. Generic state remains JSON and uses
serde_json's `float_roundtrip` feature: a regression with 20,000 deterministic
finite f64 patterns found a one-ULP position change without that feature and
now requires exact bits, including signed zero. The engine model must reject
nonfinite generic-state controls before serialization; serde_json otherwise
represents those as null. The codec independently rejects NaN/Inf in every
media PCM/BPM/peak field on both save and load.

Default desktop limits are 8 MiB of JSON metadata (including peaks), 256 media
entries, and 1 GiB of PCM. `Limits` permits caller-specified bounds. Save
preflights media counts/total PCM/peak storage and uses a JSON writer whose
buffer never grows above the configured cap. Load checks header lengths and
exact total file size before allocating metadata, then validates every media
shape and total PCM declaration before any PCM allocation. Allocation sizes
use checked arithmetic and fallible reservation. Valid sample rates are
1–768,000 Hz and channels 1–32; every payload contains complete channel frames.
JSON unknown/duplicate fields, malformed data, version disagreement, size
overflow, CRC corruption, truncation and trailing bytes are rejected.

## Publication and failure boundary

A 0600 same-directory temporary file is created exclusively, written, and
synced. **Never** publishes with an atomic hard link, so a destination created
by another writer during saving cannot be overwritten. **Replace** atomically
renames the temporary over the explicitly authorized destination. Existing
symlink/directory destinations are rejected; load also refuses symlinks and
nonregular inputs using no-follow/nonblocking open plus file-type validation.

Cancellation/failure before publication removes the owned temporary and
preserves the previous file. Publication wins cancellation arriving afterward.
The parent directory is synced after the commit; failure then returns the
committed warning outcome. Normal temporary cleanup is tested; a process killed
before cleanup can leave a private `.omatainer-project-*.tmp` orphan, which is
never treated as the user's project. A later save uses a fresh exclusive name.
The CRC detects accidental byte corruption and is not an authenticity signature.

## Private fixture evidence

- Exact state/media roundtrip, including UTF-8 names/paths, absent originals,
  +0/−0, subnormals, maximum finite PCM, exact BPM and peak bits, stereo and empty
  mono assets. The on-disk first samples are independently checked as LE bytes;
  CRC32 is checked against the standard `123456789` vector.
- Preflight limits and malformed sample rate/channels/channel frame/NaN/Inf
  retain an existing real native file and leave no temporary on normal failure.
- Independently changed magic/version/lengths, overflow, metadata fields, PCM,
  CRC, nonfinite values, truncated bytes and appended bytes are rejected. Load
  leaves every damaged/newer fixture byte-for-byte unchanged.
- Fault injection at metadata preparation, temporary creation, actual payload
  writes and precommit, plus cancellation at those same boundaries, preserves
  the previous file. A directory-sync fault reports committed and reopens the
  new complete state. A cancellation after publication likewise reports success.
- A competing actual file created just before **Never** publication is retained.
  Real symlink, directory and FIFO fixtures fail without following/mutating or
  waiting for their contents.
- A fresh test process reopens the actual file and checks complete state/media.
  Another real child is killed after its first temporary PCM write; the previous
  good file remains identical and reopens in a fresh process. A subsequent save
  and fresh-process reopen also succeed. These checks do not simulate a machine
  power failure or filesystem hardware fault.

All fixtures use private temporary paths on local Linux aarch64. No installed
application, user project or audio/MIDI hardware is changed by codec tests.

Validation: the full local suite passed **419 tests**, with four explicitly
ignored harness/benchmark entries; the fresh-process harness is exercised by
its parent regression above. `cargo build` and `git diff --check` also pass.
