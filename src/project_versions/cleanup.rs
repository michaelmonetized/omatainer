use super::*;
use sha2::{Digest, Sha256};
use std::os::unix::fs::DirBuilderExt;
use std::{ffi::CString, io::Read, os::fd::AsRawFd};

const FOLDER: &str = ".cleanup";
const RECORD: &str = "record.omat";
const MAX_RECORDS: usize = 128;
const MAX_FILES: usize = 65_536;
const MAX_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Proof {
    device: u64,
    inode: u64,
    bytes: u64,
    modified: (i64, i64),
    sha256: [u8; 32],
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    id: String,
    directories: [(u64, u64); 3],
    before: Index,
    after: Index,
    files: BTreeMap<String, Proof>,
    retained: BTreeMap<String, Proof>,
}
pub(crate) struct Saved {
    pub id: String,
    pub versions: usize,
    pub files: usize,
    pub bytes: u64,
    pub restored: bool,
    pub pending: bool,
}
pub(super) struct Prepared {
    record: Record,
    fingerprint: FileFingerprint,
}
impl Prepared {
    pub(super) fn id(&self) -> &str {
        &self.record.id
    }
}
struct Bound {
    file: File,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    schema: u32,
    record: FileFingerprint,
}
/// Verify one immutable completion marker.
/// Takes the owned recovery folder, marker name, record and cancellation; returns whether its matching private native marker exists.
fn marked(
    folder: &Bound,
    name: &str,
    prepared: &Prepared,
    cancel: &AtomicBool,
) -> Result<bool, String> {
    let path = folder.path().join(name);
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.to_string()),
    };
    let m = file.metadata().map_err(|e| e.to_string())?;
    if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
        return Err("Cleanup status must be private and owned".into());
    }
    let bundle: Bundle<Marker> = project_file::load_from_file(file, &Limits::default(), cancel)
        .map_err(|e| e.to_string())?;
    if !bundle.media.is_empty()
        || bundle.state.schema != 1
        || bundle.state.record != prepared.fingerprint
    {
        return Err("Cleanup status does not match its recovery record".into());
    }
    Ok(true)
}
/// Save a completion marker without replacing existing bytes.
/// Takes the owned folder, marker name, verified record and cancellation; returns native publication durability.
fn mark(
    folder: &Bound,
    name: &str,
    prepared: &Prepared,
    cancel: &AtomicBool,
) -> Result<SaveOutcome, String> {
    if marked(folder, name, prepared, cancel)? {
        return Ok(SaveOutcome::Durable);
    }
    save(
        &folder.path().join(name),
        Marker {
            schema: 1,
            record: prepared.fingerprint,
        },
        cancel,
        Overwrite::Never,
    )
}
impl Bound {
    /// Bind a real directory without following its final path component.
    /// Takes a directory path; returns its owned open descriptor or an OS refusal.
    fn open(path: &Path) -> Result<Self, String> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|e| e.to_string())?;
        Ok(Self { file })
    }
    /// Address an owned directory descriptor.
    /// Takes this live descriptor; returns its process-local anchored path.
    fn path(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.file.as_raw_fd()))
    }
    /// Inspect a bound directory identity.
    /// Takes this descriptor; returns its filesystem and inode numbers.
    fn identity(&self) -> Result<(u64, u64), String> {
        let m = self.file.metadata().map_err(|e| e.to_string())?;
        Ok((m.dev(), m.ino()))
    }
    /// Bind one named directory under an owned parent.
    /// Takes a validated child name; returns its real directory descriptor.
    fn child(&self, name: &str) -> Result<Self, String> {
        Self::open(&self.path().join(name))
    }
    /// Flush a bound directory's entries.
    /// Takes this owned descriptor; returns the filesystem durability result.
    fn sync(&self) -> Result<(), String> {
        self.file.sync_all().map_err(|e| e.to_string())
    }
    /// Move an owned file without replacing a destination.
    /// Takes the validated basename and bound destination; returns rename and both directory sync results, retaining any completed move on a warning.
    fn rename_new(&self, name: &str, target: &Self) -> Result<(), String> {
        let name = CString::new(name).map_err(|e| e.to_string())?;
        if unsafe {
            libc::renameat2(
                self.file.as_raw_fd(),
                name.as_ptr(),
                target.file.as_raw_fd(),
                name.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        self.sync()?;
        target.sync()
    }
}
/// Bind the exact locked store and its private recovery directory.
/// Takes the store and permission to create recovery storage; returns owned root and recovery descriptors after identity checks.
fn owned(store: &Store, create: bool) -> Result<(Bound, Bound), String> {
    store.recheck()?;
    let root = Bound::open(&store.root)?;
    if root.identity()? != store.identity[0] {
        return Err("Version folder changed".into());
    }
    if create {
        match fs::DirBuilder::new()
            .mode(0o700)
            .create(root.path().join(FOLDER))
        {
            Ok(()) => root.sync()?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    let folder = root.child(FOLDER)?;
    let m = folder.file.metadata().map_err(|e| e.to_string())?;
    if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
        return Err("Cleanup recovery folder must be private and owned by this user".into());
    }
    Ok((root, folder))
}
/// Validate a native file's recovery location.
/// Takes one relative audio or revision path; returns its directory and canonical basename without accepting traversal.
fn location(relative: &str) -> Result<(&str, &str), String> {
    let (folder, name) = relative.split_once('/').ok_or("Invalid cleanup file")?;
    let id = name.strip_suffix(".omat").ok_or("Invalid cleanup file")?;
    if !match folder {
        "audio" => hash_name(id),
        "revisions" => version_id(id),
        _ => false,
    } {
        return Err("Invalid cleanup file".into());
    }
    Ok((folder, name))
}
/// Verify exact bytes and identity through one regular file descriptor.
/// Takes a bounded native path and cancellation; returns SHA-256, filesystem identity, length and modification time only after unchanged descriptor and path checks.
fn proof(path: &Path, cancel: &AtomicBool) -> Result<Proof, String> {
    active(cancel)?;
    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| e.to_string())?;
    let before = f.metadata().map_err(|e| e.to_string())?;
    if !before.is_file() || before.len() > project_file::DEFAULT_PCM_LIMIT + 64 * 1024 * 1024 {
        return Err("Cleanup requires bounded regular native files".into());
    }
    let mut hash = Sha256::new();
    let mut bytes = [0; 65_536];
    loop {
        active(cancel)?;
        let n = f.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    let after = f.metadata().map_err(|e| e.to_string())?;
    let fp = FileFingerprint::from_metadata(&before);
    if FileFingerprint::from_metadata(&after) != fp || FileFingerprint::read(path) != Some(fp) {
        return Err("Cleanup file changed while verified".into());
    }
    Ok(Proof {
        device: before.dev(),
        inode: before.ino(),
        bytes: before.len(),
        modified: (before.mtime(), before.mtime_nsec()),
        sha256: hash.finalize().into(),
    })
}
fn index_equal(a: &Index, b: &Index) -> bool {
    a.schema == b.schema && a.entries == b.entries
}
/// Bound cumulative recovery file bytes.
/// Takes the running count and next file size; returns refusal above the checked 64 GiB inventory limit.
fn reserve(total: &mut u64, bytes: u64) -> Result<(), String> {
    *total = total
        .checked_add(bytes)
        .filter(|n| *n <= MAX_BYTES)
        .ok_or("Cleanup disk inventory exceeds 64 GiB")?;
    Ok(())
}
/// Validate a durable cleanup record against its original store.
/// Takes the native record and locked store; returns refusal for invalid indexes, identities, locations or budgets.
fn validate(record: &Record, store: &Store) -> Result<(), String> {
    for index in [&record.before, &record.after] {
        validate_index(&Bundle {
            state: index.clone(),
            media: vec![],
        })?;
    }
    if record.schema != 1
        || !version_id(&record.id)
        || record.directories != store.identity
        || record.files.len().saturating_add(record.retained.len()) > MAX_FILES
        || record
            .after
            .entries
            .iter()
            .any(|e| !record.before.entries.contains(e))
    {
        return Err("Invalid or changed cleanup recovery record".into());
    }
    let mut bytes = 0;
    for proof in record.files.values().chain(record.retained.values()) {
        reserve(&mut bytes, proof.bytes)?;
    }
    for path in record.files.keys() {
        location(path)?;
    }
    for path in record.retained.keys() {
        location(path)?;
        if record.files.contains_key(path) {
            return Err("Retained file is included in cleanup".into());
        }
    }
    Ok(())
}
/// Preserve every dependency of the retained versions.
/// Takes the bound root, cleanup record and cancellation; returns success only while each retained native file still matches its exact proof.
fn verify_retained(root: &Bound, record: &Record, cancel: &AtomicBool) -> Result<(), String> {
    for (relative, expected) in &record.retained {
        let (name, file) = location(relative)?;
        let directory = root.child(name)?;
        if proof(&directory.path().join(file), cancel)? != *expected {
            return Err("Retained version dependency changed; recovery preserved".into());
        }
    }
    Ok(())
}
/// Read one private native recovery record.
/// Takes the bound recovery parent, saved identity, locked store and cancellation; returns an unchanged validated record and fingerprint.
fn read(folder: &Bound, id: &str, store: &Store, cancel: &AtomicBool) -> Result<Prepared, String> {
    if !version_id(id) {
        return Err("Choose an exact saved cleanup identity".into());
    }
    let saved = folder.child(id)?;
    let path = saved.path().join(RECORD);
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let m = file.metadata().map_err(|e| e.to_string())?;
    if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
        return Err("Cleanup record must be a private owned regular file".into());
    }
    let fingerprint = FileFingerprint::from_metadata(&m);
    let bundle: Bundle<Record> = project_file::load_from_file(file, &Limits::default(), cancel)
        .map_err(|e| e.to_string())?;
    if !bundle.media.is_empty()
        || bundle.state.id != id
        || FileFingerprint::read(&path) != Some(fingerprint)
    {
        return Err("Cleanup recovery record changed".into());
    }
    validate(&bundle.state, store)?;
    Ok(Prepared {
        record: bundle.state,
        fingerprint,
    })
}
/// Prepare recoverable version cleanup before publishing an index.
/// Takes the locked store, exact reviewed files, candidate index and cancellation; returns a durable ownership record without moving source files.
pub(super) fn prepare(
    store: &Store,
    review: &Review,
    next: &Index,
    cancel: &AtomicBool,
) -> Result<Prepared, String> {
    let (root, folder) = owned(store, true)?;
    if fs::read_dir(folder.path())
        .map_err(|e| e.to_string())?
        .take(MAX_RECORDS)
        .count()
        >= MAX_RECORDS
    {
        return Err("Cleanup recovery already contains 128 records".into());
    }
    for file in fs::read_dir(folder.path()).map_err(|e| e.to_string())? {
        let id = file
            .map_err(|e| e.to_string())?
            .file_name()
            .into_string()
            .map_err(|_| "Invalid cleanup identity")?;
        let previous = read(&folder, &id, store, cancel)?;
        let saved = folder.child(&id)?;
        if !marked(&saved, "restored.omat", &previous, cancel)?
            && !marked(&saved, "quarantined.omat", &previous, cancel)?
        {
            return Err("Restore the unfinished cleanup before starting another batch".into());
        }
    }
    if review.files.len() > MAX_FILES {
        return Err("Cleanup exceeds 65536 files".into());
    }
    let mut files = BTreeMap::new();
    let mut bytes = 0;
    for (relative, expected) in &review.files {
        let (name, file) = location(relative)?;
        let directory = root.child(name)?;
        let path = directory.path().join(file);
        if FileFingerprint::read(&path) != Some(*expected) {
            return Err("Reviewed cleanup file changed".into());
        }
        reserve(&mut bytes, expected.byte_len())?;
        files.insert(relative.clone(), proof(&path, cancel)?);
    }
    let mut retained = BTreeMap::new();
    for name in ["audio", "revisions"] {
        let directory = root.child(name)?;
        for file in fs::read_dir(directory.path()).map_err(|e| e.to_string())? {
            active(cancel)?;
            let file = file.map_err(|e| e.to_string())?;
            let basename = file
                .file_name()
                .into_string()
                .map_err(|_| "Invalid native storage name")?;
            let relative = format!("{name}/{basename}");
            location(&relative)?;
            if files.contains_key(&relative) {
                continue;
            }
            if files.len().saturating_add(retained.len()) >= MAX_FILES {
                return Err("Cleanup dependency inventory exceeds 65536 files".into());
            }
            let path = directory.path().join(basename);
            reserve(
                &mut bytes,
                FileFingerprint::read(&path)
                    .ok_or("Cannot inspect retained dependency")?
                    .byte_len(),
            )?;
            retained.insert(relative, proof(&path, cancel)?);
        }
    }
    let words = crate::engine::midi_edit::NoteId::new().words();
    let id = format!("{:016x}{:016x}", words[0], words[1]);
    let record = Record {
        schema: 1,
        id: id.clone(),
        directories: store.identity,
        before: store.index.clone(),
        after: next.clone(),
        files,
        retained,
    };
    let stage = crate::portable_project::Stage::new(&folder.path())?;
    for name in ["audio", "revisions"] {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(stage.path.join(name))
            .map_err(|e| e.to_string())?;
    }
    if !matches!(
        save(&stage.path.join(RECORD), &record, cancel, Overwrite::Never)?,
        SaveOutcome::Durable
    ) {
        return Err(
            "Cleanup recovery record durability is unconfirmed; source files preserved".into(),
        );
    }
    if !matches!(
        crate::portable_project::publish(stage, &folder.path().join(&id), cancel)?,
        SaveOutcome::Durable
    ) {
        return Err(
            "Cleanup recovery publication durability is unconfirmed; source files preserved".into(),
        );
    }
    read(&folder, &id, store, cancel)
}
/// Retire unused native files into their durable recovery folder.
/// Takes the confirmed index and prepared record; returns moved audio count, preserving partial work on any refusal for explicit recovery.
pub(super) fn retire(
    store: &Store,
    prepared: &Prepared,
    cancel: &AtomicBool,
) -> Result<usize, String> {
    let (root, folder) = owned(store, false)?;
    let saved = read(&folder, &prepared.record.id, store, cancel)?;
    if saved.fingerprint != prepared.fingerprint || !index_equal(&store.index, &saved.record.after)
    {
        return Err("Cleanup index or recovery record changed".into());
    }
    let recovery = folder.child(&saved.record.id)?;
    verify_retained(&root, &saved.record, cancel)?;
    let mut moved = 0;
    for (relative, expected) in &saved.record.files {
        active(cancel)?;
        let (name, file) = location(relative)?;
        let source = root.child(name)?;
        let target = recovery.child(name)?;
        if proof(&source.path().join(file), cancel)? != *expected {
            return Err("Unused file changed; cleanup preserved for review".into());
        }
        source.rename_new(file, &target)?;
        if proof(&target.path().join(file), cancel)? != *expected {
            return Err("Quarantined file changed; recovery preserved".into());
        }
        moved += usize::from(name == "audio");
    }
    if !matches!(
        mark(&recovery, "quarantined.omat", &saved, cancel)?,
        SaveOutcome::Durable
    ) {
        return Err(
            "Files quarantined; status durability is unconfirmed. Recovery is preserved.".into(),
        );
    }
    Ok(moved)
}
/// Inspect saved cleanup records without modifying audio or indexes.
/// Takes the locked version store and cancellation; returns bounded recovery identities and retained byte totals.
pub(crate) fn inspect(store: &Store, cancel: &AtomicBool) -> Result<Vec<Saved>, String> {
    if !store.root.join(FOLDER).exists() {
        return Ok(vec![]);
    }
    let (root, folder) = owned(store, false)?;
    let mut saved = Vec::new();
    for file in fs::read_dir(folder.path()).map_err(|e| e.to_string())? {
        active(cancel)?;
        if saved.len() >= MAX_RECORDS {
            return Err("Cleanup recovery inventory exceeds 128 records".into());
        }
        let file = file.map_err(|e| e.to_string())?;
        let id = file
            .file_name()
            .into_string()
            .map_err(|_| "Invalid cleanup record name")?;
        let prepared = read(&folder, &id, store, cancel)?;
        let recovery = folder.child(&id)?;
        let restored = marked(&recovery, "restored.omat", &prepared, cancel)?;
        let pending = !restored && !marked(&recovery, "quarantined.omat", &prepared, cancel)?;
        let record = prepared.record;
        let mut files = 0;
        let mut bytes = 0u64;
        for (relative, expected) in record.files.iter().filter(|_| !restored) {
            let (name, file) = location(relative)?;
            let directory = recovery.child(name)?;
            match fs::symlink_metadata(directory.path().join(file)) {
                Ok(_) => {
                    if proof(&directory.path().join(file), cancel)? != *expected {
                        return Err("Cleanup recovery file changed; preserved for review".into());
                    }
                    files += 1;
                    bytes = bytes
                        .checked_add(expected.bytes)
                        .ok_or("Recovery size overflow")?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let original = root.child(name)?;
                    if proof(&original.path().join(file), cancel)? != *expected {
                        return Err("Cleanup original is unavailable; recovery preserved".into());
                    }
                }
                Err(e) => return Err(e.to_string()),
            }
        }
        saved.push(Saved {
            id,
            versions: record.before.entries.len() - record.after.entries.len(),
            files,
            bytes,
            restored,
            pending,
        });
    }
    saved.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(saved)
}
/// Restore a reviewed cleanup while preserving foreign files and later index edits.
/// Takes the locked store, saved identity, cancellation and commit admission; returns the native save outcome, retaining recovery on uncertainty.
pub(crate) fn restore<G>(
    store: &mut Store,
    id: &str,
    cancel: &AtomicBool,
    authorize: impl FnOnce() -> Result<G, String>,
) -> Result<SaveOutcome, String> {
    let (root, folder) = owned(store, false)?;
    let prepared = read(&folder, id, store, cancel)?;
    let recovery = folder.child(id)?;
    if marked(&recovery, "restored.omat", &prepared, cancel)? {
        return Ok(SaveOutcome::Durable);
    }
    if !index_equal(&store.index, &prepared.record.after)
        && !index_equal(&store.index, &prepared.record.before)
    {
        return Err("Version index changed since cleanup; recovery files preserved".into());
    }
    verify_retained(&root, &prepared.record, cancel)?;
    for (relative, expected) in &prepared.record.files {
        let (name, file) = location(relative)?;
        let original_folder = root.child(name)?;
        let recovery_folder = recovery.child(name)?;
        let source = original_folder.path().join(file);
        let archived = recovery_folder.path().join(file);
        match (
            fs::symlink_metadata(&source),
            fs::symlink_metadata(&archived),
        ) {
            (Ok(_), Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                if proof(&source, cancel)? != *expected {
                    return Err("Original cleanup path changed; recovery preserved".into());
                }
            }
            (Err(e), Ok(_)) if e.kind() == std::io::ErrorKind::NotFound => {
                if proof(&archived, cancel)? != *expected {
                    return Err("Quarantined bytes changed; recovery preserved".into());
                }
            }
            _ => {
                return Err(
                    "Cleanup source missing or destination occupied; all files preserved".into(),
                )
            }
        }
    }
    active(cancel)?;
    store.recheck()?;
    let _commit = authorize()?;
    active(cancel)?;
    store.recheck()?;
    if read(&folder, id, store, cancel)?.fingerprint != prepared.fingerprint {
        return Err("Reviewed cleanup recovery changed at publication; files preserved".into());
    }
    verify_retained(&root, &prepared.record, cancel)?;
    for (relative, expected) in &prepared.record.files {
        let (name, file) = location(relative)?;
        let target = root.child(name)?;
        let source = recovery.child(name)?;
        if fs::symlink_metadata(source.path().join(file)).is_ok() {
            if proof(&source.path().join(file), cancel)? != *expected {
                return Err("Recovery source changed; files preserved".into());
            }
            source.rename_new(file, &target)?;
        }
        if proof(&target.path().join(file), cancel)? != *expected {
            return Err("Restored bytes changed; index preserved".into());
        }
    }
    for (relative, expected) in &prepared.record.files {
        let (name, file) = location(relative)?;
        let original = root.child(name)?;
        if proof(&original.path().join(file), cancel)? != *expected {
            return Err(
                "Restored file changed before index publication; recovery preserved".into(),
            );
        }
    }
    if index_equal(&store.index, &prepared.record.before) {
        return mark(&recovery, "restored.omat", &prepared, cancel);
    }
    verify_retained(&root, &prepared.record, cancel)?;
    let outcome = save(
        &root.path().join(INDEX),
        &prepared.record.before,
        cancel,
        Overwrite::Replace,
    )?;
    store.index = prepared.record.before.clone();
    store.index_fingerprint = FileFingerprint::read(&store.root.join(INDEX));
    Ok(if store.index_fingerprint.is_none() {
        SaveOutcome::CommittedButDirectorySyncFailed("Recovery index published; inspection unavailable. All restored files and recovery records are preserved.".into())
    } else if matches!(outcome, SaveOutcome::Durable) {
        match mark(&recovery, "restored.omat", &prepared, cancel) {
            Ok(outcome) => outcome,
            Err(e) => SaveOutcome::CommittedButDirectorySyncFailed(format!(
                "Recovery index restored; status deferred: {e}. Files and records preserved."
            )),
        }
    } else {
        outcome
    })
}
