use super::*;

#[derive(Clone, Debug)]
pub(crate) struct Recovery {
    pub id: String,
    pub files: usize,
    pub description: String,
    pub active: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    device: u64,
    inode: u64,
    length: u64,
    modified: (i64, i64),
    hash: [u8; 32],
}
impl Proof {
    fn capture(path: &Path, hash: [u8; 32]) -> Result<Self, String> {
        let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        if !metadata.is_file() {
            return Err("Recovery proof requires a regular file".into());
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            hash,
        })
    }
    fn inspect(&self, path: &Path, cancel: &AtomicBool) -> Result<Option<FileFingerprint>, String> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        if !metadata.is_file()
            || metadata.dev() != self.device
            || metadata.ino() != self.inode
            || metadata.len() != self.length
            || (metadata.mtime(), metadata.mtime_nsec()) != self.modified
        {
            return Err(format!(
                "Recovery path changed; preserved: {}",
                path.display()
            ));
        }
        let fingerprint = FileFingerprint::from_metadata(&metadata);
        if content::hash_file(path, fingerprint, || !cancel.load(Ordering::Acquire))? != self.hash {
            return Err(format!(
                "Recovery bytes changed; preserved: {}",
                path.display()
            ));
        }
        Ok(Some(fingerprint))
    }
    fn remove(&self, path: &Path, cancel: &AtomicBool) -> Result<(), String> {
        if let Some(fingerprint) = self.inspect(path, cancel)? {
            let parent = Folder::open(path.parent().ok_or("Recovery path has no parent")?)?;
            let name = path.file_name().ok_or("Recovery path has no filename")?;
            if FileFingerprint::read(&parent.anchored(name)) != Some(fingerprint) {
                return Err("Recovery output changed before cleanup; preserved".into());
            }
            fs::remove_file(parent.anchored(name)).map_err(|e| e.to_string())?;
            parent.file.sync_all().map_err(|e| e.to_string())?;
            parent.check()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Row {
    before: Track,
    after: Option<Track>,
    restored: Option<Track>,
    original: PathBuf,
    recovery: PathBuf,
    destination: PathBuf,
    temporary: PathBuf,
    original_proof: Proof,
    copy_proof: Proof,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    id: String,
    rows: Vec<Row>,
}
pub(crate) struct Journal {
    path: PathBuf,
    fingerprint: Option<FileFingerprint>,
    record: Record,
}
impl Journal {
    fn path(store: &Store) -> PathBuf {
        store.path.with_extension("files.json")
    }
    fn load(store: &Store) -> Result<Option<Self>, String> {
        Self::load_path(Self::path(store))
    }
    fn archive_path(store: &Store, id: &str) -> PathBuf {
        store.path.with_extension(format!("files-{id}.json"))
    }
    fn load_path(path: PathBuf) -> Result<Option<Self>, String> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(format!("File recovery record preserved: {error}")),
        };
        let metadata = file.metadata().map_err(|e| e.to_string())?;
        if !metadata.is_file() || metadata.len() > MAX_BYTES || metadata.mode() & 0o077 != 0 {
            return Err(
                "File recovery record must be a private regular file of at most 64 MiB; preserved"
                    .into(),
            );
        }
        let fingerprint = FileFingerprint::from_metadata(&metadata);
        let mut bytes = Vec::new();
        file.take(MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES || FileFingerprint::read(&path) != Some(fingerprint) {
            return Err("File recovery record changed while reading; preserved".into());
        }
        let record: Record = serde_json::from_slice(&bytes)
            .map_err(|e| format!("File recovery record is invalid; preserved: {e}"))?;
        let journal = Self {
            path,
            fingerprint: Some(fingerprint),
            record,
        };
        journal.validate()?;
        Ok(Some(journal))
    }
    fn validate(&self) -> Result<(), String> {
        if self.record.schema != 1
            || self.record.rows.is_empty()
            || self.record.rows.len() > MAX_SELECTION
            || self.record.id.len() != 32
            || !self
                .record
                .id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("Unsupported or invalid file recovery record; preserved".into());
        }
        let mut ids = HashSet::new();
        for (index, row) in self.record.rows.iter().enumerate() {
            if !ids.insert(&row.before.id)
                || row
                    .after
                    .as_ref()
                    .is_some_and(|track| track.id != row.before.id)
                || row
                    .restored
                    .as_ref()
                    .is_some_and(|track| track.id != row.before.id)
                || [
                    &row.original,
                    &row.destination,
                    &row.temporary,
                    &row.recovery,
                ]
                .iter()
                .any(|path| {
                    !path.is_absolute()
                        || path
                            .components()
                            .any(|c| matches!(c, std::path::Component::ParentDir))
                })
                || row.original.file_name() != row.destination.file_name()
                || row.original.parent() != row.recovery.parent()
                || row.destination.parent() != row.temporary.parent()
                || row.recovery.file_name()
                    != Some(OsStr::new(&format!(
                        ".omatainer-move-{}-{index}",
                        self.record.id
                    )))
                || !row
                    .temporary
                    .file_name()
                    .is_some_and(|name| name.as_bytes().starts_with(b".omatainer-copy-"))
            {
                return Err("File recovery paths or identities are invalid; preserved".into());
            }
        }
        Ok(())
    }
    fn save(&mut self) -> Result<(), String> {
        self.validate()?;
        let bytes = serde_json::to_vec(&self.record).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("File recovery record exceeds 64 MiB".into());
        }
        let folder = Folder::open(self.path.parent().ok_or("Recovery record has no parent")?)?;
        let name = self.path.file_name().ok_or("Recovery record has no name")?;
        if store_identity(&folder.anchored(name))? != self.fingerprint {
            return Err("File recovery record changed outside this operation; preserved".into());
        }
        let temporary: std::ffi::OsString = format!(
            ".omatainer-file-journal-{}",
            crate::sampler_bank::BankId::new()?
        )
        .into();
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(folder.anchored(&temporary))
            .map_err(|e| e.to_string())?;
        let owned = file.metadata().map_err(|e| e.to_string())?;
        let result = (|| {
            file.write_all(&bytes).map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            folder.check()?;
            if store_identity(&folder.anchored(name))? != self.fingerprint {
                return Err("File recovery record changed before replacement; preserved".into());
            }
            if self.fingerprint.is_none() {
                folder.rename_new(&temporary, name)?;
            } else {
                fs::rename(folder.anchored(&temporary), folder.anchored(name))
                    .map_err(|e| e.to_string())?;
            }
            self.fingerprint = Some(FileFingerprint::from_metadata(
                &file.metadata().map_err(|e| e.to_string())?,
            ));
            folder.file.sync_all().map_err(|e| e.to_string())?;
            folder.check()?;
            if FileFingerprint::read(&self.path) != self.fingerprint {
                return Err("Installed file recovery record changed; preserved".into());
            }
            Ok(())
        })();
        let cleanup = remove_owned(&folder.anchored(&temporary), &owned);
        result.and(cleanup)
    }
    /// Record owned staged files before installing any final destination.
    /// Takes the locked catalog owner and staged copies; returns a durable recovery owner without changing source files.
    pub(crate) fn prepare(store: &Store, staged: &Staged) -> Result<Self, String> {
        if Self::load(store)?.is_some() {
            return Err(
                "Inspect and restore the pending file move before starting another transfer".into(),
            );
        }
        let id = crate::sampler_bank::BankId::new()?.to_string();
        let rows = staged
            .outputs
            .iter()
            .enumerate()
            .map(|(index, output)| {
                Ok(Row {
                    before: output.target.track.clone(),
                    after: None,
                    restored: None,
                    original: output.source.path.clone(),
                    recovery: output
                        .source
                        .path
                        .parent()
                        .ok_or("Source has no parent")?
                        .join(format!(".omatainer-move-{id}-{index}")),
                    destination: staged.folder.path.join(&output.name),
                    temporary: staged.folder.path.join(&output.current),
                    original_proof: Proof::capture(&output.source.path, output.hash)?,
                    copy_proof: Proof::capture(
                        &staged.folder.anchored(&output.current),
                        output.hash,
                    )?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut journal = Self {
            path: Self::path(store),
            fingerprint: None,
            record: Record {
                schema: 1,
                id,
                rows,
            },
        };
        journal.save()?;
        Ok(journal)
    }
    /// Bind the pending catalog locations before the catalog replacement.
    /// Takes the complete candidate; records each exact prepared row for restart and restoration.
    pub(crate) fn bind_candidate(&mut self, catalog: &Catalog) -> Result<(), String> {
        for row in &mut self.record.rows {
            row.after = Some(
                catalog
                    .tracks
                    .iter()
                    .find(|track| track.id == row.before.id)
                    .ok_or("Candidate lost a moved track")?
                    .clone(),
            );
        }
        self.save()
    }
    /// Retire originals only after confirmed catalog publication.
    /// Takes the saved catalog and cancellation; preserves original bytes in their source folders and leaves the recovery record available.
    pub(crate) fn retire_originals(
        &self,
        catalog: &Catalog,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        self.require_rows(catalog, |row| row.after.as_ref())?;
        for row in &self.record.rows {
            row.copy_proof
                .inspect(&row.destination, cancel)?
                .ok_or("Moved destination is unavailable; original preserved")?;
            if row.original_proof.inspect(&row.original, cancel)?.is_some() {
                let folder = Folder::open(row.original.parent().unwrap())?;
                folder.rename_new(
                    row.original.file_name().unwrap(),
                    row.recovery.file_name().unwrap(),
                )?;
                folder.file.sync_all().map_err(|e| e.to_string())?;
                folder.check()?;
                row.original_proof
                    .inspect(&row.recovery, cancel)?
                    .ok_or("Retired original disappeared; keep the recovery record")?;
            } else {
                row.original_proof
                    .inspect(&row.recovery, cancel)?
                    .ok_or("Original and recovery files are unavailable")?;
            }
        }
        Ok(())
    }
    fn require_rows<'a>(
        &'a self,
        catalog: &Catalog,
        select: impl Fn(&'a Row) -> Option<&'a Track>,
    ) -> Result<(), String> {
        for row in &self.record.rows {
            let expected =
                select(row).ok_or("This file transaction has no published catalog review")?;
            if !catalog.tracks.iter().any(|track| track == expected) {
                return Err("A moved track changed after publication; review recovery before changing it again".into());
            }
        }
        Ok(())
    }
    fn matches<'a>(
        &'a self,
        catalog: &Catalog,
        select: impl Fn(&'a Row) -> Option<&'a Track>,
    ) -> bool {
        self.require_rows(catalog, select).is_ok()
    }
    /// Restore original locations while preserving unrelated catalog changes.
    /// Takes the current locked store and cancellation; returns a complete restoration candidate or cleans an uncommitted transaction after verifying its originals.
    pub(crate) fn restore_prepare(
        &mut self,
        store: &Store,
        cancel: &AtomicBool,
    ) -> Result<Option<Catalog>, String> {
        if self.matches(&store.catalog, |row| row.restored.as_ref()) {
            self.finish_restore(&store.catalog, cancel)?;
            return Ok(None);
        }
        if self.matches(&store.catalog, |row| Some(&row.before)) {
            for row in &self.record.rows {
                row.original_proof
                    .inspect(&row.original, cancel)?
                    .ok_or("Original is unavailable; uncommitted copy recovery was preserved")?;
            }
            self.cleanup_copies(cancel)?;
            self.clear()?;
            return Ok(None);
        }
        self.require_rows(&store.catalog, |row| row.after.as_ref())?;
        let mut installed = Vec::new();
        for row in &self.record.rows {
            row.copy_proof
                .inspect(&row.destination, cancel)?
                .ok_or("Current moved copy is unavailable; restoration preserved")?;
            if row.original_proof.inspect(&row.original, cancel)?.is_none() {
                row.original_proof
                    .inspect(&row.recovery, cancel)?
                    .ok_or("Original recovery bytes are unavailable")?;
                let folder = Folder::open(row.original.parent().unwrap())?;
                folder.rename_new(
                    row.recovery.file_name().unwrap(),
                    row.original.file_name().unwrap(),
                )?;
                folder.file.sync_all().map_err(|e| e.to_string())?;
                folder.check()?;
            }
            let fingerprint = row
                .original_proof
                .inspect(&row.original, cancel)?
                .ok_or("Restored original could not be verified")?;
            installed.push(Installed {
                target: Target::capture(row.after.as_ref().unwrap()),
                destination: row.original.clone(),
                fingerprint,
                hash: row.original_proof.hash,
            });
        }
        let candidate =
            store
                .catalog
                .adopt_file_copies(&installed, store.catalog.crates.revision(), cancel)?;
        for row in &mut self.record.rows {
            row.restored = Some(
                candidate
                    .tracks
                    .iter()
                    .find(|track| track.id == row.before.id)
                    .unwrap()
                    .clone(),
            );
        }
        self.save()?;
        Ok(Some(candidate))
    }
    fn cleanup_copies(&self, cancel: &AtomicBool) -> Result<(), String> {
        for row in &self.record.rows {
            row.copy_proof.remove(&row.temporary, cancel)?;
            row.copy_proof.remove(&row.destination, cancel)?;
        }
        Ok(())
    }
    /// Complete cleanup after the restored catalog is confirmed saved.
    /// Takes the confirmed catalog and cancellation; removes only exact owned copies and the unchanged recovery record.
    pub(crate) fn finish_restore(
        &self,
        catalog: &Catalog,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        self.require_rows(catalog, |row| row.restored.as_ref())?;
        for row in &self.record.rows {
            row.original_proof
                .inspect(&row.original, cancel)?
                .ok_or("Restored original unavailable; copies preserved")?;
        }
        self.cleanup_copies(cancel)?;
        self.clear()
    }
    /// Clear an unchanged recovery record after verified output rollback.
    /// Takes this owner; returns after directory sync or preserves a changed record.
    pub(crate) fn clear(&self) -> Result<(), String> {
        if FileFingerprint::read(&self.path) != self.fingerprint {
            return Err("Recovery record changed before cleanup; preserved".into());
        }
        let folder = Folder::open(self.path.parent().unwrap())?;
        fs::remove_file(folder.anchored(self.path.file_name().unwrap()))
            .map_err(|e| e.to_string())?;
        folder.file.sync_all().map_err(|e| e.to_string())?;
        folder.check()
    }
    fn review(&self, active: bool) -> Recovery {
        Recovery{id:self.record.id.clone(),files:self.record.rows.len(),active,description:"Originals remain recoverable in their source folders. Restore verifies every selected row and owned file before changing saved locations. Changed or unknown files are preserved.".into()}
    }
    /// Load one exact recovery operation for an explicit restore request.
    /// Takes the store and reviewed operation identity; refuses a stale or absent record.
    pub(crate) fn reviewed(store: &Store, id: &str) -> Result<Self, String> {
        let journal = match Self::load(store)? {
            Some(journal) => journal,
            None => Self::load_path(Self::archive_path(store, id))?
                .ok_or("No reviewed file recovery record")?,
        };
        if journal.record.id != id {
            return Err("Pending move changed; inspect file recovery again".into());
        }
        Ok(journal)
    }
    /// Keep moved catalog locations and retain the complete recovery record.
    /// Takes the confirmed store and cancellation; archives one unchanged record without deleting original audio or replacing another recovery record.
    pub(crate) fn keep(self, store: &Store, cancel: &AtomicBool) -> Result<(), String> {
        if self.path != Self::path(store) {
            return Err("This move is already retained".into());
        }
        if archived(store)?.len() >= 128 {
            return Err(
                "128 saved moves are retained; restore a reviewed move before retaining another"
                    .into(),
            );
        }
        self.retire_originals(&store.catalog, cancel)?;
        if FileFingerprint::read(&self.path) != self.fingerprint {
            return Err("Move record changed before retention; preserved".into());
        }
        let archive = Self::archive_path(store, &self.record.id);
        let folder = Folder::open(self.path.parent().unwrap())?;
        folder.rename_new(self.path.file_name().unwrap(), archive.file_name().unwrap())?;
        folder.file.sync_all().map_err(|e| e.to_string())?;
        folder.check()?;
        let retained =
            Self::load_path(archive)?.ok_or("Retained move record could not be reopened")?;
        if retained.record.id != self.record.id {
            return Err("Retained move identity changed; preserved".into());
        }
        Ok(())
    }
}

/// Inspect the private move record on the catalog worker.
/// Takes the locked store; returns a bounded description or preserves a malformed record with an error.
pub(crate) fn pending(store: &Store) -> Result<Option<Recovery>, String> {
    Ok(Journal::load(store)?.map(|journal| journal.review(true)))
}

/// Inspect bounded retained move records without changing source audio.
/// Takes the locked store; returns up to 128 private saved recovery descriptions or refuses malformed records without deleting them.
pub(crate) fn archived(store: &Store) -> Result<Vec<Recovery>, String> {
    let prefix_path = store.path.with_extension("files-");
    let prefix = prefix_path.file_name().unwrap().as_bytes();
    let mut paths = Vec::new();
    for entry in fs::read_dir(store.path.parent().unwrap()).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(id) = name
            .as_bytes()
            .strip_prefix(prefix)
            .and_then(|bytes| bytes.strip_suffix(b".json"))
        else {
            continue;
        };
        if id.len() != 32
            || !id
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        {
            return Err("Malformed saved move filename; preserved for recovery review".into());
        }
        if paths.len() == 128 {
            return Err("Saved move review exceeds 128 records; all records are preserved".into());
        }
        paths.push(entry.path());
    }
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            Journal::load_path(path)?
                .map(|journal| journal.review(false))
                .ok_or_else(|| "Saved recovery record disappeared during review".into())
        })
        .collect()
}
