//! Read-only preservation proof for a staged tag rewrite. The tag library owns
//! serialization; this independent inventory refuses loss of unrelated data,
//! including metadata the library may have discarded during parsing.
use super::{active, FileFingerprint, FileType, MAX_SOURCE, TAG_READ_BYTES};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    sync::atomic::AtomicBool,
};

const MAX_RECORDS: usize = 4096;

#[derive(Clone, Copy, Default)]
pub(crate) struct ChangedFields {
    pub title: bool,
    pub artist: bool,
    pub bpm: bool,
    pub key: bool,
}
impl ChangedFields {
    fn id3(self, id: &[u8]) -> bool {
        matches!(id, b"TIT2") && self.title
            || matches!(id, b"TPE1") && self.artist
            || matches!(id, b"TBPM") && self.bpm
            || matches!(id, b"TKEY") && self.key
    }
    fn vorbis(self, key: &[u8]) -> bool {
        key.eq_ignore_ascii_case(b"TITLE") && self.title
            || key.eq_ignore_ascii_case(b"ARTIST") && self.artist
            || key.eq_ignore_ascii_case(b"BPM") && self.bpm
            || (key.eq_ignore_ascii_case(b"INITIALKEY") || key.eq_ignore_ascii_case(b"KEY"))
                && self.key
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Record(Vec<u8>, u64, [u8; 32]);
#[derive(Default, Debug, PartialEq, Eq)]
struct Inventory {
    // Container order matters. The editable tag and padding may move; all other
    // chunks/blocks and the complete MPEG/FLAC audio tail must remain exact.
    outer: Vec<Record>,
    // Frame/comment order does not carry meaning, but duplicate multiplicity
    // does. Sorting keeps duplicate artwork and private frames observable.
    fields: Vec<Record>,
    // FLAC permits picture blocks anywhere after STREAMINFO. Keep every byte
    // and their relative order while allowing the tag writer to move the block
    // group past the edited Vorbis-comment block and other metadata.
    pictures: Vec<Record>,
    id3_version: Option<u8>,
    vendor: Option<Vec<u8>>,
}
struct Input<'a> {
    file: &'a mut File,
    cancel: &'a AtomicBool,
    length: u64,
    fingerprint: FileFingerprint,
    metadata_bytes: u64,
}
impl<'a> Input<'a> {
    fn new(file: &'a mut File, cancel: &'a AtomicBool) -> Result<Self, String> {
        active(cancel)?;
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_SOURCE {
            return Err("preservation requires a regular file within 8 GiB".into());
        }
        Ok(Self {
            file,
            cancel,
            length: metadata.len(),
            fingerprint: FileFingerprint::from_metadata(&metadata),
            metadata_bytes: 0,
        })
    }
    fn bounds(&self, offset: u64, length: u64) -> Result<(), String> {
        active(self.cancel)?;
        if offset
            .checked_add(length)
            .is_none_or(|end| end > self.length)
        {
            return Err("metadata extent is outside the captured file".into());
        }
        Ok(())
    }
    fn bytes(&mut self, offset: u64, length: u64) -> Result<Vec<u8>, String> {
        self.bounds(offset, length)?;
        self.metadata_bytes = self
            .metadata_bytes
            .checked_add(length)
            .ok_or("metadata length overflow")?;
        if self.metadata_bytes > TAG_READ_BYTES {
            return Err("preservation metadata exceeds 16 MiB".into());
        }
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut bytes = vec![0; usize::try_from(length).map_err(|_| "metadata length overflow")?];
        for block in bytes.chunks_mut(65536) {
            active(self.cancel)?;
            self.file.read_exact(block).map_err(|e| e.to_string())?;
        }
        Ok(bytes)
    }
    fn record(&mut self, kind: &[u8], offset: u64, length: u64) -> Result<Record, String> {
        self.bounds(offset, length)?;
        self.file
            .seek(SeekFrom::Start(offset))
            .map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut remaining = length;
        let mut buffer = [0u8; 65536];
        while remaining != 0 {
            active(self.cancel)?;
            let count = remaining.min(buffer.len() as u64) as usize;
            self.file
                .read_exact(&mut buffer[..count])
                .map_err(|e| e.to_string())?;
            hash.update(&buffer[..count]);
            remaining -= count as u64;
        }
        Ok(Record(kind.into(), length, hash.finalize().into()))
    }
    fn finish(&self) -> Result<(), String> {
        active(self.cancel)?;
        if FileFingerprint::from_metadata(&self.file.metadata().map_err(|e| e.to_string())?)
            != self.fingerprint
        {
            return Err("media changed while verifying metadata preservation".into());
        }
        Ok(())
    }
}
fn record(kind: &[u8], bytes: &[u8]) -> Record {
    Record(
        kind.into(),
        bytes.len() as u64,
        Sha256::digest(bytes).into(),
    )
}
fn push(records: &mut Vec<Record>, value: Record) -> Result<(), String> {
    if records.len() >= MAX_RECORDS {
        return Err("preservation exceeds 4,096 metadata records".into());
    }
    records.push(value);
    Ok(())
}
fn synchsafe(bytes: &[u8]) -> Result<usize, String> {
    if bytes.len() != 4 || bytes.iter().any(|b| b & 128 != 0) {
        return Err("invalid ID3 size".into());
    }
    Ok(bytes.iter().fold(0, |n, b| (n << 7) | usize::from(*b)))
}
fn id3_header(bytes: &[u8]) -> Result<(u8, usize), String> {
    if bytes.len() < 10
        || &bytes[..3] != b"ID3"
        || !matches!(bytes[3], 3 | 4)
        || bytes[4] != 0
        || bytes[5] != 0
    {
        return Err(
            "this ID3 version or extended/unsynchronized tag requires sidecar editing".into(),
        );
    }
    Ok((bytes[3], synchsafe(&bytes[6..10])?))
}
fn text(encoding: u8, bytes: &[u8]) -> Result<String, String> {
    let value = match encoding {
        0 => bytes.iter().map(|b| char::from(*b)).collect(),
        3 => std::str::from_utf8(bytes)
            .map_err(|_| "invalid UTF-8 in retained tag")?
            .into(),
        1 | 2 => {
            let (little, bytes) = if encoding == 2 {
                (false, bytes)
            } else if bytes.starts_with(&[0xff, 0xfe]) {
                (true, &bytes[2..])
            } else if bytes.starts_with(&[0xfe, 0xff]) {
                (false, &bytes[2..])
            } else {
                return Err("UTF-16 tag without byte-order mark requires sidecar editing".into());
            };
            if bytes.len() % 2 != 0 {
                return Err("odd UTF-16 retained tag length".into());
            }
            let units: Vec<_> = bytes
                .chunks_exact(2)
                .map(|b| {
                    if little {
                        u16::from_le_bytes([b[0], b[1]])
                    } else {
                        u16::from_be_bytes([b[0], b[1]])
                    }
                })
                .collect();
            String::from_utf16(&units).map_err(|_| "invalid UTF-16 in retained tag")?
        }
        _ => return Err("unsupported retained text encoding".into()),
    };
    Ok(value)
}
fn terminated(bytes: &[u8], encoding: u8) -> Result<(&[u8], &[u8]), String> {
    let width = if matches!(encoding, 1 | 2) { 2 } else { 1 };
    let end = bytes
        .chunks_exact(width)
        .position(|part| part.iter().all(|b| *b == 0))
        .ok_or("unterminated retained tag field")?
        * width;
    Ok((&bytes[..end], &bytes[end + width..]))
}
fn encoded_text(encoding: u8, bytes: &[u8]) -> Result<Vec<u8>, String> {
    // A final terminator is optional in ID3 text. Interior separators and all
    // nonempty values remain in the proof, including repeated values.
    Ok(text(encoding, bytes)?
        .trim_end_matches('\0')
        .as_bytes()
        .to_vec())
}
fn retained_frame(id: &[u8], flags: &[u8], bytes: &[u8]) -> Result<Record, String> {
    let mut key = id.to_vec();
    key.extend_from_slice(flags);
    if flags != [0, 0] {
        return Ok(record(&key, bytes));
    }
    if id.first() == Some(&b'T') {
        let (&encoding, value) = bytes.split_first().ok_or("empty retained ID3 text frame")?;
        return Ok(record(&key, &encoded_text(encoding, value)?));
    }
    if id == b"APIC" {
        let (&encoding, value) = bytes.split_first().ok_or("empty retained picture frame")?;
        let (mime, value) = terminated(value, 0)?;
        let (&kind, value) = value.split_first().ok_or("picture type missing")?;
        let (description, data) = terminated(value, encoding)?;
        let description = encoded_text(encoding, description)?;
        let mut normalized = Vec::new();
        normalized.extend_from_slice(&(mime.len() as u64).to_le_bytes());
        normalized.extend_from_slice(mime);
        normalized.push(kind);
        normalized.extend_from_slice(&(description.len() as u64).to_le_bytes());
        normalized.extend_from_slice(&description);
        normalized.extend_from_slice(data);
        return Ok(record(&key, &normalized));
    }
    Ok(record(&key, bytes))
}
fn id3(bytes: &[u8], changed: ChangedFields, inventory: &mut Inventory) -> Result<(), String> {
    let (version, length) = id3_header(bytes)?;
    if inventory.id3_version.replace(version).is_some() {
        return Err("multiple ID3 groups require sidecar editing".into());
    }
    let end = 10usize
        .checked_add(length)
        .filter(|end| *end <= bytes.len())
        .ok_or("truncated ID3 extent")?;
    if bytes[end..].iter().any(|b| *b != 0) {
        return Err("unrecognized data follows ID3 group".into());
    }
    let mut cursor = 10;
    let mut frames = 0;
    while cursor < end {
        let left = &bytes[cursor..end];
        if left[0] == 0 {
            if left.iter().any(|b| *b != 0) {
                return Err("nonzero data hidden in ID3 padding".into());
            }
            break;
        }
        if left.len() < 10
            || !left[..4]
                .iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            return Err("invalid retained ID3 frame header".into());
        }
        frames += 1;
        if frames > MAX_RECORDS {
            return Err("too many ID3 frames".into());
        }
        let length = if version == 4 {
            synchsafe(&left[4..8])?
        } else {
            u32::from_be_bytes(left[4..8].try_into().unwrap()) as usize
        };
        let next = cursor
            .checked_add(10)
            .and_then(|n| n.checked_add(length))
            .filter(|next| *next <= end)
            .ok_or("truncated retained ID3 frame")?;
        if changed.id3(&left[..4]) {
            if left[8..10] != [0, 0] {
                return Err("flagged edited ID3 frame requires sidecar editing".into());
            }
        } else {
            push(
                &mut inventory.fields,
                retained_frame(&left[..4], &left[8..10], &bytes[cursor + 10..next])?,
            )?;
        }
        cursor = next;
        if inventory.fields.len() >= MAX_RECORDS {
            return Err("too many retained ID3 frames".into());
        }
    }
    Ok(())
}
fn take_le<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8], String> {
    if bytes.len() < 4 {
        return Err("truncated Vorbis comment length".into());
    }
    let n = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    *bytes = &bytes[4..];
    if n > bytes.len() {
        return Err("truncated Vorbis comment value".into());
    }
    let (value, rest) = bytes.split_at(n);
    *bytes = rest;
    Ok(value)
}
fn vorbis(
    mut bytes: &[u8],
    changed: ChangedFields,
    inventory: &mut Inventory,
) -> Result<(), String> {
    if inventory.vendor.is_some() {
        return Err("multiple Vorbis comment blocks require sidecar editing".into());
    }
    inventory.vendor = Some(take_le(&mut bytes)?.into());
    if bytes.len() < 4 {
        return Err("missing Vorbis comment count".into());
    }
    let count = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    bytes = &bytes[4..];
    if count > MAX_RECORDS {
        return Err("too many Vorbis comments".into());
    }
    for _ in 0..count {
        let entry = take_le(&mut bytes)?;
        let split = entry
            .iter()
            .position(|b| *b == b'=')
            .ok_or("invalid Vorbis comment key")?;
        let (key, value) = (&entry[..split], &entry[split + 1..]);
        if key.is_empty()
            || !key.iter().all(|b| (0x20..=0x7d).contains(b))
            || std::str::from_utf8(value).is_err()
        {
            return Err("invalid retained Vorbis comment".into());
        }
        if !changed.vorbis(key) {
            push(
                &mut inventory.fields,
                record(&key.to_ascii_uppercase(), value),
            )?;
        }
    }
    if !bytes.is_empty() {
        return Err("unexpected data after Vorbis comments".into());
    }
    Ok(())
}
fn inventory(
    file: &mut File,
    format: FileType,
    changed: ChangedFields,
    cancel: &AtomicBool,
) -> Result<Inventory, String> {
    let mut input = Input::new(file, cancel)?;
    let mut inventory = Inventory::default();
    match format {
        FileType::Mpeg => {
            let header = input.bytes(0, input.length.min(10))?;
            let mut tail = 0;
            if header.starts_with(b"ID3") {
                let (_, size) = id3_header(&header)?;
                tail = 10 + size as u64;
                id3(&input.bytes(0, tail)?, changed, &mut inventory)?;
            }
            push(
                &mut inventory.outer,
                input.record(b"MPEG-tail", tail, input.length - tail)?,
            )?;
        }
        FileType::Wav | FileType::Aiff => {
            let header = input.bytes(0, 12)?;
            let little = format == FileType::Wav;
            if (little && (&header[..4] != b"RIFF" || &header[8..] != b"WAVE"))
                || (!little
                    && (&header[..4] != b"FORM" || !matches!(&header[8..], b"AIFF" | b"AIFC")))
            {
                return Err("unexpected tag-rewrite container".into());
            }
            let length = if little {
                u32::from_le_bytes(header[4..8].try_into().unwrap())
            } else {
                u32::from_be_bytes(header[4..8].try_into().unwrap())
            } as u64;
            if length + 8 != input.length {
                return Err("container extent differs from file; use sidecar editing".into());
            }
            push(&mut inventory.outer, record(b"container", &header[8..]))?;
            let mut offset = 12;
            let mut chunks = 0;
            while offset < input.length {
                chunks += 1;
                if chunks > MAX_RECORDS {
                    return Err("too many container chunks".into());
                }
                let header = input.bytes(offset, 8)?;
                let length = if little {
                    u32::from_le_bytes(header[4..8].try_into().unwrap())
                } else {
                    u32::from_be_bytes(header[4..8].try_into().unwrap())
                } as u64;
                let padded = length + length % 2;
                input.bounds(offset + 8, padded)?;
                if header[..4].eq_ignore_ascii_case(b"ID3 ") {
                    id3(&input.bytes(offset + 8, length)?, changed, &mut inventory)?;
                    if length % 2 != 0 && input.bytes(offset + 8 + length, 1)? != [0] {
                        return Err("nonzero ID3 chunk padding".into());
                    }
                } else {
                    push(
                        &mut inventory.outer,
                        input.record(&header[..4], offset, 8 + padded)?,
                    )?;
                }
                offset += 8 + padded;
            }
        }
        FileType::Flac => {
            if input.bytes(0, 4)? != b"fLaC" {
                return Err("FLAC preamble requires sidecar editing".into());
            }
            let mut offset = 4;
            let mut blocks = 0;
            loop {
                blocks += 1;
                if blocks > MAX_RECORDS {
                    return Err("too many FLAC metadata blocks".into());
                }
                let header = input.bytes(offset, 4)?;
                let kind = header[0] & 127;
                let length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as u64;
                input.bounds(offset + 4, length)?;
                match kind {
                    1 => {
                        if input.bytes(offset + 4, length)?.iter().any(|b| *b != 0) {
                            return Err("nonzero FLAC padding requires sidecar editing".into());
                        }
                    }
                    4 => vorbis(&input.bytes(offset + 4, length)?, changed, &mut inventory)?,
                    6 => push(
                        &mut inventory.pictures,
                        input.record(&[kind], offset + 4, length)?,
                    )?,
                    _ => push(
                        &mut inventory.outer,
                        input.record(&[kind], offset + 4, length)?,
                    )?,
                }
                offset += 4 + length;
                if header[0] & 128 != 0 {
                    break;
                }
            }
            push(
                &mut inventory.outer,
                input.record(b"FLAC-tail", offset, input.length - offset)?,
            )?;
        }
        _ => return Err("this format supports sidecar editing only".into()),
    }
    input.finish()?;
    inventory.fields.sort();
    Ok(inventory)
}

pub(crate) fn verify(
    original: &mut File,
    staged: &mut File,
    format: FileType,
    changed: ChangedFields,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let before = inventory(original, format, changed, cancel)?;
    let after = inventory(staged, format, changed, cancel)?;
    if before.outer != after.outer
        || before.fields != after.fields
        || before.pictures != after.pictures
        || before.vendor.is_some() && before.vendor != after.vendor
        || before.id3_version.is_some()
            && after.id3_version.is_some()
            && before.id3_version != after.id3_version
    {
        return Err("staged rewrite would change unrelated tags, artwork, or container data; use sidecar editing".into());
    }
    Ok(())
}

/// Inspect only container headers so the writer can keep an existing v2.3 tag.
pub(crate) fn id3_version(
    file: &mut File,
    format: FileType,
    cancel: &AtomicBool,
) -> Result<Option<u8>, String> {
    let mut input = Input::new(file, cancel)?;
    let result = match format {
        FileType::Mpeg => {
            let header = input.bytes(0, input.length.min(10))?;
            if header.starts_with(b"ID3") {
                Some(id3_header(&header)?.0)
            } else {
                None
            }
        }
        FileType::Wav | FileType::Aiff => {
            let mut offset = 12;
            let mut found = None;
            let mut chunks = 0;
            while offset < input.length {
                chunks += 1;
                if chunks > MAX_RECORDS {
                    return Err("too many container chunks".into());
                }
                let header = input.bytes(offset, 8)?;
                let length = if format == FileType::Wav {
                    u32::from_le_bytes(header[4..8].try_into().unwrap())
                } else {
                    u32::from_be_bytes(header[4..8].try_into().unwrap())
                } as u64;
                input.bounds(offset + 8, length + length % 2)?;
                if header[..4].eq_ignore_ascii_case(b"ID3 ") {
                    if found.is_some() {
                        return Err("multiple ID3 groups require sidecar editing".into());
                    }
                    if length < 10 {
                        return Err("short ID3 chunk".into());
                    }
                    found = Some(id3_header(&input.bytes(offset + 8, 10)?)?.0);
                }
                offset += 8 + length + length % 2;
            }
            found
        }
        FileType::Flac => None,
        _ => return Err("this format supports sidecar editing only".into()),
    };
    input.finish()?;
    Ok(result)
}

#[cfg(test)]
mod tests;
