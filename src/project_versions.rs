//! Named native revisions share immutable decoded audio by content identity.
use crate::{
    engine::{dsp::Sample, media_source::FileFingerprint},
    project_file::{self, Bundle, Limits, Overwrite, SaveOutcome},
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::{self, File, OpenOptions},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

const MAX_VERSIONS: usize = 128;
const INDEX: &str = "versions.omat";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Entry {
    pub id: String,
    pub name: String,
    pub notes: String,
    pub captured_unix_ms: u64,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema: u32,
    entries: Vec<Entry>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reference {
    hash: String,
    metadata: project_file::Media,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision<T> {
    schema: u32,
    entry: Entry,
    document: T,
    media: Vec<Reference>,
}
pub(crate) struct Store {
    root: PathBuf,
    identity: [(u64, u64); 3],
    index_fingerprint: Option<FileFingerprint>,
    creation_warning: Option<String>,
    _lock: File,
    index: Index,
}
pub(crate) struct Review {
    pub entries: Vec<Entry>,
    pub orphaned_audio: Vec<String>,
    pub orphaned_revisions: Vec<String>,
    files: BTreeMap<String, FileFingerprint>,
    fingerprint: FileFingerprint,
}
pub(crate) struct Pruned {
    pub removed: usize,
    pub reclaimed_audio: usize,
    pub warning: Option<String>,
}

fn active(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("Version operation cancelled before publication".into())
    } else {
        Ok(())
    }
}
fn hash_name(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn version_id(text: &str) -> bool {
    text.len() == 32
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn entry_valid(entry: &Entry) -> bool {
    version_id(&entry.id)
        && !entry.name.trim().is_empty()
        && entry.name == entry.name.trim()
        && entry.name.len() <= 256
        && !entry.name.chars().any(char::is_control)
        && entry.notes.len() <= 4096
        && !entry.notes.contains('\0')
}
fn load<T: DeserializeOwned>(path: &Path, cancel: &AtomicBool) -> Result<Bundle<T>, String> {
    project_file::load(path, &Limits::default(), cancel).map_err(|e| e.to_string())
}
fn save<T: Serialize>(
    path: &Path,
    state: T,
    cancel: &AtomicBool,
    overwrite: Overwrite,
) -> Result<SaveOutcome, String> {
    project_file::save(
        path,
        &Bundle {
            state,
            media: Vec::new(),
        },
        overwrite,
        &Limits::default(),
        cancel,
    )
    .map_err(|e| e.to_string())
}
fn directory(path: &Path) -> Result<fs::Metadata, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() {
        return Err("Version storage must use real directories".into());
    }
    Ok(metadata)
}
/// Validate the bounded native index before owning or modifying its folder.
/// Takes a decoded index bundle; returns refusal for malformed identities or media.
fn validate_index(bundle: &Bundle<Index>) -> Result<(), String> {
    if !bundle.media.is_empty()
        || bundle.state.schema != 1
        || bundle.state.entries.len() > MAX_VERSIONS
        || bundle.state.entries.iter().any(|entry| !entry_valid(entry))
        || bundle
            .state
            .entries
            .iter()
            .map(|e| &e.id)
            .collect::<BTreeSet<_>>()
            .len()
            != bundle.state.entries.len()
    {
        return Err("Invalid or unsupported version index".into());
    }
    Ok(())
}
impl Store {
    /// Open one application-owned revision folder under an exclusive worker lock.
    /// Takes an absolute folder and cancellation flag; create starts a new empty store.
    pub(crate) fn open(root: &Path, create: bool, cancel: &AtomicBool) -> Result<Self, String> {
        active(cancel)?;
        if !root.is_absolute() || root.as_os_str().len() > 4096 {
            return Err("Choose an absolute version folder".into());
        }
        let mut creation_warning = None;
        if !root.exists() && create {
            let stage = crate::portable_project::Stage::new(
                root.parent().ok_or("Version folder needs a parent")?,
            )?;
            fs::create_dir(stage.path.join("audio")).map_err(|e| e.to_string())?;
            fs::create_dir(stage.path.join("revisions")).map_err(|e| e.to_string())?;
            save(
                &stage.path.join(INDEX),
                Index {
                    schema: 1,
                    entries: Vec::new(),
                },
                cancel,
                Overwrite::Never,
            )?;
            for folder in ["audio", "revisions"] {
                File::open(stage.path.join(folder))
                    .and_then(|f| f.sync_all())
                    .map_err(|e| e.to_string())?;
            }
            if let SaveOutcome::CommittedButDirectorySyncFailed(w) =
                crate::portable_project::publish(stage, root, cancel)?
            {
                creation_warning = Some(w);
            }
        }
        let metadata = directory(root)?;
        directory(&root.join("audio"))?;
        directory(&root.join("revisions"))?;
        let bundle = load::<Index>(&root.join(INDEX), cancel)?;
        validate_index(&bundle)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(root.join("owner.lock"))
            .map_err(|e| e.to_string())?;
        if !lock.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Version ownership lock must be a regular file".into());
        }
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Version storage is busy; retry after its current operation".into());
        }
        let bundle = load::<Index>(&root.join(INDEX), cancel)?;
        validate_index(&bundle)?;
        let store = Self {
            root: root.into(),
            identity: [
                metadata,
                directory(&root.join("audio"))?,
                directory(&root.join("revisions"))?,
            ]
            .map(|m| (m.dev(), m.ino())),
            index_fingerprint: Some(
                FileFingerprint::read(&root.join(INDEX)).ok_or("Cannot inspect version index")?,
            ),
            creation_warning,
            _lock: lock,
            index: bundle.state,
        };
        store.recheck()?;
        Ok(store)
    }
    fn recheck(&self) -> Result<(), String> {
        for (path, identity) in [
            self.root.clone(),
            self.root.join("audio"),
            self.root.join("revisions"),
        ]
        .iter()
        .zip(self.identity)
        {
            let metadata = directory(path)?;
            if (metadata.dev(), metadata.ino()) != identity {
                return Err("Version folder changed during the operation".into());
            }
        }
        if self.index_fingerprint.is_none()
            || FileFingerprint::read(&self.root.join(INDEX)) != self.index_fingerprint
        {
            return Err("Version index changed; refresh the list".into());
        }
        Ok(())
    }
    pub(crate) fn entries(&self) -> &[Entry] {
        &self.index.entries
    }
    fn revision<T: DeserializeOwned>(
        &self,
        entry: &Entry,
        cancel: &AtomicBool,
    ) -> Result<Revision<T>, String> {
        if !self.index.entries.contains(entry) {
            return Err("Selected version changed; refresh the list".into());
        }
        let bundle = load::<Revision<T>>(
            &self
                .root
                .join("revisions")
                .join(format!("{}.omat", entry.id)),
            cancel,
        )?;
        if !bundle.media.is_empty()
            || bundle.state.schema != 1
            || bundle.state.entry != *entry
            || bundle.state.media.len() > project_file::DEFAULT_MEDIA_LIMIT
            || bundle.state.media.iter().any(|r| !hash_name(&r.hash))
        {
            return Err("Invalid named revision or media reference".into());
        }
        Ok(bundle.state)
    }
    /// Save a named immutable musical revision while sharing unchanged audio.
    /// Takes a native bundle, name/notes and a final commit callback; returns the published entry.
    pub(crate) fn snapshot<T: Serialize, G>(
        &mut self,
        bundle: &Bundle<T>,
        name: String,
        notes: String,
        cancel: &AtomicBool,
        authorize: impl FnOnce() -> Result<G, String>,
    ) -> Result<(Entry, SaveOutcome), String> {
        self.recheck()?;
        active(cancel)?;
        let words = crate::engine::midi_edit::NoteId::new().words();
        let entry = Entry {
            id: format!("{:016x}{:016x}", words[0], words[1]),
            name,
            notes,
            captured_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_millis()
                .min(u64::MAX as u128) as u64,
        };
        if !entry_valid(&entry) || self.index.entries.len() >= MAX_VERSIONS {
            return Err(
                "Use a visible name, notes up to 4096 bytes and at most 128 versions".into(),
            );
        }
        if bundle.media.len() > project_file::DEFAULT_MEDIA_LIMIT
            || bundle
                .media
                .iter()
                .map(|s| s.data.len() as u64 * 4)
                .sum::<u64>()
                > project_file::DEFAULT_PCM_LIMIT
        {
            return Err("Version dependencies exceed native project limits".into());
        }
        let mut media = Vec::new();
        for sample in &bundle.media {
            active(cancel)?;
            let digest = crate::project_dependencies::audio_hash(sample, cancel)?;
            let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
            let path = self.root.join("audio").join(format!("{hash}.omat"));
            if path.exists() {
                let existing = load::<()>(&path, cancel)?;
                if existing.media.len() != 1
                    || crate::project_dependencies::audio_hash(&existing.media[0], cancel)?
                        != digest
                {
                    return Err(
                        "Shared immutable audio was changed; no version was published".into(),
                    );
                }
            } else {
                project_file::save(
                    &path,
                    &Bundle {
                        state: (),
                        media: vec![sample.clone()],
                    },
                    Overwrite::Never,
                    &Limits::default(),
                    cancel,
                )
                .map_err(|e| e.to_string())?;
            }
            media.push(Reference {
                hash,
                metadata: project_file::Media::from_sample(sample),
            });
        }
        save(
            &self
                .root
                .join("revisions")
                .join(format!("{}.omat", entry.id)),
            Revision {
                schema: 1,
                entry: entry.clone(),
                document: &bundle.state,
                media,
            },
            cancel,
            Overwrite::Never,
        )?;
        File::open(self.root.join("audio"))
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        File::open(self.root.join("revisions"))
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        active(cancel)?;
        self.recheck()?;
        let _guard = authorize()?;
        let mut next = self.index.clone();
        next.entries.push(entry.clone());
        let outcome = save(&self.root.join(INDEX), &next, cancel, Overwrite::Replace)?;
        self.index = next;
        self.index_fingerprint = FileFingerprint::read(&self.root.join(INDEX));
        let warning = self.creation_warning.take().or_else(|| {
            self.index_fingerprint
                .is_none()
                .then(|| "Snapshot committed; index inspection unavailable".into())
        });
        let outcome = match (outcome, warning) {
            (SaveOutcome::Durable, Some(w)) => SaveOutcome::CommittedButDirectorySyncFailed(w),
            (outcome, _) => outcome,
        };
        Ok((entry, outcome))
    }
    /// Reopen exactly the selected immutable revision and verify every audio hash.
    /// Takes its verified entry; returns a complete native document without changing a live session.
    pub(crate) fn reopen<T: DeserializeOwned>(
        &self,
        entry: &Entry,
        cancel: &AtomicBool,
    ) -> Result<Bundle<T>, String> {
        self.recheck()?;
        let revision = self.revision::<T>(entry, cancel)?;
        let mut media = Vec::new();
        let mut loaded: HashMap<String, Arc<Sample>> = HashMap::new();
        let mut total = 0u64;
        for reference in revision.media {
            active(cancel)?;
            let sample = match loaded.get(&reference.hash) {
                Some(sample) => sample.clone(),
                None => {
                    let mut asset = load::<()>(
                        &self
                            .root
                            .join("audio")
                            .join(format!("{}.omat", reference.hash)),
                        cancel,
                    )?;
                    if asset.media.len() != 1 {
                        return Err("Shared audio record has an invalid asset count".into());
                    }
                    let sample = asset.media.pop().unwrap();
                    let digest = crate::project_dependencies::audio_hash(&sample, cancel)?;
                    let hash: String = digest.iter().map(|b| format!("{b:02x}")).collect();
                    if hash != reference.hash {
                        return Err("Shared immutable audio checksum changed".into());
                    }
                    loaded.insert(hash, sample.clone());
                    sample
                }
            };
            total = total
                .checked_add(sample.data.len() as u64 * 4)
                .ok_or("Version audio size overflow")?;
            if total > project_file::DEFAULT_PCM_LIMIT {
                return Err("Version audio exceeds native project bounds".into());
            }
            let data = sample.data.clone();
            media.push(
                reference
                    .metadata
                    .into_sample(data, sample.sr, sample.ch)
                    .map_err(|e| e.to_string())?,
            );
        }
        Ok(Bundle {
            state: revision.document,
            media,
        })
    }
    /// Preview the complete asset impact before pruning selected versions.
    /// Takes stable version IDs; returns removed entries and newly unreferenced audio names.
    pub(crate) fn review_prune(
        &self,
        ids: &[String],
        cancel: &AtomicBool,
    ) -> Result<Review, String> {
        self.recheck()?;
        let selected: BTreeSet<_> = ids.iter().collect();
        if selected.len() != ids.len()
            || ids
                .iter()
                .any(|id| !self.index.entries.iter().any(|e| &e.id == id))
        {
            return Err("Select current versions once each before pruning".into());
        }
        let mut removed = Vec::new();
        let mut retained = BTreeSet::new();

        for entry in &self.index.entries {
            let revision = self.revision::<serde_json::Value>(entry, cancel)?;
            if selected.contains(&entry.id) {
                removed.push(entry.clone());
            } else {
                retained.extend(revision.media.into_iter().map(|r| r.hash));
            }
        }
        let mut files = BTreeMap::new();
        let mut orphaned_audio = Vec::new();
        let mut orphaned_revisions = Vec::new();
        for (folder, audio) in [("audio", true), ("revisions", false)] {
            for (i, file) in fs::read_dir(self.root.join(folder))
                .map_err(|e| e.to_string())?
                .enumerate()
            {
                active(cancel)?;
                if i >= 65_536 {
                    return Err("Version file inventory exceeds its supported limit".into());
                }
                let file = file.map_err(|e| e.to_string())?;
                let name = file
                    .file_name()
                    .into_string()
                    .map_err(|_| "Invalid owned version file name")?;
                let id = name
                    .strip_suffix(".omat")
                    .ok_or("Unexpected file in version storage; no cleanup authorized")?;
                if !(if audio { hash_name(id) } else { version_id(id) })
                    || !fs::symlink_metadata(file.path())
                        .map_err(|e| e.to_string())?
                        .is_file()
                {
                    return Err("Unexpected file in version storage; no cleanup authorized".into());
                }
                let unused = if audio {
                    !retained.contains(id)
                } else {
                    !self
                        .index
                        .entries
                        .iter()
                        .any(|e| e.id == id && !selected.contains(&e.id))
                };
                if unused {
                    files.insert(
                        format!("{folder}/{name}"),
                        FileFingerprint::read(&file.path()).ok_or("Cannot inspect cleanup file")?,
                    );
                    if audio {
                        orphaned_audio.push(id.to_string());
                    } else {
                        orphaned_revisions.push(id.to_string());
                    }
                }
            }
        }
        orphaned_audio.sort();
        orphaned_revisions.sort();
        Ok(Review {
            entries: removed,
            orphaned_audio,
            orphaned_revisions,
            files,
            fingerprint: self
                .index_fingerprint
                .ok_or("Cannot inspect version index")?,
        })
    }
    /// Publish the reviewed pruning decision before reclaiming unreferenced files.
    /// Takes a still-current review and commit callback; returns truthful committed cleanup warnings.
    pub(crate) fn prune<G>(
        &mut self,
        review: Review,
        cancel: &AtomicBool,
        authorize: impl FnOnce() -> Result<G, String>,
    ) -> Result<Pruned, String> {
        let ids: Vec<_> = review.entries.iter().map(|e| e.id.clone()).collect();
        let current = self.review_prune(&ids, cancel)?;
        if current.entries != review.entries
            || current.orphaned_audio != review.orphaned_audio
            || current.orphaned_revisions != review.orphaned_revisions
            || current.files != review.files
            || current.fingerprint != review.fingerprint
        {
            return Err("Version storage changed after pruning review; review again".into());
        }
        active(cancel)?;
        self.recheck()?;
        let _guard = authorize()?;
        let mut next = self.index.clone();
        next.entries.retain(|entry| !ids.contains(&entry.id));
        let outcome = save(&self.root.join(INDEX), &next, cancel, Overwrite::Replace)?;
        self.index = next;
        self.index_fingerprint = FileFingerprint::read(&self.root.join(INDEX));
        let mut warning = match outcome {
            SaveOutcome::Durable => None,
            SaveOutcome::CommittedButDirectorySyncFailed(warning) => Some(warning),
        };
        warning = warning.or_else(|| {
            self.index_fingerprint.is_none().then(|| {
                "Pruning committed; cleanup deferred because index inspection failed".into()
            })
        });
        if warning.is_none() {
            if let Err(error) = self.recheck() {
                warning = Some(format!("Pruning committed; cleanup deferred: {error}"));
            }
        }
        let mut reclaimed = 0;
        if warning.is_none() {
            for id in &review.orphaned_revisions {
                if let Err(error) =
                    fs::remove_file(self.root.join("revisions").join(format!("{id}.omat")))
                {
                    warning = Some(format!(
                        "Pruning committed; revision cleanup deferred: {error}"
                    ));
                    break;
                }
            }
            if warning.is_none() {
                for hash in &review.orphaned_audio {
                    if let Err(error) =
                        fs::remove_file(self.root.join("audio").join(format!("{hash}.omat")))
                    {
                        warning = Some(format!(
                            "Pruning committed; audio cleanup deferred: {error}"
                        ));
                        break;
                    }
                    reclaimed += 1;
                }
            }
            for path in [
                self.root.join("audio"),
                self.root.join("revisions"),
                self.root.clone(),
            ] {
                if let Err(error) = File::open(path).and_then(|f| f.sync_all()) {
                    warning = Some(format!("Pruning committed; folder sync failed: {error}"));
                    break;
                }
            }
        }
        Ok(Pruned {
            removed: review.entries.len(),
            reclaimed_audio: reclaimed,
            warning,
        })
    }
}

#[cfg(test)]
mod tests;
