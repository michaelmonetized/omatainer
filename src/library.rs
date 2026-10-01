//! Durable DJ catalog, independent of performance/project documents. All I/O
//! runs on the crate metadata worker. Unknown schemas/fields fail closed.
use crate::engine::{
    media_source::{FileFingerprint, LibSource},
    preparation::Preparation,
};
use crate::ui::bpm::{Bpm, Origin};
use serde::{Deserialize, Serialize};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

mod content;
pub(crate) use content::Relocate;

const SCHEMA: u32 = 4;
const MAX_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TRACKS: usize = 100_000;
const MAX_VERSIONS: usize = 1_000_000;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct TrackId(pub String);
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Metadata {
    pub title: String,
    pub artist: String,
    pub bpm: Bpm,
    pub key: String,
    pub duration: Option<f64>,
    pub last_play: Option<SystemTime>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Version {
    pub fingerprint: Option<FileFingerprint>,
    pub metadata: Metadata,
    pub preparation: Preparation,
    #[serde(default)]
    pub content_hash: Option<[u8; 32]>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Track {
    pub id: TrackId,
    pub source: LibSource,
    pub current: usize,
    // Replaced bytes do not inherit preparation, but their old prepared version
    // remains durable. This is an archive, not an automatic move/content matcher.
    pub versions: Vec<Version>,
    #[serde(default)]
    pub previous_locations: Vec<PreviousLocation>,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreviousLocation {
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    pub schema: u32,
    pub tracks: Vec<Track>,
    #[serde(skip)]
    index: HashMap<LibSource, usize>,
    #[serde(skip)]
    relocations: HashMap<PreviousLocation, usize>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            tracks: vec![],
            index: HashMap::new(),
            relocations: HashMap::new(),
        }
    }
}
impl Catalog {
    pub fn track(&self, source: &LibSource) -> Option<&Track> {
        self.index.get(source).map(|&i| &self.tracks[i])
    }
    pub fn version(
        &self,
        source: &LibSource,
        fingerprint: Option<FileFingerprint>,
    ) -> Option<&Version> {
        self.tracks.get(self.version_track(source, fingerprint)?)?
            .versions
            .iter()
            .find(|version| version.fingerprint == fingerprint)
    }
    fn version_track(&self, source: &LibSource, fingerprint: Option<FileFingerprint>) -> Option<usize> {
        self.index.get(source).copied().filter(|&i| self.tracks[i].versions.iter().any(|v| v.fingerprint == fingerprint))
            .or_else(|| self.relocations.get(&PreviousLocation { source: source.clone(), fingerprint: fingerprint? }).copied())
    }
    pub(crate) fn track_for_version(&self, source: &LibSource, fingerprint: Option<FileFingerprint>) -> Option<&Track> {
        self.tracks.get(self.version_track(source, fingerprint)?)
    }
    /// Resolve only a verified byte-equivalent current version. Stable track
    /// identity alone cannot credit replacement content at a relocated path.
    pub(crate) fn equivalent_current(
        &self,
        source: &LibSource,
        fingerprint: Option<FileFingerprint>,
    ) -> Option<(&LibSource, Option<FileFingerprint>)> {
        let track = self.track_for_version(source, fingerprint)?;
        let old = track.versions.iter().find(|v| v.fingerprint == fingerprint)?;
        let current = track.versions.get(track.current)?;
        (old.content_hash.is_some() && old.content_hash == current.content_hash)
            .then_some((&track.source, current.fingerprint))
    }
    fn validate(&mut self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!("unsupported library schema {}", self.schema));
        }
        if self.tracks.len() > MAX_TRACKS {
            return Err("library exceeds 100000 tracks".into());
        }
        let mut ids = HashSet::new();
        let mut versions = 0usize;
        self.index.clear();
        self.relocations.clear();
        for (i, track) in self.tracks.iter().enumerate() {
            if track.id.0.len() != 32
                || !track
                    .id
                    .0
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
                || !ids.insert(&track.id)
            {
                return Err("invalid or duplicate track identity".into());
            }
            validate_source(&track.source)?;
            if self.index.insert(track.source.clone(), i).is_some() {
                return Err("duplicate library location".into());
            }
            if track.versions.is_empty() || track.current >= track.versions.len() {
                return Err("invalid current media version".into());
            }
            if track.previous_locations.len() > 64 { return Err("track relocation history exceeds 64 locations".into()); }
            for previous in &track.previous_locations {
                validate_source(&previous.source)?;
                if !matches!(&previous.source, LibSource::File(_))
                    || !track.versions.iter().any(|v| v.fingerprint == Some(previous.fingerprint))
                    || self.relocations.insert(previous.clone(), i).is_some() {
                    return Err("invalid or conflicting relocated track association".into());
                }
            }
            let mut fingerprints = HashSet::new();
            versions += track.versions.len();
            if versions > MAX_VERSIONS {
                return Err("library version limit exceeded".into());
            }
            for version in &track.versions {
                let m = &version.metadata;
                if !fingerprints.insert(version.fingerprint)
                    || !version.preparation.valid()
                    || [&m.title, &m.artist, &m.key]
                        .into_iter()
                        .any(|s| s.len() > 4096)
                    || m.duration
                        .is_some_and(|d| !d.is_finite() || d < 0.0 || d > 1.0e10)
                    || m.bpm.value().is_some_and(|v| !v.is_finite() || v <= 1.0)
                    || (m.bpm.origin == Origin::Unknown) != m.bpm.value().is_none()
                {
                    return Err("invalid library metadata or preparation".into());
                }
            }
        }
        for (previous, &owner) in &self.relocations {
            if self.index.get(&previous.source).is_some_and(|&i| i != owner
                && self.tracks[i].versions.iter().any(|v| v.fingerprint == Some(previous.fingerprint))) {
                return Err("relocated content belongs to conflicting track identities".into());
            }
        }
        Ok(())
    }
    pub fn upsert(
        &mut self,
        source: LibSource,
        fingerprint: Option<FileFingerprint>,
        metadata: Metadata,
    ) -> Result<&mut Version, String> {
        let relocated = fingerprint.and_then(|fingerprint| self.relocations.get(&PreviousLocation { source: source.clone(), fingerprint })).copied();
        let i = match relocated.or_else(|| self.index.get(&source).copied()) {
            Some(i) => i,
            None => {
                validate_source(&source)?;
                if self.tracks.len() >= MAX_TRACKS {
                    return Err("library track limit reached".into());
                }
                let mut bytes = [0; 16];
                File::open("/dev/urandom")
                    .and_then(|mut file| file.read_exact(&mut bytes))
                    .map_err(|e| e.to_string())?;
                // Shared built-in stems need global identities so a full
                // catalog import can coexist with a fresh app's default crate.
                let id = match &source {
                    LibSource::Builtin(stem) => {
                        TrackId(format!("{:032x}", stem.index() as u128 + 1))
                    }
                    _ => TrackId(bytes.iter().map(|byte| format!("{byte:02x}")).collect()),
                };
                let i = self.tracks.len();
                self.index.insert(source.clone(), i);
                self.tracks.push(Track {
                    id,
                    source,
                    current: 0,
                    versions: vec![],
                    previous_locations: vec![],
                });
                i
            }
        };
        let track = &mut self.tracks[i];
        let index = if let Some(index) = track
            .versions
            .iter()
            .position(|v| v.fingerprint == fingerprint)
        {
            index
        } else {
            track.versions.push(Version {
                fingerprint,
                metadata: metadata.clone(),
                preparation: Preparation::default(),
                content_hash: None,
            });
            track.versions.len() - 1
        };
        if relocated.is_none() { track.current = index; }
        let version = &mut track.versions[index];
        // Rescanning filename hints cannot erase decoded analysis/user values.
        let old = &version.metadata;
        let bpm = if old.bpm.origin == Origin::User
            || (matches!(metadata.bpm.origin, Origin::Unknown | Origin::FilenameHint)
                && matches!(old.bpm.origin, Origin::Heuristic | Origin::User))
        {
            old.bpm
        } else {
            metadata.bpm
        };
        version.metadata = Metadata {
            title: if old.title.is_empty() {
                metadata.title.clone()
            } else {
                old.title.clone()
            },
            artist: if old.artist.is_empty() {
                metadata.artist.clone()
            } else {
                old.artist.clone()
            },
            key: if old.key.is_empty() {
                metadata.key.clone()
            } else {
                old.key.clone()
            },
            bpm,
            duration: metadata.duration.or(old.duration),
            last_play: metadata.last_play.max(old.last_play),
            ..metadata
        };
        Ok(version)
    }
    pub fn merge_import(&mut self, other: Catalog) -> Result<(), String> {
        // Validate all conflicts before mutation; imports are never partial.
        let mut candidate = self.clone();
        for track in other.tracks {
            if let Some(&index) = candidate.index.get(&track.source) {
                let existing = &mut candidate.tracks[index];
                if existing != &track {
                    let pristine = |t: &Track| {
                        t.versions.len() == 1
                            && t.versions[0].preparation == Preparation::default()
                            && t.versions[0].metadata.last_play.is_none()
                            && t.versions[0].metadata.bpm.origin == Origin::Builtin
                    };
                    if existing.id == track.id
                        && matches!(track.source, LibSource::Builtin(_))
                        && pristine(existing)
                    {
                        *existing = track;
                    } else if existing.id == track.id
                        && matches!(track.source, LibSource::Builtin(_))
                        && pristine(&track)
                    {
                        // Importing factory defaults cannot erase prepared stems.
                    } else {
                        return Err(
                            "import conflicts with an existing track; no data was replaced".into(),
                        );
                    }
                }
            } else if candidate.tracks.iter().any(|t| t.id == track.id) {
                return Err("imported identity belongs to a different location".into());
            } else {
                candidate.tracks.push(track);
            }
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}

pub(crate) fn validate_source(source: &LibSource) -> Result<(), String> {
    let token = |s: &str| !s.trim().is_empty() && s.len() <= 4096 && !s.contains('\0');
    let path = |p: &Path| p.to_str().is_some_and(token);
    let valid = match source {
        LibSource::Builtin(_) => true,
        LibSource::File(p) => p.is_absolute() && path(p),
        LibSource::Removable {
            volume_id,
            relative_path,
        } => {
            token(volume_id)
                && path(relative_path)
                && !relative_path.is_absolute()
                && relative_path
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
        }
        LibSource::Provider { provider, media_id } => token(provider) && token(media_id),
    };
    valid
        .then_some(())
        .ok_or_else(|| "invalid or unsupported library location".into())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct V1 {
    schema: u32,
    tracks: Vec<V1Track>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct V1Track {
    id: TrackId,
    source: LibSource,
    fingerprint: Option<FileFingerprint>,
    metadata: Metadata,
    preparation: Preparation,
}

pub(crate) fn read(path: &Path) -> Result<Catalog, String> {
    read_with_identity(path).map(|(catalog, _)| catalog)
}

fn read_with_identity(path: &Path) -> Result<(Catalog, FileFingerprint), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    let meta = file.metadata().map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return Err("library must be a regular file of at most 64 MiB".into());
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("library exceeds 64 MiB".into());
    }
    let header: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut catalog = match header.get("schema").and_then(|v| v.as_u64()) {
        Some(2 | 3 | 4) => {
            let mut current: Catalog = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            current.schema = SCHEMA;
            current
        },
        Some(1) => {
            let old: V1 = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            debug_assert_eq!(old.schema, 1);
            Catalog {
                schema: SCHEMA,
                index: HashMap::new(),
            relocations: HashMap::new(),
                tracks: old
                    .tracks
                    .into_iter()
                    .map(|track| Track {
                        id: track.id,
                        source: track.source,
                        current: 0,
                        previous_locations: vec![],
                        versions: vec![Version {
                            fingerprint: track.fingerprint,
                            metadata: track.metadata,
                            preparation: track.preparation,
                            content_hash: None,
                        }],
                    })
                    .collect(),
            }
        }
        version => {
            return Err(format!(
                "unsupported library schema {version:?}; original file preserved"
            ))
        }
    };
    catalog.validate()?;
    let identity = FileFingerprint::from_metadata(&meta);
    if FileFingerprint::from_metadata(&file.metadata().map_err(|e| e.to_string())?) != identity
        || store_identity(path)? != Some(identity)
    {
        return Err("DJ library changed while it was read; original file preserved".into());
    }
    Ok((catalog, identity))
}

fn store_identity(path: &Path) -> Result<Option<FileFingerprint>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(FileFingerprint::from_metadata(&metadata))),
        Ok(_) => Err("DJ library path is not a regular file; original path preserved".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) struct Store {
    path: PathBuf,
    _lock: File,
    identity: Option<FileFingerprint>,
    pub catalog: Catalog,
    saved: Vec<u8>,
}
impl Drop for Store {
    fn drop(&mut self) {
        // Closing only this descriptor can leave an inherited fork/dup handle
        // holding the same flock after this writer has finished.
        let _ = self._lock.unlock();
    }
}
impl Store {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let parent = path.parent().ok_or("library has no parent directory")?;
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path.with_extension("lock"))
            .map_err(|e| e.to_string())?;
        lock.try_lock()
            .map_err(|_| "DJ library is already open by another writer".to_string())?;
        // A malformed/newer file is never treated as an empty library. Only
        // genuinely absent stores are initialized. Backups are explicit recovery.
        let (catalog, identity) = match fs::symlink_metadata(&path) {
            Ok(_) => read_with_identity(&path).map(|(catalog, identity)| (catalog, Some(identity)))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if path
                    .with_extension("backup.json")
                    .symlink_metadata()
                    .is_ok()
                {
                    return Err("primary DJ library is missing; backup.json was preserved. Restore the verified backup and restart before saving".into());
                }
                (Catalog::default(), None)
            }
            Err(e) => return Err(e.to_string()),
        };
        Ok(Self {
            identity,
            path,
            _lock: lock,
            catalog,
            saved: vec![],
        })
    }
    pub fn save(&mut self) -> Result<(), String> {
        self.save_with(|_| Ok(()))
    }
    fn save_with(
        &mut self,
        mut checkpoint: impl FnMut(u8) -> Result<(), String>,
    ) -> Result<(), String> {
        self.catalog.validate()?;
        let bytes = serde_json::to_vec(&self.catalog).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("DJ library exceeds 64 MiB; existing store preserved".into());
        }
        if store_identity(&self.path)? != self.identity
            || (self.identity.is_none() && self.path.symlink_metadata().is_ok())
        {
            return Err("DJ library changed outside this process; original file preserved; restart to reload".into());
        }
        if bytes == self.saved {
            return Ok(());
        }
        let temp = self
            .path
            .with_extension(format!("tmp-{}", std::process::id()));
        // A crashed write's temporary file is never considered committed data.
        // Refuse leftovers rather than deleting an unknown file automatically.
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|e| format!("temporary library write: {e}"))?;
        let temporary_owned = file.metadata().map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            checkpoint(0)?;
            file.sync_all().map_err(|e| e.to_string())?;
            checkpoint(1)?;
            let written = file.metadata().map_err(|e| e.to_string())?;
            if store_identity(&self.path)? != self.identity {
                return Err("DJ library changed during save; original file preserved".into());
            }
            if self.identity.is_some() {
                let original = OpenOptions::new().read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(&self.path).map_err(|e| e.to_string())?;
                let before = original.metadata().map_err(|e| e.to_string())?;
                if Some(FileFingerprint::from_metadata(&before)) != self.identity {
                    return Err("DJ library changed before backup; original file preserved".into());
                }
                let backup_temp = self
                    .path
                    .with_extension(format!("backup-{}", std::process::id()));
                fs::hard_link(&self.path, &backup_temp).map_err(|e| e.to_string())?;
                let owned = fs::symlink_metadata(&backup_temp).map_err(|e| e.to_string())?;
                let renamed = (|| {
                    checkpoint(4)?;
                    let current = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
                    if !same_content_identity(&before, &owned) || !same_content_identity(&before, &current) {
                        return Err("DJ library changed during backup; external bytes preserved".into());
                    }
                    fs::rename(&backup_temp, self.path.with_extension("backup.json"))
                        .map_err(|e| e.to_string())
                })();
                // POSIX rename is a successful no-op when both paths already
                // link the same inode (e.g. retry after checkpoint 2). Retire
                // only the temporary link we created, including that case.
                let cleanup = remove_owned(&backup_temp, &owned);
                let refresh = (|| {
                    let current = fs::symlink_metadata(&self.path).map_err(|e| e.to_string())?;
                    let still_owned = original.metadata().map_err(|e| e.to_string())?;
                    if !same_content_identity(&before, &current)
                        || !same_content_identity(&before, &still_owned)
                        || FileFingerprint::from_metadata(&current) != FileFingerprint::from_metadata(&still_owned)
                    {
                        return Err("DJ library changed during backup; external bytes preserved".into());
                    }
                    // Only our original descriptor's expected link-related
                    // ctime change is accepted. Never adopt a replacement path.
                    self.identity = Some(FileFingerprint::from_metadata(&still_owned));
                    Ok::<(), String>(())
                })();
                renamed?;
                cleanup?;
                refresh?;
            }
            checkpoint(2)?;
            if store_identity(&self.path)? != self.identity {
                return Err("DJ library changed before commit; external bytes preserved".into());
            }
            let pending = fs::symlink_metadata(&temp).map_err(|e| e.to_string())?;
            if !same_content_identity(&written, &pending) {
                return Err("temporary DJ library changed before commit; preserved".into());
            }
            fs::rename(&temp, &self.path).map_err(|e| e.to_string())?;
            let committed = file.metadata().map_err(|e| format!("replacement committed; identity unavailable: {e}"))?;
            if !same_content_identity(&written, &committed) {
                return Err("replacement committed but externally modified; durability unconfirmed".into());
            }
            self.identity = Some(FileFingerprint::from_metadata(&committed));
            checkpoint(3).map_err(|error| {
                format!("replacement committed; durability unconfirmed: {error}")
            })?;
            if store_identity(&self.path).map_err(|error| format!("replacement committed; identity recheck failed: {error}"))? != self.identity {
                return Err("replacement committed but changed externally; original external file preserved".into());
            }
            File::open(self.path.parent().unwrap())
                .and_then(|dir| dir.sync_all())
                .map_err(|e| {
                    format!(
                        "replacement committed; directory sync failed; durability unconfirmed: {e}"
                    )
                })?;
            self.saved = bytes;
            Ok(())
        })();
        let cleanup = remove_owned(&temp, &temporary_owned);
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) => Err(error),
            (Ok(()), Err(error)) => Err(format!("replacement committed; temporary cleanup: {error}")),
            (Err(error), Err(cleanup)) => Err(format!("{error}; temporary cleanup: {cleanup}")),
        }
    }
}

fn same_content_identity(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.is_file() && b.is_file() && a.dev() == b.dev() && a.ino() == b.ino()
        && a.len() == b.len() && a.mtime() == b.mtime() && a.mtime_nsec() == b.mtime_nsec()
        && a.mode() == b.mode() && a.uid() == b.uid() && a.gid() == b.gid()
}

fn remove_owned(path: &Path, owned: &fs::Metadata) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(current) if current.dev() == owned.dev() && current.ino() == owned.ino() =>
            fs::remove_file(path).map_err(|e| e.to_string()),
        Ok(_) => Err("temporary DJ library path changed; preserved".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

pub(crate) fn default_path() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("omatainer/library.json")
}
#[cfg(test)]
mod tests;
