//! Bounded native session archives. Archive names never become arbitrary paths.
use crate::{
    project_dependencies::Key,
    project_file::{self, Bundle, Limits, Overwrite, SaveOutcome},
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

const MAGIC: &[u8; 8] = b"OMATPCK\0";
const VERSION: u32 = 1;
const MAX_METADATA: usize = 64 * 1024 * 1024;
pub(crate) const MAX_SOURCE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 3 * 1024 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Media {
    pub key: Key,
    pub name: String,
    pub frames: usize,
    pub sample_rate: u32,
    pub channels: u16,
    pub source: Option<String>,
    pub collected: Option<String>,
    pub rights: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Device {
    pub placement: String,
    pub identifier: String,
    pub settings_sha256: [u8; 32],
    pub unavailable: bool,
    pub state_schema: Option<u32>,
    pub rights: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub path: String,
    pub bytes: u64,
    pub sha256: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub schema: u32,
    pub application: String,
    pub license_manifest: String,
    pub license_notices: String,
    pub media: Vec<Media>,
    pub devices: Vec<Device>,
    pub unresolved: Vec<String>,
    pub entries: Vec<Entry>,
}
fn hex(hash: [u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub(crate) fn asset_name(hash: [u8; 32]) -> String {
    format!("{}.source", hex(hash))
}
fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Portable project operation cancelled before publication".into())
    } else {
        Ok(())
    }
}
impl Manifest {
    /// Read bounded distribution notices retained by the archive publisher.
    /// Returns the existing license catalogue or rejects oversized display fields.
    pub fn catalog(&self) -> Result<crate::licenses::Catalog, String> {
        let catalog =
            crate::licenses::Catalog::parse(&self.license_manifest, &self.license_notices)?;
        if catalog.manifest.entries.iter().any(|entry| {
            entry.id.len() > 1024
                || entry.name.len() > 4096
                || entry.category.len() > 128
                || entry.license.len() > 4096
                || entry.redistribution.len() > 16384
        }) {
            return Err("Oversized portable distribution notice".into());
        }
        Ok(catalog)
    }
    /// Validate fixed archive paths, checksums and dependency references.
    /// Returns the exact payload length or rejects unsupported/ambiguous data.
    pub fn validate(&self) -> Result<u64, String> {
        if self.schema != VERSION
            || self.application.len() > 256
            || self.media.len() > project_file::DEFAULT_MEDIA_LIMIT
            || self.entries.is_empty()
            || self.entries.len() > project_file::DEFAULT_MEDIA_LIMIT + 1
            || self.devices.len() > 81920
            || self.unresolved.len() > 4096
        {
            return Err("Unsupported or oversized portable project manifest".into());
        }
        self.catalog()?;
        let mut paths = HashSet::new();
        let mut keys = HashSet::new();
        let mut size = 0u64;
        for (index, entry) in self.entries.iter().enumerate() {
            if (index == 0 && entry.path != "session.omat")
                || (index != 0 && entry.path != asset_name(entry.sha256))
                || !paths.insert(&entry.path)
                || entry.bytes > MAX_ARCHIVE_BYTES
            {
                return Err("Invalid, duplicated or unsafe archive entry".into());
            }
            size = size
                .checked_add(entry.bytes)
                .filter(|size| *size <= MAX_ARCHIVE_BYTES)
                .ok_or("Portable archive exceeds 3 GiB")?;
        }
        let source_size: u64 = self.entries.iter().skip(1).map(|entry| entry.bytes).sum();
        if source_size > MAX_SOURCE_BYTES {
            return Err("Collected original sources exceed 1 GiB".into());
        }
        for media in &self.media {
            if !keys.insert(&media.key)
                || media.key.original_path.len() > 4096
                || media.key.original_path.contains('\0')
                || media.name.len() > 4096
                || media
                    .source
                    .as_ref()
                    .is_some_and(|source| source.len() > 8192 || source.contains('\0'))
                || media.rights.len() > 4096
                || media.sample_rate == 0
                || media.sample_rate > project_file::MAX_SAMPLE_RATE
                || media.channels == 0
                || media.channels > project_file::MAX_CHANNELS
                || media
                    .collected
                    .as_ref()
                    .is_some_and(|path| path == "session.omat" || !paths.contains(path))
            {
                return Err("Invalid portable media dependency".into());
            }
        }
        if self.devices.iter().any(|device| {
            device.placement.len() > 8192
                || device.placement.contains('\0')
                || device.identifier.is_empty()
                || device.identifier.len() > 1024
                || device.identifier.chars().any(char::is_control)
                || device.state_schema == Some(0)
                || device.rights.len() > 4096
        }) || self
            .unresolved
            .iter()
            .any(|line| line.len() > 16384 || line.contains('\0'))
        {
            return Err("Invalid portable device or unresolved dependency".into());
        }
        if self.entries.iter().skip(1).any(|entry| {
            !self
                .media
                .iter()
                .any(|media| media.collected.as_ref() == Some(&entry.path))
        }) {
            return Err("Unreferenced archive source entry".into());
        }
        Ok(size)
    }
}

pub(crate) struct Stage {
    pub path: PathBuf,
    identity: (u64, u64),
    retained: bool,
}
impl Stage {
    /// Create a private sibling staging directory on the destination filesystem.
    /// The returned owner removes only its own still-identical uncommitted path.
    pub fn new(parent: &Path) -> Result<Self, String> {
        let path = parent.join(format!(
            ".omatainer-portable-{}",
            crate::sampler_bank::BankId::new()?
        ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|e| e.to_string())?;
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        Ok(Self {
            path,
            identity: (metadata.dev(), metadata.ino()),
            retained: false,
        })
    }
    fn recheck(&self) -> Result<(), String> {
        let metadata = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || (metadata.dev(), metadata.ino()) != self.identity {
            return Err("Portable staging directory changed".into());
        }
        Ok(())
    }
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.retained && self.recheck().is_ok() {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
fn read_regular(path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Portable input is not a regular file".into());
    }
    Ok(file)
}
fn create(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| e.to_string())
}
fn measured(path: &Path, cancel: &AtomicBool) -> Result<Entry, String> {
    let mut file = read_regular(path)?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; CHUNK];
    let mut size = 0u64;
    loop {
        active(cancel)?;
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size = size
            .checked_add(n as u64)
            .filter(|size| *size <= MAX_ARCHIVE_BYTES)
            .ok_or("Portable input exceeds archive bound")?;
        hash.update(&bytes[..n]);
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    if size != before.len()
        || crate::engine::media_source::FileFingerprint::from_metadata(&before)
            != crate::engine::media_source::FileFingerprint::from_metadata(&after)
    {
        return Err("Portable input changed during measurement".into());
    }
    Ok(Entry {
        path: path
            .file_name()
            .ok_or("Missing staged filename")?
            .to_str()
            .ok_or("Non-Unicode staged filename")?
            .into(),
        bytes: size,
        sha256: hash.finalize().into(),
    })
}

/// Write one self-contained session and prepared original sources as an archive.
/// The caller owns captured-state validation and source preparation. Existing
/// destinations are refused; cancellation before the final hard link preserves them.
pub(crate) fn export<T: Serialize>(
    destination: &Path,
    stage: &Stage,
    bundle: &Bundle<T>,
    mut manifest: Manifest,
    cancel: &AtomicBool,
) -> Result<SaveOutcome, String> {
    active(cancel)?;
    stage.recheck()?;
    if fs::symlink_metadata(destination).is_ok() {
        return Err("Archive destination already exists; choose a new path".into());
    }
    project_file::save(
        &stage.path.join("session.omat"),
        bundle,
        Overwrite::Never,
        &Limits::default(),
        cancel,
    )
    .map_err(|e| e.to_string())?;
    let mut entries = vec![measured(&stage.path.join("session.omat"), cancel)?];
    let mut seen = HashSet::new();
    for name in manifest
        .media
        .iter()
        .filter_map(|media| media.collected.as_ref())
    {
        if !seen.insert(name.clone()) {
            continue;
        }
        if name.len() != 71
            || !name.is_ascii()
            || !name.ends_with(".source")
            || !name.as_bytes()[..64]
                .iter()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("Invalid prepared source filename".into());
        }
        let entry = measured(&stage.path.join(name), cancel)?;
        if entry.path != asset_name(entry.sha256) {
            return Err("Collected source differs from its checksum".into());
        }
        entries.push(entry);
    }
    manifest.entries = entries;
    manifest.validate()?;
    let metadata = serde_json::to_vec(&manifest).map_err(|e| e.to_string())?;
    if metadata.len() > MAX_METADATA {
        return Err("Portable manifest exceeds 64 MiB".into());
    }
    let temporary = stage.path.join("package.tmp");
    let mut output = create(&temporary)?;
    let mut digest = Sha256::new();
    let mut write = |bytes: &[u8]| -> Result<(), String> {
        active(cancel)?;
        output.write_all(bytes).map_err(|e| e.to_string())?;
        digest.update(bytes);
        Ok(())
    };
    write(MAGIC)?;
    write(&VERSION.to_le_bytes())?;
    write(&(metadata.len() as u64).to_le_bytes())?;
    write(&metadata)?;
    let mut buffer = [0u8; CHUNK];
    for entry in &manifest.entries {
        let mut file = read_regular(&stage.path.join(&entry.path))?;
        let mut left = entry.bytes;
        let mut hash = Sha256::new();
        while left != 0 {
            let count = left.min(CHUNK as u64) as usize;
            file.read_exact(&mut buffer[..count])
                .map_err(|e| e.to_string())?;
            write(&buffer[..count])?;
            hash.update(&buffer[..count]);
            left -= count as u64;
        }
        if hash.finalize().as_slice() != entry.sha256 {
            return Err("Staged payload changed during archive export".into());
        }
    }
    output
        .write_all(&digest.finalize())
        .map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    active(cancel)?;
    stage.recheck()?;
    fs::hard_link(&temporary, destination).map_err(|e| e.to_string())?;
    let parent = destination
        .parent()
        .ok_or("Archive destination needs a parent folder")?;
    Ok(
        match File::open(parent).and_then(|parent| parent.sync_all()) {
            Ok(()) => SaveOutcome::Durable,
            Err(e) => SaveOutcome::CommittedButDirectorySyncFailed(e.to_string()),
        },
    )
}

/// Inspect an archive manifest without extracting or trusting its payload.
/// Returns validated dependency metadata; import verifies all declared bytes.
pub(crate) fn preview(path: &Path, cancel: &AtomicBool) -> Result<Manifest, String> {
    active(cancel)?;
    let mut file = read_regular(path)?;
    let (manifest, _) = header(&mut file, cancel)?;
    Ok(manifest)
}
fn header(file: &mut File, cancel: &AtomicBool) -> Result<(Manifest, Vec<u8>), String> {
    active(cancel)?;
    let size = file.metadata().map_err(|e| e.to_string())?.len();
    if size > MAX_ARCHIVE_BYTES + MAX_METADATA as u64 + 52 {
        return Err("Portable archive exceeds its byte bound".into());
    }
    let mut header = [0u8; 20];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if &header[..8] != MAGIC || u32::from_le_bytes(header[8..12].try_into().unwrap()) != VERSION {
        return Err("Unsupported portable archive format".into());
    }
    let length = u64::from_le_bytes(header[12..20].try_into().unwrap());
    if length > MAX_METADATA as u64 {
        return Err("Portable manifest exceeds 64 MiB".into());
    }
    let mut metadata = vec![0u8; length as usize];
    file.read_exact(&mut metadata).map_err(|e| e.to_string())?;
    active(cancel)?;
    let manifest: Manifest = serde_json::from_slice(&metadata).map_err(|e| e.to_string())?;
    if 20 + length + manifest.validate()? + 32 != size {
        return Err("Portable archive length differs from its manifest".into());
    }
    let mut bytes = header.to_vec();
    bytes.extend_from_slice(&metadata);
    Ok((manifest, bytes))
}

/// Extract only validated fixed names into a new private staging directory.
/// Every payload and the complete archive are checksum-verified. The caller
/// validates/updates the session before publishing the returned owned stage.
pub(crate) fn extract<T: DeserializeOwned>(
    archive: &Path,
    parent: &Path,
    cancel: &AtomicBool,
) -> Result<(Stage, Manifest, Bundle<T>), String> {
    active(cancel)?;
    let mut input = read_regular(archive)?;
    let before = crate::engine::media_source::FileFingerprint::from_metadata(
        &input.metadata().map_err(|e| e.to_string())?,
    );
    let (manifest, header) = header(&mut input, cancel)?;
    let stage = Stage::new(parent)?;
    let mut total = Sha256::new();
    total.update(&header);
    let mut buffer = [0u8; CHUNK];
    for entry in &manifest.entries {
        let mut output = create(&stage.path.join(&entry.path))?;
        let mut left = entry.bytes;
        let mut hash = Sha256::new();
        while left != 0 {
            active(cancel)?;
            let count = left.min(CHUNK as u64) as usize;
            input
                .read_exact(&mut buffer[..count])
                .map_err(|e| e.to_string())?;
            output
                .write_all(&buffer[..count])
                .map_err(|e| e.to_string())?;
            hash.update(&buffer[..count]);
            total.update(&buffer[..count]);
            left -= count as u64;
        }
        if hash.finalize().as_slice() != entry.sha256 {
            return Err("Portable payload checksum failed".into());
        }
        output.sync_all().map_err(|e| e.to_string())?;
    }
    let mut expected = [0u8; 32];
    input.read_exact(&mut expected).map_err(|e| e.to_string())?;
    if total.finalize().as_slice() != expected
        || crate::engine::media_source::FileFingerprint::from_metadata(
            &input.metadata().map_err(|e| e.to_string())?,
        ) != before
    {
        return Err("Portable archive checksum failed or source changed".into());
    }
    active(cancel)?;
    stage.recheck()?;
    let bundle: Bundle<T> =
        project_file::load(&stage.path.join("session.omat"), &Limits::default(), cancel)
            .map_err(|e| e.to_string())?;
    let mut matched = HashSet::new();
    for sample in &bundle.media {
        let key = Key {
            audio_hash: crate::project_dependencies::audio_hash(sample, cancel)?,
            original_path: sample.path.clone(),
        };
        let dependency = manifest
            .media
            .iter()
            .find(|media| media.key == key)
            .ok_or("Embedded audio differs from portable manifest")?;
        if dependency.frames != sample.frames()
            || dependency.sample_rate != sample.sr
            || dependency.channels != sample.ch
        {
            return Err("Portable media shape or identity differs from its manifest".into());
        }
        matched.insert(key);
    }
    if matched.len() != manifest.media.len() {
        return Err("Portable manifest contains audio absent from its session".into());
    }
    Ok((stage, manifest, bundle))
}

/// Copy one selected source into the owned stage and verify its decoded audio.
/// `remaining` bounds original bytes read; `key` and `pcm_bytes` bind the capture.
/// Returns the content-addressed relative name and bytes read, leaving originals intact.
pub(crate) fn collect(
    stage: &Stage,
    source: &crate::engine::media_source::LibSource,
    key: &Key,
    pcm_bytes: u64,
    remaining: u64,
    cancel: &AtomicBool,
) -> Result<(String, u64), String> {
    use crate::{engine::media_source::FileFingerprint, media_location::Snapshot};
    active(cancel)?;
    stage.recheck()?;
    let snapshot = Snapshot::discover().map_err(|e| e.to_string())?;
    let location = snapshot.resolve(source).map_err(|e| e.to_string())?;
    location
        .recheck_with(&snapshot)
        .map_err(|e| e.to_string())?;
    let access = snapshot.access(&location.path).map_err(|e| e.to_string())?;
    let mut input = read_regular(&location.path)?;
    let before = FileFingerprint::from_metadata(&input.metadata().map_err(|e| e.to_string())?);
    if before.byte_len() > remaining || FileFingerprint::read(&location.path) != Some(before) {
        return Err("Selected sources exceed the 1 GiB read limit or source changed".into());
    }
    let temporary = stage.path.join("source.tmp");
    let mut output = create(&temporary)?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; CHUNK];
    let mut bytes = 0u64;
    loop {
        active(cancel)?;
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        bytes = bytes
            .checked_add(count as u64)
            .filter(|bytes| *bytes <= remaining)
            .ok_or("Selected sources exceed the 1 GiB read limit")?;
        output
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        digest.update(&buffer[..count]);
    }
    output.sync_all().map_err(|e| e.to_string())?;
    let after = Snapshot::discover().map_err(|e| e.to_string())?;
    access
        .check(&after, &location.path)
        .map_err(|e| e.to_string())?;
    location.recheck_with(&after).map_err(|e| e.to_string())?;
    if bytes != before.byte_len()
        || FileFingerprint::from_metadata(&input.metadata().map_err(|e| e.to_string())?) != before
        || FileFingerprint::read(&location.path) != Some(before)
    {
        return Err("Original source changed during collection".into());
    }
    crate::project_dependencies::load_source(
        &crate::engine::media_source::LibSource::File(temporary.clone()),
        key.audio_hash,
        pcm_bytes.saturating_add(4 * 1024 * 1024),
        || cancel.load(Ordering::Acquire),
    )
    .map_err(|e| e.detail)?;
    let name = asset_name(digest.finalize().into());
    if stage.path.join(&name).exists() {
        fs::remove_file(&temporary).map_err(|e| e.to_string())?;
    } else {
        fs::rename(&temporary, stage.path.join(&name)).map_err(|e| e.to_string())?;
    }
    Ok((name, bytes))
}

/// Publish a completed imported directory without replacing any existing path.
/// Cancellation before rename preserves the destination; later sync failure
/// reports a committed warning. Linux renameat2 supplies atomic conflict refusal.
pub(crate) fn publish(
    stage: Stage,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<SaveOutcome, String> {
    publish_directory(stage, destination, cancel, None)
}

/// Publish a complete staged directory into a verified empty real directory.
/// Takes its original device/inode and cancellation flag; refuses changed or
/// nonempty destinations and returns the same durable publication outcome.
pub(crate) fn publish_into_empty(
    stage: Stage,
    destination: &Path,
    identity: (u64, u64),
    cancel: &AtomicBool,
) -> Result<SaveOutcome, String> {
    publish_directory(stage, destination, cancel, Some(identity))
}

fn publish_directory(
    stage: Stage,
    destination: &Path,
    cancel: &AtomicBool,
    empty: Option<(u64, u64)>,
) -> Result<SaveOutcome, String> {
    publish_directory_with(stage, destination, cancel, empty, || {})
}
fn publish_directory_with(
    mut stage: Stage,
    destination: &Path,
    cancel: &AtomicBool,
    empty: Option<(u64, u64)>,
    before_exchange: impl FnOnce(),
) -> Result<SaveOutcome, String> {
    active(cancel)?;
    stage.recheck()?;
    File::open(&stage.path)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())?;
    let old =
        std::ffi::CString::new(stage.path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let new =
        std::ffi::CString::new(destination.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    active(cancel)?;
    if let Some(identity) = empty {
        let metadata = fs::symlink_metadata(destination).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || (metadata.dev(), metadata.ino()) != identity
            || fs::read_dir(destination).map_err(|e| e.to_string())?.next().is_some()
        {
            return Err("Version destination is no longer the reviewed empty directory".into());
        }
    }
    before_exchange();
    if unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            old.as_ptr(),
            libc::AT_FDCWD,
            new.as_ptr(),
            if empty.is_some() { libc::RENAME_EXCHANGE } else { libc::RENAME_NOREPLACE },
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error().to_string());
    }
    if let Some(identity) = empty {
        stage.retained = true;
        let reviewed = fs::symlink_metadata(&stage.path).is_ok_and(|displaced|
            displaced.is_dir() && (displaced.dev(), displaced.ino()) == identity
                && fs::read_dir(&stage.path).is_ok_and(|mut entries| entries.next().is_none()));
        if !reviewed {
            if !fs::symlink_metadata(destination).is_ok_and(|installed|
                installed.is_dir() && (installed.dev(), installed.ino()) == stage.identity) {
                return Err(format!("Version destination changed during exchange; displaced contents retained at {}", stage.path.display()));
            }
            if unsafe { libc::renameat2(libc::AT_FDCWD, old.as_ptr(), libc::AT_FDCWD, new.as_ptr(), libc::RENAME_EXCHANGE) } != 0 {
                return Err(format!("Version exchange could not be restored: {}; displaced contents retained at {}", std::io::Error::last_os_error(), stage.path.display()));
            }
            stage.retained = false;
            return Err("Version destination changed before exchange; original contents restored".into());
        }
        stage.retained = true;
        if let Err(error) = fs::remove_dir(&stage.path) {
            return Ok(SaveOutcome::CommittedButDirectorySyncFailed(format!("Version folder published; empty displaced directory retained at {}: {error}",stage.path.display())));
        }
    }
    stage.retained = true;
    Ok(
        match destination
            .parent()
            .ok_or_else(|| "Import destination needs a parent folder".to_string())
            .and_then(|parent| {
                File::open(parent)
                    .and_then(|file| file.sync_all())
                    .map_err(|e| e.to_string())
            }) {
            Ok(()) => SaveOutcome::Durable,
            Err(e) => SaveOutcome::CommittedButDirectorySyncFailed(e),
        },
    )
}

#[cfg(test)]
mod tests;
