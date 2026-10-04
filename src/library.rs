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
pub(crate) mod backup;
pub(crate) mod tags;
pub(crate) mod annotations;
pub(crate) mod search;
pub(crate) mod smart_crates;
pub(crate) mod protection;
pub(crate) mod relocation_search;
mod analysis;
pub(crate) mod crates;
pub(crate) mod watch_roots;
mod collections;
pub(crate) use content::Relocate;
/// Verify the encoded bytes of a project source against its captured file.
/// `path` and `expected` bind identity; `active` stops worker reads.
/// Returns SHA-256, or refuses a changed, unavailable or oversized file.
pub(crate) fn hash_project_source(path: &Path, expected: FileFingerprint, active: impl FnMut() -> bool) -> Result<[u8; 32], String> {
    content::hash_file(path, expected, active)
}

const SCHEMA: u32 = 12;
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
    #[serde(default)]
    pub analysis: Option<crate::track_analysis::Record>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<tags::TagMetadata>,
    /// Set only by a verified within-track audio-preserving tag transaction,
    /// or by subsequently verifying an exact byte copy of such a version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_identity: Option<crate::media_tags::payload::Identity>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Track {
    pub id: TrackId,
    #[serde(default, skip_serializing_if = "annotations::Annotations::is_empty")]
    pub annotations: annotations::Annotations,
    #[serde(default, skip_serializing_if = "protection::Locks::is_empty")]
    pub locks: protection::Locks,
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
    pub crates: crates::CrateForest<TrackId>,
    pub watched_roots: watch_roots::Book,
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
            crates: Default::default(),
            watched_roots: Default::default(),
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
        tags::equivalent_audio(old, current)
            .then_some((&track.source, current.fingerprint))
    }
    /// Preserve the selected current version after an archived capture/scan.
    /// The caller has already checked the current I/O identity off the GUI.
    pub(crate) fn restore_current(&mut self,source:&LibSource,current:usize) {
        if let Some(&i)=self.index.get(source) {if current<self.tracks[i].versions.len() {self.tracks[i].current=current;}}
    }
    /// A freshly measured descriptor hash may restore preparation from a
    /// version of this exact track whose saved digest matches. No filesystem
    /// reads or pathname/title association occur here.
    pub(crate) fn preparation_for_content(&self,source:&LibSource,fingerprint:FileFingerprint,hash:[u8;32])->Option<Preparation> {
        let track=self.track(source)?;
        if let Some(v)=track.versions.iter().find(|v|v.fingerprint==Some(fingerprint)) {
            if v.content_hash.is_some_and(|saved|saved!=hash) {return None;}
            if v.content_hash==Some(hash) || v.preparation!=Preparation::default() {return Some(v.preparation);}
        }
        track.versions.iter().rev().find(|v|v.content_hash==Some(hash)).map(|v|v.preparation)
    }
    fn validate(&mut self) -> Result<(), String> {
        if self.schema != SCHEMA {
            return Err(format!("unsupported library schema {}", self.schema));
        }
        self.watched_roots.validate()?;
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
            track.annotations.validate()?;
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
                if !matches!(&previous.source, LibSource::File(_) | LibSource::Removable {..})
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
                    || version.analysis.as_ref().is_some_and(|record|
                        !record.valid() || version.fingerprint.is_none() || version.content_hash.is_none())
                    || [&m.title, &m.artist, &m.key]
                        .into_iter()
                        .any(|s| s.len() > 4096)
                    || m.duration
                        .is_some_and(|d| !d.is_finite() || d < 0.0 || d > 1.0e10)
                    || !m.bpm.valid()
                    || version.tags.as_ref().is_some_and(|tags| !tags.valid())
                    || version.audio_identity.as_ref().is_some_and(|identity|
                        !identity.valid() || version.fingerprint.is_none() || version.content_hash.is_none())
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
        self.crates.validate(|id| ids.contains(id)).map_err(|error| error.to_string())?;
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
                    annotations: Default::default(),
                    locks: Default::default(),
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
                analysis: None,
                tags: None,
                audio_identity: None,
            });
            track.versions.len() - 1
        };
        let archived_tag_receipt = index != track.current
            && track.versions[index].audio_identity.is_some()
            && tags::equivalent_audio(&track.versions[index], &track.versions[track.current]);
        if relocated.is_none() && !archived_tag_receipt { track.current = index; }
        let locks=track.locks;
        let version = &mut track.versions[index];
        let protected_metadata=(!locks.is_empty()).then(||version.metadata.clone());
        if metadata.bpm.origin == Origin::Heuristic {
            if let Some(tags) = &mut version.tags { tags.automatic_bpm = metadata.bpm; }
        }
        // Rescanning filename hints cannot erase decoded analysis/user values.
        let old = &version.metadata;
        let analyzed = version.analysis.as_ref();
        let bpm = if old.bpm.origin == Origin::User {
            old.bpm
        } else if metadata.bpm.origin == Origin::User {
            metadata.bpm
        } else if old.bpm.origin == Origin::EmbeddedTag {
            old.bpm
        } else if metadata.bpm.origin == Origin::EmbeddedTag {
            metadata.bpm
        } else if let Some(measured) = analyzed.and_then(|record| record.bpm.as_ref()) {
            measured.value.map_or(Bpm::UNKNOWN, |value| Bpm::new(value, Origin::Heuristic))
        } else if matches!(metadata.bpm.origin, Origin::Unknown | Origin::FilenameHint)
            && old.bpm.origin == Origin::Heuristic {
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
            duration: analyzed.and_then(|record| record.duration.as_ref()).map(|measured| measured.value)
                .or(metadata.duration).or(old.duration),
            last_play: metadata.last_play.max(old.last_play),
            ..metadata
        };
        tags::reconcile(version);
        if let Some(previous)=protected_metadata {locks.preserve(&previous,&mut version.metadata);}
        Ok(version)
    }
    pub fn merge_import(&mut self, mut other: Catalog) -> Result<(), String> {
        // Validate all conflicts before mutation; imports are never partial.
        other.validate()?;
        let mut candidate = self.clone();
        for track in other.tracks {
            if let Some(&index) = candidate.index.get(&track.source) {
                let existing = &mut candidate.tracks[index];
                if existing != &track {
                    let pristine = |t: &Track| {
                        t.annotations.is_empty() && t.locks.is_empty() && t.versions.len() == 1
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
        let known: HashSet<_> = candidate.tracks.iter().map(|track| &track.id).collect();
        candidate.crates.merge_import(candidate.crates.revision(), &other.crates, |id| known.contains(id))
            .map_err(|error| error.to_string())?;
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BeforeCollections {
    schema: u32,
    tracks: Vec<Track>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BeforeWatchedRoots {
    schema: u32,
    tracks: Vec<Track>,
    crates: crates::CrateForest<TrackId>,
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
    let schema = header.get("schema").and_then(|v| v.as_u64());
    if schema.is_some_and(|schema| schema < 8) {
        tags::reject_legacy_fields(&header)?;
    }
    if schema.is_some_and(|version| version < 9) && (
        header.get("tracks").and_then(|v|v.as_array()).is_some_and(|tracks| tracks.iter().any(|track|track.get("annotations").is_some()))
        || header.get("crates").and_then(|v|v.get("nodes")).and_then(|v|v.as_array()).is_some_and(|nodes|nodes.iter().any(|node|node.get("annotation_rule").is_some()))) {
        return Err("Track annotations and smart annotation rules require library schema 9; original file preserved".into());
    }
    if schema.is_some_and(|version| version < 10) && header.get("crates").and_then(|v|v.get("nodes")).and_then(|v|v.as_array()).is_some_and(|nodes|nodes.iter().any(|node|node.get("smart_rule").is_some())) {
        return Err("Typed smart crate rules require library schema 10; original file preserved".into());
    }
    if schema.is_some_and(|version|version<11) && header.get("tracks").and_then(|v|v.as_array()).is_some_and(|tracks|tracks.iter().any(|track|track.get("locks").is_some())) {
        return Err("Preparation locks require library schema 11; original file preserved".into());
    }
    if schema.is_some_and(|version| version < 12) && header.get("crates").and_then(|v|v.get("nodes")).and_then(|v|v.as_array()).is_some_and(|nodes|nodes.iter().any(|node|node.get("favorite").is_some())) {
        return Err("Crate favorites require library schema 12; original file preserved".into());
    }
    let mut catalog = match schema {
        Some(12) => serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
        Some(11) | Some(10) | Some(9) => {
            let mut old: Catalog = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            old.schema = SCHEMA;
            old
        },
        Some(8) => {
            let mut old: Catalog = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            old.schema = SCHEMA;
            old
        },
        Some(7) => {
            let mut old: Catalog = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            old.schema = SCHEMA;
            old
        },
        Some(6) => {
            let old: BeforeWatchedRoots = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            debug_assert_eq!(old.schema,6);
            Catalog { tracks: old.tracks, crates: old.crates, ..Default::default() }
        },
        Some(2 | 3 | 4 | 5) => {
            let old: BeforeCollections = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            debug_assert!((2..=5).contains(&old.schema));
            Catalog { tracks: old.tracks, ..Default::default() }
        },
        Some(1) => {
            let old: V1 = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            debug_assert_eq!(old.schema, 1);
            Catalog {
                schema: SCHEMA,
                crates: Default::default(),
                watched_roots: Default::default(),
                index: HashMap::new(),
            relocations: HashMap::new(),
                tracks: old
                    .tracks
                    .into_iter()
                    .map(|track| Track {
                        annotations: Default::default(),
                    locks: Default::default(),
                        id: track.id,
                        source: track.source,
                        current: 0,
                        previous_locations: vec![],
                        versions: vec![Version {
                            fingerprint: track.fingerprint,
                            metadata: track.metadata,
                            preparation: track.preparation,
                            content_hash: None,
                            analysis: None,
                            tags: None,
                            audio_identity: None,
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
    if header.get("schema").and_then(|v|v.as_u64()).is_some_and(|v|v<7)
        && catalog.tracks.iter().any(|t|t.previous_locations.iter().any(|p|!matches!(p.source,LibSource::File(_)))) {
        return Err("invalid legacy relocated track association".into());
    }
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
    last_save_replaced: bool,
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
            last_save_replaced: false,
        })
    }
    pub(crate) fn last_save_replaced(&self) -> bool { self.last_save_replaced }
    #[cfg(test)]
    pub(crate) fn save_for_test(&mut self, checkpoint: impl FnMut(u8) -> Result<(), String>) -> Result<(), String> {
        self.save_with(checkpoint)
    }
    pub fn save(&mut self) -> Result<(), String> {
        self.save_with(|_| Ok(()))
    }
    fn save_with(
        &mut self,
        mut checkpoint: impl FnMut(u8) -> Result<(), String>,
    ) -> Result<(), String> {
        self.last_save_replaced = false;
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
            self.last_save_replaced = true;
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
