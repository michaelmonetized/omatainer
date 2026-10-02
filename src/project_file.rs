//! Native project container, used only by project I/O workers.
//!
//! Version 1: magic, LE u32 version, LE u64 JSON length, LE u64 PCM length,
//! bounded JSON envelope, concatenated LE f32 assets, LE CRC32 of all preceding
//! bytes. Sample metadata stores float bits so signed zero is lossless. The
//! caller owns strict validation of its generic project state and media indices.
use crate::engine::dsp::Sample;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

pub const MAGIC: [u8; 8] = *b"OMATPRJ\0";
pub const FORMAT_VERSION: u32 = 1;
pub const DEFAULT_METADATA_LIMIT: usize = 64 * 1024 * 1024;
pub const DEFAULT_MEDIA_LIMIT: usize = 256;
pub const DEFAULT_PCM_LIMIT: u64 = 1024 * 1024 * 1024;
pub const MAX_SAMPLE_RATE: u32 = 768_000;
pub const MAX_CHANNELS: u16 = 32;
const HEADER_LEN: usize = 28;
const FOOTER_LEN: u64 = 4;
pub(crate) const CONTAINER_OVERHEAD: u64 = HEADER_LEN as u64 + FOOTER_LEN;
const IO_CHUNK: usize = 16 * 1024;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub struct Bundle<T> {
    pub state: T,
    pub media: Vec<Arc<Sample>>,
}
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_metadata_bytes: usize,
    pub max_media: usize,
    pub max_pcm_bytes: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_metadata_bytes: DEFAULT_METADATA_LIMIT,
            max_media: DEFAULT_MEDIA_LIMIT,
            max_pcm_bytes: DEFAULT_PCM_LIMIT,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overwrite {
    Never,
    Replace,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveOutcome {
    Durable,
    /// The destination already contains the complete new project. The caller
    /// must not treat this warning as a failed/precommit save or retry blindly.
    CommittedButDirectorySyncFailed(String),
}
#[derive(Debug)]
pub enum Error {
    Cancelled,
    Invalid(String),
    Io {
        stage: &'static str,
        source: io::Error,
    },
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Project file operation cancelled before commit"),
            Self::Invalid(reason) => write!(f, "Invalid project: {reason}"),
            Self::Io { stage, source } => write!(f, "Project {stage}: {source}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Self::Io { source, .. } = self {
            Some(source)
        } else {
            None
        }
    }
}
fn io_error(stage: &'static str, source: io::Error) -> Error {
    Error::Io { stage, source }
}
fn invalid(reason: impl Into<String>) -> Error {
    Error::Invalid(reason.into())
}
fn check_cancel(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Acquire) {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn memory(error: std::collections::TryReserveError) -> Error {
    invalid(format!("declared allocation cannot be reserved: {error}"))
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<T> {
    format_version: u32,
    state: T,
    media: Vec<Media>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Media {
    name: String,
    path: String,
    sample_rate: u32,
    channels: u16,
    values: u64,
    bpm_bits: u32,
    peaks_bits: Vec<[u32; 3]>,
}
impl Media {
    fn validate(&self) -> Result<u64, Error> {
        validate_shape(
            self.sample_rate,
            self.channels,
            self.values,
            f32::from_bits(self.bpm_bits),
        )?;
        for peak in &self.peaks_bits {
            if peak.iter().any(|bits| !f32::from_bits(*bits).is_finite()) {
                return Err(invalid("nonfinite waveform peak"));
            }
        }
        self.values
            .checked_mul(4)
            .ok_or_else(|| invalid("PCM size overflow"))
    }
}
fn validate_shape(sr: u32, ch: u16, values: u64, bpm: f32) -> Result<(), Error> {
    if sr == 0 || sr > MAX_SAMPLE_RATE {
        return Err(invalid(format!(
            "sample rate must be 1..={MAX_SAMPLE_RATE}"
        )));
    }
    if ch == 0 || ch > MAX_CHANNELS {
        return Err(invalid(format!("channel count must be 1..={MAX_CHANNELS}")));
    }
    if values % ch as u64 != 0 {
        return Err(invalid("PCM has an incomplete channel frame"));
    }
    if !bpm.is_finite() {
        return Err(invalid("nonfinite media BPM"));
    }
    Ok(())
}
fn total_size(metadata: u64, pcm: u64) -> Result<u64, Error> {
    metadata
        .checked_add(pcm)
        .and_then(|n| n.checked_add(CONTAINER_OVERHEAD))
        .ok_or_else(|| invalid("container size overflow"))
}

struct BoundedJson {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let end = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|n| *n <= self.limit)
            .ok_or_else(|| io::Error::other("metadata exceeds configured byte limit"))?;
        if end > self.bytes.capacity() {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(1024)
                .max(end)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(io::Error::other)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn encode_metadata<'a, T: Serialize>(
    bundle: &'a Bundle<T>,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<(Vec<u8>, u64), Error> {
    if bundle.media.len() > limits.max_media {
        return Err(invalid("too many embedded media assets"));
    }
    let mut pcm = 0u64;
    let mut metadata_floor = 0usize;
    // Validate every count before creating copied names/peaks or PCM buffers.
    for sample in &bundle.media {
        check_cancel(cancel)?;
        validate_shape(sample.sr, sample.ch, sample.data.len() as u64, sample.bpm)?;
        pcm = pcm
            .checked_add(
                (sample.data.len() as u64)
                    .checked_mul(4)
                    .ok_or_else(|| invalid("PCM size overflow"))?,
            )
            .ok_or_else(|| invalid("total PCM size overflow"))?;
        if pcm > limits.max_pcm_bytes {
            return Err(invalid(
                "embedded PCM exceeds configured desktop byte limit",
            ));
        }
        metadata_floor = metadata_floor
            .checked_add(sample.name.len())
            .and_then(|n| n.checked_add(sample.path.len()))
            .and_then(|n| {
                sample
                    .peaks
                    .len()
                    .checked_mul(12)
                    .and_then(|p| n.checked_add(p))
            })
            .ok_or_else(|| invalid("media metadata size overflow"))?;
        if metadata_floor > limits.max_metadata_bytes {
            return Err(invalid("media metadata/peaks exceed configured byte limit"));
        }
        for values in sample.data.chunks(IO_CHUNK / 4) {
            check_cancel(cancel)?;
            if values.iter().any(|v| !v.is_finite()) {
                return Err(invalid("nonfinite PCM value"));
            }
        }
        if sample.peaks.iter().flatten().any(|v| !v.is_finite()) {
            return Err(invalid("nonfinite waveform peak"));
        }
    }
    let media = bundle
        .media
        .iter()
        .map(|sample| Media {
            name: sample.name.clone(),
            path: sample.path.clone(),
            sample_rate: sample.sr,
            channels: sample.ch,
            values: sample.data.len() as u64,
            bpm_bits: sample.bpm.to_bits(),
            peaks_bits: sample
                .peaks
                .iter()
                .map(|peak| peak.map(f32::to_bits))
                .collect(),
        })
        .collect();
    let envelope = Envelope {
        format_version: FORMAT_VERSION,
        state: &bundle.state,
        media,
    };
    let mut writer = BoundedJson {
        bytes: Vec::new(),
        limit: limits.max_metadata_bytes,
    };
    serde_json::to_writer(&mut writer, &envelope)
        .map_err(|error| invalid(format!("metadata serialization: {error}")))?;
    check_cancel(cancel)?;
    total_size(writer.bytes.len() as u64, pcm)?;
    Ok((writer.bytes, pcm))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    MetadataReady,
    TempCreated,
    PayloadChunk,
    BeforeCommit,
    AfterCommit,
    DirectorySync,
}
struct Temporary {
    path: PathBuf,
}
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
fn create_temporary(parent: &Path) -> Result<(Temporary, File), Error> {
    for _ in 0..32 {
        let path = parent.join(format!(
            ".omatainer-project-{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(file) => return Ok((Temporary { path }, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(io_error("create temporary", error)),
        }
    }
    Err(invalid("could not reserve a unique temporary project file"))
}

/// All Err outcomes occur before publication and preserve the prior destination.
/// Cancellation is cooperative; publication wins a cancellation arriving later.
pub fn save<T: Serialize>(
    path: &Path,
    bundle: &Bundle<T>,
    overwrite: Overwrite,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<SaveOutcome, Error> {
    save_with_hook(path, bundle, overwrite, limits, cancel, &mut |_| Ok(()))
}
fn save_with_hook<T: Serialize>(
    path: &Path,
    bundle: &Bundle<T>,
    overwrite: Overwrite,
    limits: &Limits,
    cancel: &AtomicBool,
    hook: &mut impl FnMut(Phase) -> io::Result<()>,
) -> Result<SaveOutcome, Error> {
    check_cancel(cancel)?;
    match fs::symlink_metadata(path) {
        Ok(_) if overwrite == Overwrite::Never => {
            return Err(io_error(
                "create destination",
                io::Error::from(io::ErrorKind::AlreadyExists),
            ))
        }
        Ok(meta) if !meta.file_type().is_file() => {
            return Err(invalid(
                "replacement destination must be a regular file, not a symlink or directory",
            ))
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error("inspect destination", error)),
    }
    let (metadata, pcm) = encode_metadata(bundle, limits, cancel)?;
    hook(Phase::MetadataReady).map_err(|e| io_error("metadata preparation", e))?;
    check_cancel(cancel)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let (temporary, mut file) = create_temporary(parent)?;
    hook(Phase::TempCreated).map_err(|e| io_error("temporary preparation", e))?;
    let mut header = [0u8; HEADER_LEN];
    header[..8].copy_from_slice(&MAGIC);
    header[8..12].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    header[12..20].copy_from_slice(&(metadata.len() as u64).to_le_bytes());
    header[20..28].copy_from_slice(&pcm.to_le_bytes());
    let mut crc = Crc::new();
    write_checked(&mut file, &header, &mut crc, cancel)?;
    for chunk in metadata.chunks(IO_CHUNK) {
        write_checked(&mut file, chunk, &mut crc, cancel)?;
    }
    let mut bytes = [0u8; IO_CHUNK];
    for sample in &bundle.media {
        for values in sample.data.chunks(IO_CHUNK / 4) {
            for (value, bytes) in values.iter().zip(bytes.chunks_exact_mut(4)) {
                bytes.copy_from_slice(&value.to_le_bytes());
            }
            write_checked(&mut file, &bytes[..values.len() * 4], &mut crc, cancel)?;
            hook(Phase::PayloadChunk).map_err(|e| io_error("write PCM", e))?;
        }
    }
    file.write_all(&crc.finish().to_le_bytes())
        .map_err(|e| io_error("write checksum", e))?;
    file.sync_all().map_err(|e| io_error("sync temporary", e))?;
    hook(Phase::BeforeCommit).map_err(|e| io_error("prepare commit", e))?;
    check_cancel(cancel)?;
    match overwrite {
        Overwrite::Never => fs::hard_link(&temporary.path, path),
        Overwrite::Replace => fs::rename(&temporary.path, path),
    }
    .map_err(|e| io_error("publish project", e))?;
    // No error or cancellation after this point may claim the save did not
    // happen. Clean temporary links before syncing the committed directory.
    drop(file);
    drop(temporary);
    let sync = (|| {
        hook(Phase::AfterCommit)?;
        hook(Phase::DirectorySync)?;
        File::open(parent)?.sync_all()
    })();
    Ok(match sync {
        Ok(()) => SaveOutcome::Durable,
        Err(error) => SaveOutcome::CommittedButDirectorySyncFailed(error.to_string()),
    })
}
fn write_checked(
    file: &mut File,
    bytes: &[u8],
    crc: &mut Crc,
    cancel: &AtomicBool,
) -> Result<(), Error> {
    check_cancel(cancel)?;
    file.write_all(bytes)
        .map_err(|e| io_error("write temporary", e))?;
    crc.update(bytes);
    Ok(())
}

pub fn load<T: DeserializeOwned>(
    path: &Path,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<Bundle<T>, Error> {
    check_cancel(cancel)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| io_error("open", e))?;
    load_from_file(file, limits, cancel)
}

/// Decode the caller's already-open regular file. Recovery uses this entry
/// point so digest verification and decoding cannot select different inodes.
pub(crate) fn load_from_file<T: DeserializeOwned>(
    file: File,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<Bundle<T>, Error> {
    load_from_file_measured(file, limits, cancel).map(|(bundle, _)| bundle)
}

/// Also return validated encoded metadata bytes, so a multi-sidecar reader can
/// apply one aggregate envelope budget before allocating the next asset.
pub(crate) fn load_from_file_measured<T: DeserializeOwned>(
    mut file: File,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<(Bundle<T>, usize), Error> {
    check_cancel(cancel)?;
    let stat = file.metadata().map_err(|e| io_error("inspect input", e))?;
    if !stat.is_file() {
        return Err(invalid("input is not a regular file"));
    }
    let mut header = [0u8; HEADER_LEN];
    read_exact(&mut file, &mut header)?;
    if header[..8] != MAGIC {
        return Err(invalid("unrecognized file magic"));
    }
    let version = u32::from_le_bytes(header[8..12].try_into().unwrap());
    if version != FORMAT_VERSION {
        return Err(invalid(format!(
            "unsupported format version {version}; this build reads {FORMAT_VERSION}"
        )));
    }
    let metadata_len = u64::from_le_bytes(header[12..20].try_into().unwrap());
    let pcm_len = u64::from_le_bytes(header[20..28].try_into().unwrap());
    if metadata_len > limits.max_metadata_bytes as u64 || pcm_len > limits.max_pcm_bytes {
        return Err(invalid(
            "declared metadata or PCM exceeds configured desktop limit",
        ));
    }
    if stat.len() != total_size(metadata_len, pcm_len)? {
        return Err(invalid("container is truncated or has trailing data"));
    }
    let metadata_len = usize::try_from(metadata_len)
        .map_err(|_| invalid("metadata length exceeds address space"))?;
    let mut metadata = Vec::new();
    metadata.try_reserve_exact(metadata_len).map_err(memory)?;
    metadata.resize(metadata_len, 0);
    for chunk in metadata.chunks_mut(IO_CHUNK) {
        check_cancel(cancel)?;
        read_exact(&mut file, chunk)?;
    }
    let envelope: Envelope<T> =
        serde_json::from_slice(&metadata).map_err(|error| invalid(format!("metadata: {error}")))?;
    if envelope.format_version != FORMAT_VERSION {
        return Err(invalid("header and metadata versions disagree"));
    }
    if envelope.media.len() > limits.max_media {
        return Err(invalid("too many embedded media assets"));
    }
    let mut declared = 0u64;
    for media in &envelope.media {
        declared = declared
            .checked_add(media.validate()?)
            .ok_or_else(|| invalid("total PCM size overflow"))?;
        if declared > limits.max_pcm_bytes {
            return Err(invalid("total PCM exceeds configured desktop byte limit"));
        }
        usize::try_from(media.values).map_err(|_| invalid("PCM values exceed address space"))?;
    }
    if declared != pcm_len {
        return Err(invalid(
            "media declarations do not match PCM payload length",
        ));
    }
    let mut crc = Crc::new();
    crc.update(&header);
    crc.update(&metadata);
    let mut media = Vec::new();
    media
        .try_reserve_exact(envelope.media.len())
        .map_err(memory)?;
    let mut bytes = [0u8; IO_CHUNK];
    for asset in envelope.media {
        check_cancel(cancel)?;
        let len = asset.values as usize;
        let mut data = Vec::new();
        data.try_reserve_exact(len).map_err(memory)?;
        while data.len() < len {
            let count = (len - data.len()).min(IO_CHUNK / 4);
            let chunk = &mut bytes[..count * 4];
            check_cancel(cancel)?;
            read_exact(&mut file, chunk)?;
            crc.update(chunk);
            for value in chunk.chunks_exact(4) {
                let value = f32::from_le_bytes(value.try_into().unwrap());
                if !value.is_finite() {
                    return Err(invalid("nonfinite PCM value"));
                }
                data.push(value);
            }
        }
        media.push(Arc::new(Sample {
            name: asset.name,
            path: asset.path,
            sr: asset.sample_rate,
            ch: asset.channels,
            data,
            bpm: f32::from_bits(asset.bpm_bits),
            peaks: Arc::new(
                asset
                    .peaks_bits
                    .into_iter()
                    .map(|peak| peak.map(f32::from_bits))
                    .collect(),
            ),
        }));
    }
    let mut footer = [0; 4];
    read_exact(&mut file, &mut footer)?;
    if u32::from_le_bytes(footer) != crc.finish() {
        return Err(invalid("checksum mismatch; project bytes are damaged"));
    }
    let mut extra = [0; 1];
    if file
        .read(&mut extra)
        .map_err(|e| io_error("check end of file", e))?
        != 0
    {
        return Err(invalid("trailing data"));
    }
    check_cancel(cancel)?;
    Ok((
        Bundle {
            state: envelope.state,
            media,
        },
        metadata_len,
    ))
}
fn read_exact(file: &mut File, bytes: &mut [u8]) -> Result<(), Error> {
    file.read_exact(bytes).map_err(|e| io_error("read", e))
}

const fn crc_table() -> [u32; 256] {
    let mut table = [0; 256];
    let mut n = 0;
    while n < 256 {
        let mut value = n as u32;
        let mut bit = 0;
        while bit < 8 {
            value = if value & 1 != 0 {
                (value >> 1) ^ 0xedb88320
            } else {
                value >> 1
            };
            bit += 1;
        }
        table[n] = value;
        n += 1;
    }
    table
}
const CRC_TABLE: [u32; 256] = crc_table();
struct Crc(u32);
impl Crc {
    fn new() -> Self {
        Self(u32::MAX)
    }
    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 = CRC_TABLE[((self.0 ^ *byte as u32) & 255) as usize] ^ (self.0 >> 8);
        }
    }
    fn finish(self) -> u32 {
        !self.0
    }
}

#[cfg(test)]
mod tests;
