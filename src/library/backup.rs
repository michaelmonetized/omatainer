//! Complete catalog snapshots and verified local copies, owned by one worker.
use super::*;
use crate::{
    engine::performance::WorkPermit, media_location::Snapshot, portable_project::Stage,
    project_file::SaveOutcome,
};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicBool, Ordering};

const VERSION: u32 = 1;
const MAX_FILE: u64 = 8 * 1024 * 1024 * 1024;
pub(crate) const MAX_COLLECTION: u64 = 1024 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    track: TrackId,
    fingerprint: FileFingerprint,
    asset: String,
    sha256: [u8; 32],
    bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u32,
    library_schema: u32,
    catalog_sha256: [u8; 32],
    collection_limit: u64,
    entries: Vec<Entry>,
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Progress {
    pub files: usize,
    pub total: usize,
    pub bytes: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct Summary {
    pub tracks: usize,
    pub crates: usize,
    pub collected: usize,
    pub bytes: u64,
}
pub(crate) struct Published {
    pub catalog: PathBuf,
    pub summary: Summary,
    pub outcome: SaveOutcome,
}
struct Verified {
    catalog: Catalog,
    manifest: Manifest,
    summary: Summary,
    guards: Vec<(PathBuf, FileFingerprint)>,
    root: Directory,
    media: Directory,
}
struct Directory {
    path: PathBuf,
    identity: (u64, u64),
    access: crate::media_location::Access,
}
impl Directory {
    /// Capture a real directory and its mount view.
    /// `path` must be absolute; returns guards without following its final symlink.
    fn capture(path: &Path) -> Result<Self, String> {
        validate_source(&LibSource::File(path.into()))?;
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_dir() {
            return Err("Library backup path must be a real directory".into());
        }
        let access = Snapshot::discover()
            .and_then(|s| s.access(path))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            path: path.into(),
            identity: (metadata.dev(), metadata.ino()),
            access,
        })
    }
    /// Recheck the captured directory and mount.
    /// Returns an error if `self` no longer names the same directory.
    fn check(&self) -> Result<(), String> {
        let metadata = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
        let snapshot = Snapshot::discover().map_err(|e| e.to_string())?;
        self.access
            .check(&snapshot, &self.path)
            .map_err(|e| e.to_string())?;
        if !metadata.is_dir() || (metadata.dev(), metadata.ino()) != self.identity {
            return Err("Library backup directory changed during access".into());
        }
        Ok(())
    }
}

/// Stop a backup before publication when its owner cancels.
/// `cancel` is the shared user/performance flag; returns permission to continue.
fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Library backup cancelled".into())
    } else {
        Ok(())
    }
}
/// Create a private regular output without replacing a pathname.
/// `path` belongs to our stage; returns the new descriptor.
fn create(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| e.to_string())
}
/// Open an unchanged bounded regular source.
/// `path`, `expected` and `limit` bind the read; returns one verified descriptor.
fn regular(path: &Path, expected: FileFingerprint, limit: u64) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.len() > limit
        || FileFingerprint::from_metadata(&metadata) != expected
        || !same_file(path, expected)
    {
        return Err("Library backup source changed or exceeds its file limit".into());
    }
    Ok(file)
}
/// Compare a captured regular file without following its final symlink.
/// Takes `path` and `expected`; returns false for replacement, removal or links.
fn same_file(path: &Path, expected: FileFingerprint) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_file() && FileFingerprint::from_metadata(&metadata) == expected
    })
}
/// Recheck a descriptor and its original pathname after reading.
/// Takes the captured `expected`; refuses replacement or in-place changes.
fn unchanged(file: &File, path: &Path, expected: FileFingerprint) -> Result<(), String> {
    if FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?) != expected
        || !same_file(path, expected)
    {
        return Err("Library backup source changed during reading".into());
    }
    Ok(())
}
/// Copy exact encoded bytes with bounded reads and a measured digest.
/// `progress` receives read counts; returns SHA-256 only after both identities agree.
fn copy(
    path: &Path,
    expected: FileFingerprint,
    destination: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64),
) -> Result<[u8; 32], String> {
    let snapshot = Snapshot::discover().map_err(|e| e.to_string())?;
    let access = snapshot.access(path).map_err(|e| e.to_string())?;
    let mut input = regular(path, expected, MAX_FILE)?;
    let mut output = create(destination)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut hash = Sha256::new();
    let mut total = 0u64;
    loop {
        active(cancel)?;
        let count = input.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or("Library copy length overflow")?;
        if total > expected.byte_len() {
            return Err("Library source grew during copy".into());
        }
        output
            .write_all(&buffer[..count])
            .map_err(|e| e.to_string())?;
        hash.update(&buffer[..count]);
        progress(count as u64);
    }
    if total != expected.byte_len() {
        return Err("Library source was truncated during copy".into());
    }
    unchanged(&input, path, expected)?;
    access
        .check(&Snapshot::discover().map_err(|e| e.to_string())?, path)
        .map_err(|e| e.to_string())?;
    output.sync_all().map_err(|e| e.to_string())?;
    let output_identity =
        FileFingerprint::from_metadata(&output.metadata().map_err(|e| e.to_string())?);
    unchanged(&output, destination, output_identity)?;
    active(cancel)?;
    Ok(hash.finalize().into())
}
/// Read bounded snapshot metadata without accepting a changing source.
/// Returns the captured bytes and fingerprint for `path`.
fn metadata(path: &Path, cancel: &AtomicBool) -> Result<(Vec<u8>, FileFingerprint), String> {
    active(cancel)?;
    let expected = FileFingerprint::read(path).ok_or("Library backup metadata is missing")?;
    let mut file = regular(path, expected, MAX_BYTES)?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        active(cancel)?;
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if bytes.len() as u64 + count as u64 > MAX_BYTES {
            return Err("Library backup metadata exceeds 64 MiB".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    unchanged(&file, path, expected)?;
    Ok((bytes, expected))
}
/// Write bounded snapshot metadata into a private stage.
/// `value` must already validate; returns SHA-256 of the exact saved JSON.
fn write_json(path: &Path, value: &impl Serialize) -> Result<[u8; 32], String> {
    struct Output {
        file: File,
        hash: Sha256,
        bytes: u64,
    }
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.bytes.saturating_add(bytes.len() as u64) > MAX_BYTES {
                return Err(std::io::Error::other(
                    "Library backup metadata exceeds 64 MiB",
                ));
            }
            let count = self.file.write(bytes)?;
            self.hash.update(&bytes[..count]);
            self.bytes += count as u64;
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.file.flush()
        }
    }
    let mut output = Output {
        file: create(path)?,
        hash: Sha256::new(),
        bytes: 0,
    };
    serde_json::to_writer(&mut output, value).map_err(|e| e.to_string())?;
    output.file.sync_all().map_err(|e| e.to_string())?;
    let identity =
        FileFingerprint::from_metadata(&output.file.metadata().map_err(|e| e.to_string())?);
    unchanged(&output.file, path, identity)?;
    Ok(output.hash.finalize().into())
}
/// Accept native local audio suffixes for copied assets.
/// Returns a lowercase suffix for `path`, or rejects unsupported names.
fn extension(path: &Path) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ["wav", "mp3", "flac", "ogg", "aif", "aiff", "m4a", "aac"].contains(&extension.as_str()) {
        Ok(extension)
    } else {
        Err("Library collection accepts WAV, MP3, FLAC, Ogg, AIFF, M4A and AAC files".into())
    }
}
/// Encode a digest as an opaque filename stem.
/// `hash` contains measured bytes; returns exactly 64 lowercase hex digits.
fn digest_name(hash: [u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}
/// Validate an opaque asset basename against its measured digest.
/// `entry` cannot introduce a directory, traversal or unrelated filename.
fn asset(entry: &Entry) -> Result<(), String> {
    let suffix = extension(Path::new(&entry.asset))?;
    if entry.asset != format!("{}.{}", digest_name(entry.sha256), suffix)
        || entry.bytes > MAX_FILE
        || entry.fingerprint.byte_len() != entry.bytes
    {
        return Err("Invalid library backup asset name or length".into());
    }
    Ok(())
}
/// Publish a complete private directory under performance admission.
/// `work` orders mode entry; returns the actual durable/committed outcome.
fn publish(
    stage: Stage,
    destination: &Path,
    cancel: &AtomicBool,
    work: Option<&WorkPermit>,
) -> Result<SaveOutcome, String> {
    let _commit = work
        .map(|work| work.commit().map_err(|e| e.to_string()))
        .transpose()?;
    crate::portable_project::publish(stage, destination, cancel)
}
/// Refuse an existing destination before starting expensive work.
/// `path` must be an absolute native location; final publication rechecks it atomically.
fn new_destination(path: &Path) -> Result<(), String> {
    validate_source(&LibSource::File(path.into()))?;
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
        Ok(_) => Err("Library destination already exists; choose a new directory".into()),
    }
}
/// Export one immutable catalog and optionally every local current version.
/// `collect_limit` grants and bounds copying; `destination` must not exist.
/// Returns its published catalog path, counts and durability outcome.
pub(crate) fn export(
    snapshot: &Catalog,
    destination: &Path,
    collect_limit: Option<u64>,
    cancel: &AtomicBool,
    work: Option<&WorkPermit>,
    mut progress: impl FnMut(Progress),
) -> Result<Published, String> {
    active(cancel)?;
    new_destination(destination)?;
    if collect_limit.is_some_and(|limit| limit == 0 || limit > MAX_COLLECTION) {
        return Err("Library collection limit must be between 1 byte and 1 TiB".into());
    }
    let mut catalog = snapshot.clone();
    catalog.validate()?;
    let parent = Directory::capture(
        destination
            .parent()
            .ok_or("Backup destination has no parent")?,
    )?;
    let stage = Stage::new(&parent.path)?;
    let media = stage.path.join("media");
    fs::create_dir(&media).map_err(|e| e.to_string())?;
    let selected = if collect_limit.is_some() {
        catalog
            .tracks
            .iter()
            .filter(|t| matches!(t.source, LibSource::File(_) | LibSource::Removable { .. }))
            .count()
    } else {
        0
    };
    let mut state = Progress {
        total: selected,
        ..Default::default()
    };
    progress(state);
    let mut written = Vec::new();
    let mut entries = Vec::new();
    let mut guards = Vec::new();
    let mut assets: HashMap<[u8; 32], String> = HashMap::new();
    let mut planned = 0u64;
    let mut unique_bytes = 0u64;
    if let Some(limit) = collect_limit {
        let snapshot = Snapshot::discover().map_err(|e| e.to_string())?;
        for track in &mut catalog.tracks {
            active(cancel)?;
            if !matches!(
                track.source,
                LibSource::File(_) | LibSource::Removable { .. }
            ) {
                continue;
            }
            let fingerprint = track.versions[track.current]
                .fingerprint
                .ok_or("Cannot collect a local track without a captured source identity")?;
            planned = planned
                .checked_add(fingerprint.byte_len())
                .ok_or("Library collection length overflow")?;
            if fingerprint.byte_len() > MAX_FILE || planned > limit {
                return Err("Local library exceeds the authorized collection limit".into());
            }
            let location = snapshot.resolve(&track.source).map_err(|e| e.to_string())?;
            let suffix = extension(&location.path)?;
            let temporary = media.join(format!("{}.part", track.id.0));
            let hash = copy(&location.path, fingerprint, &temporary, cancel, |count| {
                state.bytes += count;
                progress(state);
            })?;
            location.recheck().map_err(|e| e.to_string())?;
            let version = &mut track.versions[track.current];
            if version.content_hash.is_some_and(|old| old != hash) {
                return Err("Collected bytes conflict with the saved content proof".into());
            }
            version.content_hash = Some(hash);
            let name = if let Some(name) = assets.get(&hash) {
                fs::remove_file(&temporary).map_err(|e| e.to_string())?;
                name.clone()
            } else {
                let name = format!("{}.{}", digest_name(hash), suffix);
                let output = media.join(&name);
                fs::rename(&temporary, &output).map_err(|e| e.to_string())?;
                written.push((
                    output.clone(),
                    FileFingerprint::read(&output).ok_or("Copied backup asset is unavailable")?,
                ));
                unique_bytes += fingerprint.byte_len();
                assets.insert(hash, name.clone());
                name
            };
            let access = snapshot.access(&location.path).map_err(|e| e.to_string())?;
            guards.push((location, access, fingerprint));
            entries.push(Entry {
                track: track.id.clone(),
                fingerprint,
                asset: name,
                sha256: hash,
                bytes: fingerprint.byte_len(),
            });
            state.files += 1;
            progress(state);
        }
    }
    catalog.validate()?;
    let catalog_sha256 = write_json(&stage.path.join("catalog.json"), &catalog)?;
    let manifest = Manifest {
        schema: VERSION,
        library_schema: SCHEMA,
        catalog_sha256,
        collection_limit: collect_limit.unwrap_or(0),
        entries,
    };
    write_json(&stage.path.join("manifest.json"), &manifest)?;
    for name in ["catalog.json", "manifest.json"] {
        let path = stage.path.join(name);
        written.push((
            path.clone(),
            FileFingerprint::read(&path).ok_or("Written backup metadata is unavailable")?,
        ));
    }
    File::open(&media)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())?;
    let fresh = Snapshot::discover().map_err(|e| e.to_string())?;
    for (location, access, expected) in guards {
        active(cancel)?;
        access
            .check(&fresh, &location.path)
            .map_err(|e| e.to_string())?;
        location.recheck_with(&fresh).map_err(|e| e.to_string())?;
        if !same_file(&location.path, expected) {
            return Err("Library source changed before backup publication".into());
        }
    }
    for (path, fingerprint) in written {
        active(cancel)?;
        if !same_file(&path, fingerprint) {
            return Err("Written backup changed before publication".into());
        }
    }
    parent.check()?;
    let summary = Summary {
        tracks: catalog.tracks.len(),
        crates: catalog.crates.nodes().len(),
        collected: manifest.entries.len(),
        bytes: unique_bytes,
    };
    let outcome = publish(stage, destination, cancel, work)?;
    Ok(Published {
        catalog: destination.join("catalog.json"),
        summary,
        outcome,
    })
}

/// Verify all metadata, identities and assets in an existing backup.
/// Returns a private candidate and recheck guards; never edits the backup.
fn verify(
    path: &Path,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<Verified, String> {
    let root = Directory::capture(path)?;
    let media = Directory::capture(&path.join("media"))?;
    let (bytes, manifest_fp) = metadata(&path.join("manifest.json"), cancel)?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if manifest.schema != VERSION
        || !(12..=SCHEMA).contains(&manifest.library_schema)
        || manifest.entries.len() > MAX_TRACKS
        || manifest.collection_limit > MAX_COLLECTION
        || (!manifest.entries.is_empty() && manifest.collection_limit == 0)
    {
        return Err("Unsupported or oversized library backup manifest".into());
    }
    let (bytes, catalog_fp) = metadata(&path.join("catalog.json"), cancel)?;
    if <[u8; 32]>::from(Sha256::digest(&bytes)) != manifest.catalog_sha256 {
        return Err("Library backup catalog checksum does not match".into());
    }
    let mut catalog = super::catalog_from_bytes(&bytes, Some(manifest.library_schema))?;
    let tracks: HashMap<_, _> = catalog
        .tracks
        .iter()
        .map(|track| (&track.id, track))
        .collect();
    let mut ids = HashSet::new();
    let mut assets = HashMap::new();
    let mut guards = vec![
        (path.join("manifest.json"), manifest_fp),
        (path.join("catalog.json"), catalog_fp),
    ];
    let mut state = Progress {
        total: manifest.entries.len(),
        ..Default::default()
    };
    let mut logical_bytes = 0u64;
    let mut unique_bytes = 0u64;
    for entry in &manifest.entries {
        active(cancel)?;
        asset(entry)?;
        let track = tracks
            .get(&entry.track)
            .ok_or("Backup asset refers to an unknown track")?;
        let version = &track.versions[track.current];
        if !ids.insert(&entry.track)
            || !matches!(
                track.source,
                LibSource::File(_) | LibSource::Removable { .. }
            )
            || version.fingerprint != Some(entry.fingerprint)
            || version.content_hash != Some(entry.sha256)
        {
            return Err("Backup asset conflicts with its captured catalog identity".into());
        }
        logical_bytes = logical_bytes
            .checked_add(entry.bytes)
            .ok_or("Backup collection length overflow")?;
        if logical_bytes > manifest.collection_limit {
            return Err("Backup collection exceeds its recorded limit".into());
        }
        if let Some((hash, bytes)) = assets.get(&entry.asset) {
            if (*hash, *bytes) != (entry.sha256, entry.bytes) {
                return Err("Conflicting duplicate backup asset".into());
            }
        } else {
            let path = media.path.join(&entry.asset);
            let fingerprint =
                FileFingerprint::read(&path).ok_or("Backup media asset is missing")?;
            if fingerprint.byte_len() != entry.bytes {
                return Err("Backup asset length does not match".into());
            }
            let hash = content::hash_file_progress(
                &path,
                fingerprint,
                || !cancel.load(Ordering::Acquire),
                |count| {
                    state.bytes += count;
                    progress(state);
                },
            )?;
            if hash != entry.sha256 {
                return Err("Backup asset checksum does not match".into());
            }
            unique_bytes += entry.bytes;
            guards.push((path, fingerprint));
            assets.insert(entry.asset.clone(), (entry.sha256, entry.bytes));
        }
        state.files += 1;
        progress(state);
    }
    let allowed: HashSet<_> = ["manifest.json", "catalog.json", "media"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    check_contents(&root.path, &allowed)?;
    check_contents(&media.path, &assets.keys().cloned().collect())?;
    root.check()?;
    media.check()?;
    let summary = Summary {
        tracks: catalog.tracks.len(),
        crates: catalog.crates.nodes().len(),
        collected: manifest.entries.len(),
        bytes: unique_bytes,
    };
    Ok(Verified {
        catalog,
        manifest,
        summary,
        guards,
        root,
        media,
    })
}
/// Reject undeclared files and symlinks in a snapshot directory.
/// `allowed` contains exact basenames; returns no partial acceptance.
fn check_contents(path: &Path, allowed: &HashSet<String>) -> Result<(), String> {
    let mut seen = HashSet::new();
    for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "Invalid backup filename")?;
        if !allowed.contains(&name)
            || !seen.insert(name)
            || entry.file_type().map_err(|e| e.to_string())?.is_symlink()
        {
            return Err("Library backup contains an undeclared file or symlink".into());
        }
    }
    if seen != *allowed {
        return Err("Library backup is incomplete".into());
    }
    Ok(())
}
/// Inspect a complete backup without publishing or activating it.
/// `path` is read-only; returns verified catalog and media counts.
pub(crate) fn inspect(
    path: &Path,
    cancel: &AtomicBool,
    progress: impl FnMut(Progress),
) -> Result<Summary, String> {
    let verified = verify(path, cancel, progress)?;
    recheck(&verified, cancel)?;
    Ok(verified.summary)
}
/// Recheck every captured snapshot path immediately before publication.
/// Takes `verified` and cancellation; refuses a changing backup.
fn recheck(verified: &Verified, cancel: &AtomicBool) -> Result<(), String> {
    verified.root.check()?;
    verified.media.check()?;
    for (path, fingerprint) in &verified.guards {
        active(cancel)?;
        if !same_file(path, *fingerprint) {
            return Err("Library backup changed after verification".into());
        }
    }
    Ok(())
}
/// Restore a verified snapshot into a new independent directory.
/// `destination` is never overwritten; copied media gets distinct track paths.
/// Returns a catalog for explicit import, retaining preparation and old identities.
pub(crate) fn restore(
    path: &Path,
    destination: &Path,
    cancel: &AtomicBool,
    work: Option<&WorkPermit>,
    mut progress: impl FnMut(Progress),
) -> Result<Published, String> {
    new_destination(destination)?;
    if destination.starts_with(path) {
        return Err("Restore destination must be outside the read-only backup".into());
    }
    let mut verified = verify(path, cancel, &mut progress)?;
    let parent = Directory::capture(
        destination
            .parent()
            .ok_or("Restore destination has no parent")?,
    )?;
    let stage = Stage::new(&parent.path)?;
    let media = stage.path.join("media");
    fs::create_dir(&media).map_err(|e| e.to_string())?;
    let indexes: HashMap<_, _> = verified
        .catalog
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.id.clone(), i))
        .collect();
    let mut written = Vec::new();
    let fingerprints: HashMap<_, _> = verified
        .guards
        .iter()
        .map(|(path, fp)| (path.clone(), *fp))
        .collect();
    let mut state = Progress {
        total: verified.manifest.entries.len(),
        ..Default::default()
    };
    for entry in &verified.manifest.entries {
        active(cancel)?;
        let original = verified.media.path.join(&entry.asset);
        let expected = fingerprints
            .get(&original)
            .copied()
            .ok_or("Verified backup asset is missing")?;
        let name = format!("{}.{}", entry.track.0, extension(Path::new(&entry.asset))?);
        let copied = media.join(&name);
        let hash = copy(&original, expected, &copied, cancel, |count| {
            state.bytes += count;
            progress(state);
        })?;
        if hash != entry.sha256 {
            return Err("Restored asset checksum does not match".into());
        }
        let fingerprint = FileFingerprint::read(&copied).ok_or("Restored file is unavailable")?;
        written.push((copied, fingerprint));
        let track = &mut verified.catalog.tracks[indexes[&entry.track]];
        let previous = PreviousLocation {
            source: track.source.clone(),
            fingerprint: entry.fingerprint,
        };
        if !track.previous_locations.contains(&previous) {
            if track.previous_locations.len() >= 64 {
                return Err("Track relocation history is full (64 locations)".into());
            }
            track.previous_locations.push(previous);
        }
        let mut version = track.versions[track.current].clone();
        version.fingerprint = Some(fingerprint);
        if let Some(index) = track
            .versions
            .iter()
            .position(|v| v.fingerprint == Some(fingerprint))
        {
            track.versions[index] = version;
            track.current = index;
        } else {
            track.versions.push(version);
            track.current = track.versions.len() - 1;
        }
        track.source = LibSource::File(destination.join("media").join(name));
        state.files += 1;
        progress(state);
    }
    verified.catalog.validate()?;
    write_json(&stage.path.join("library.json"), &verified.catalog)?;
    File::open(&media)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| e.to_string())?;
    for (path, fingerprint) in written {
        active(cancel)?;
        if !same_file(&path, fingerprint) {
            return Err("Restored media changed before publication".into());
        }
    }
    recheck(&verified, cancel)?;
    parent.check()?;
    let outcome = publish(stage, destination, cancel, work)?;
    Ok(Published {
        catalog: destination.join("library.json"),
        summary: verified.summary,
        outcome,
    })
}

#[cfg(test)]
mod tests;
