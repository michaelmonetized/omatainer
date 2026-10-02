//! Bounded worker-side embedded-tag inspection. Filename hints and sidecar
//! edits are deliberately separate from observations of actual file metadata.
use crate::engine::media_source::FileFingerprint;
use crate::media_location::{Location, Snapshot};
use lofty::{
    config::{apply_global_options, GlobalOptions, ParseOptions, ParsingMode},
    file::{FileType, TaggedFile, TaggedFileExt},
    probe::Probe,
    tag::{ItemKey, TagType},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    os::unix::fs::OpenOptionsExt,
    sync::atomic::{AtomicBool, Ordering},
};

pub(crate) mod payload;
pub(crate) mod preservation;
pub(crate) mod write;

const MAX_SOURCE: u64 = 8 * 1024 * 1024 * 1024;
const TAG_READ_BYTES: u64 = 16 * 1024 * 1024;
const TAG_OPERATIONS: u64 = 1_000_000;
const MAX_ITEM_ALLOCATION: usize = 4 * 1024 * 1024;
pub(crate) const MAX_TEXT_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TagSource {
    Id3v1,
    Id3v2,
    Vorbis,
    RiffInfo,
    AiffText,
    Mp4,
    Ape,
    Other,
}
impl TagSource {
    fn from_tag(tag: TagType) -> Self {
        match tag {
            TagType::Id3v1 => Self::Id3v1,
            TagType::Id3v2 => Self::Id3v2,
            TagType::VorbisComments => Self::Vorbis,
            TagType::RiffInfo => Self::RiffInfo,
            TagType::AiffText => Self::AiffText,
            TagType::Mp4Ilst => Self::Mp4,
            TagType::Ape => Self::Ape,
            _ => Self::Other,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Id3v1 => "ID3v1",
            Self::Id3v2 => "ID3v2",
            Self::Vorbis => "Vorbis comments",
            Self::RiffInfo => "RIFF INFO",
            Self::AiffText => "AIFF text",
            Self::Mp4 => "MP4 tags",
            Self::Ape => "APE tags",
            Self::Other => "embedded tag",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Field {
    pub value: String,
    pub source: TagSource,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fields {
    pub title: Option<Field>,
    pub artist: Option<Field>,
    pub bpm: Option<Field>,
    pub key: Option<Field>,
}
#[derive(Clone, Debug)]
pub(crate) struct Observation {
    pub format: FileType,
    pub fingerprint: FileFingerprint,
    pub fields: Fields,
    pub notices: Vec<String>,
    /// False when only tolerant inspection succeeded. This does not replace
    /// strict, artwork-enabled parsing and preservation checks before a write.
    pub rewrite_eligible: bool,
}
impl Observation {
    pub fn supports_write(&self) -> bool {
        self.rewrite_eligible
            && matches!(
                self.format,
                FileType::Mpeg | FileType::Flac | FileType::Wav | FileType::Aiff
            )
    }
}
pub(crate) trait Cancellation {
    fn cancelled(&self) -> bool;
}
impl Cancellation for AtomicBool {
    fn cancelled(&self) -> bool {
        self.load(Ordering::Acquire)
    }
}
impl Cancellation for std::sync::Arc<AtomicBool> {
    fn cancelled(&self) -> bool {
        self.load(Ordering::Acquire)
    }
}
fn active(cancel: &dyn Cancellation) -> Result<(), String> {
    if cancel.cancelled() {
        Err("tag inspection cancelled".into())
    } else {
        Ok(())
    }
}
pub(crate) fn text_valid(value: &str) -> bool {
    value.len() <= MAX_TEXT_BYTES && !value.chars().any(char::is_control)
}
pub(crate) fn parse_options(art: bool) -> ParseOptions {
    ParseOptions::new()
        .read_properties(false)
        .read_cover_art(art)
        .implicit_conversions(false)
        .parsing_mode(ParsingMode::Strict)
}

/// A parser cannot seek outside the captured regular file or perform unbounded
/// reads/seeks. Cancellation is checked at every operation, including seeks
/// that skip artwork/audio. Blocking kernel I/O remains cooperatively cancelled.
struct Budget<'a> {
    file: &'a mut File,
    cancel: &'a dyn Cancellation,
    length: u64,
    bytes: u64,
    operations: u64,
}
impl Budget<'_> {
    fn check(&mut self) -> io::Result<()> {
        active(self.cancel).map_err(io::Error::other)?;
        self.operations = self
            .operations
            .checked_sub(1)
            .ok_or_else(|| io::Error::other("tag parser operation limit exceeded"))?;
        Ok(())
    }
}
impl Read for Budget<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.check()?;
        if buf.is_empty() {
            return Ok(0);
        }
        let remaining = self.length.saturating_sub(self.file.stream_position()?);
        if remaining == 0 {
            return Ok(0);
        }
        if self.bytes == 0 {
            return Err(io::Error::other("tag parser exceeded 16 MiB read budget"));
        }
        let count = buf
            .len()
            .min(self.bytes.min(remaining).min(usize::MAX as u64) as usize);
        let n = self.file.read(&mut buf[..count])?;
        self.bytes -= n as u64;
        Ok(n)
    }
}
impl Seek for Budget<'_> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.check()?;
        let target = match pos {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.file.stream_position()? as i128 + n as i128,
            SeekFrom::End(n) => self.length as i128 + n as i128,
        };
        if target < 0 || target > self.length as i128 {
            return Err(io::Error::other(
                "tag parser attempted to seek outside captured file",
            ));
        }
        self.file.seek(SeekFrom::Start(target as u64))
    }
}
pub(crate) fn read_tagged(
    file: &mut File,
    cancel: &dyn Cancellation,
    art: bool,
) -> Result<TaggedFile, String> {
    read_tagged_mode(file, cancel, art, ParsingMode::Strict)
}
fn read_tagged_mode(
    file: &mut File,
    cancel: &dyn Cancellation,
    art: bool,
    mode: ParsingMode,
) -> Result<TaggedFile, String> {
    active(cancel)?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE {
        return Err("tags require a regular file within 8 GiB".into());
    }
    file.rewind().map_err(|e| e.to_string())?;
    // Lofty options are thread-local. Compressed ID3 support is disabled in the
    // pinned dependency: unsupported compressed frames fail instead of inflating
    // an unbounded collection of compressed metadata into the worker heap.
    apply_global_options(
        GlobalOptions::new()
            .allocation_limit(MAX_ITEM_ALLOCATION)
            .use_custom_resolvers(false)
            .preserve_format_specific_items(true),
    );
    let reader = Budget {
        file,
        cancel,
        length: metadata.len(),
        bytes: TAG_READ_BYTES,
        operations: TAG_OPERATIONS,
    };
    let parsed = Probe::new(reader)
        .guess_file_type()
        .map_err(|e| e.to_string())?
        .options(parse_options(art).parsing_mode(mode))
        .read()
        .map_err(|e| format!("embedded tags unavailable: {e}"))?;
    active(cancel)?;
    if parsed.tags().len() > 16
        || parsed
            .tags()
            .iter()
            .map(|tag| tag.item_count() as usize)
            .sum::<usize>()
            > 4096
    {
        return Err("embedded tags exceed 16 groups or 4,096 fields".into());
    }
    Ok(parsed)
}
fn fields(parsed: &TaggedFile) -> (Fields, Vec<String>) {
    let mut tags: Vec<_> = parsed.tags().iter().collect();
    let primary = parsed.primary_tag_type();
    tags.sort_by_key(|t| t.tag_type() != primary);
    let mut notices = Vec::new();
    let mut select = |keys: &[ItemKey], label: &str| {
        let mut selected: Option<Field> = None;
        let mut invalid = false;
        let mut conflict = false;
        for tag in &tags {
            for value in keys.iter().flat_map(|key| tag.get_strings(*key)) {
                if value.trim().is_empty() {
                    continue;
                }
                if !text_valid(value) {
                    invalid = true;
                    continue;
                }
                if let Some(old) = &selected {
                    conflict |= old.value != value;
                } else {
                    selected = Some(Field {
                        value: value.into(),
                        source: TagSource::from_tag(tag.tag_type()),
                    });
                }
            }
        }
        if invalid {
            notices.push(format!(
                "{label}: tag value exceeds 4,096 bytes or contains control characters"
            ));
        }
        if conflict {
            notices.push(format!(
                "{label}: embedded tag values disagree; using {}",
                selected.as_ref().unwrap().source.label()
            ));
        }
        selected
    };
    let fields = Fields {
        title: select(&[ItemKey::TrackTitle], "Title"),
        artist: select(&[ItemKey::TrackArtist], "Artist"),
        bpm: select(&[ItemKey::Bpm, ItemKey::IntegerBpm], "BPM"),
        key: select(&[ItemKey::InitialKey], "Key"),
    };
    (fields, notices)
}
pub(crate) fn inspect(
    location: &Location,
    expected: FileFingerprint,
    cancel: &AtomicBool,
) -> Result<Observation, String> {
    inspect_cancellable(location, expected, cancel)
}
pub(crate) fn inspect_cancellable(
    location: &Location,
    expected: FileFingerprint,
    cancel: &dyn Cancellation,
) -> Result<Observation, String> {
    active(cancel)?;
    let before = Snapshot::discover().map_err(|e| e.to_string())?;
    location.recheck_with(&before).map_err(|e| e.to_string())?;
    let access = before.access(&location.path).map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&location.path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || FileFingerprint::from_metadata(&metadata) != expected {
        return Err("media changed before tag inspection".into());
    }
    let (parsed, strict_failure) = match read_tagged(&mut file, cancel, false) {
        Ok(parsed) => (parsed, None),
        Err(error) => {
            active(cancel)?;
            // Legacy tags can be readable while failing strict preservation
            // requirements (for example an empty ID3v1 year). Read-only values
            // remain useful, but must never authorize a lossy rewrite.
            let parsed = read_tagged_mode(&mut file, cancel, false, ParsingMode::BestAttempt)?;
            let mut detail = String::new();
            for ch in error.chars().filter(|ch| !ch.is_control()) {
                if detail.len() + ch.len_utf8() > 512 {
                    break;
                }
                detail.push(ch);
            }
            (parsed, Some(format!(
                "Read-only tag observation: strict validation failed ({detail}); embedded rewriting is unavailable"
            )))
        }
    };
    let (fields, mut notices) = fields(&parsed);
    let rewrite_eligible = strict_failure.is_none();
    notices.extend(strict_failure);
    active(cancel)?;
    let after = Snapshot::discover().map_err(|e| e.to_string())?;
    access
        .check(&after, &location.path)
        .map_err(|e| e.to_string())?;
    location.recheck_with(&after).map_err(|e| e.to_string())?;
    if FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?) != expected
        || after
            .inspect(location)
            .map(|m| FileFingerprint::from_metadata(&m))
            .ok()
            != Some(expected)
    {
        return Err("media changed during tag inspection".into());
    }
    Ok(Observation {
        format: parsed.file_type(),
        fingerprint: expected,
        fields,
        notices,
        rewrite_eligible,
    })
}

#[cfg(test)]
mod tests;
