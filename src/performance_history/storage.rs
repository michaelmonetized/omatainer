//! Bounded private history files. All calls belong to the history worker.
use super::{Session, valid_id};
use std::{collections::BTreeMap, fs::{self, File, OpenOptions}, io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt}, path::{Path, PathBuf}};

pub(crate) const MAX_SESSIONS: usize = 1024;
pub(crate) const MAX_SESSION_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MAX_ROOT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Revision { dev: u64, ino: u64, len: u64, modified: (i64, i64), changed: (i64, i64) }
impl Revision {
    fn of(metadata: &fs::Metadata) -> Self { Self { dev: metadata.dev(), ino: metadata.ino(), len: metadata.len(),
        modified: (metadata.mtime(), metadata.mtime_nsec()), changed: (metadata.ctime(), metadata.ctime_nsec()) } }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Commit { Durable, CommittedUnconfirmed(String) }
pub(crate) struct Store {
    root: PathBuf,
    lock: File,
    pub sessions: BTreeMap<String, Session>,
    revisions: BTreeMap<String, Revision>,
    total_bytes: u64,
    poisoned: bool,
}
impl Drop for Store { fn drop(&mut self) { let _ = self.lock.unlock(); } }

fn private_file(metadata: &fs::Metadata) -> Result<(), String> {
    if !metadata.is_file() || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0 || metadata.nlink() != 1
    { Err("history requires a private regular file owned by this user".into()) } else { Ok(()) }
}
fn revision(path: &Path) -> Result<Option<Revision>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => { private_file(&metadata)?; Ok(Some(Revision::of(&metadata))) },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}
fn read(path: &Path) -> Result<(Session, Revision), String> {
    let mut file = OpenOptions::new().read(true).custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?; private_file(&metadata)?;
    if metadata.len() > MAX_SESSION_BYTES { return Err("history session exceeds 16 MiB".into()); }
    let before = Revision::of(&metadata);
    let mut bytes = Vec::new();
    (&mut file).take(MAX_SESSION_BYTES + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_SESSION_BYTES || Revision::of(&file.metadata().map_err(|e| e.to_string())?) != before
        || revision(path)?.as_ref() != Some(&before)
    { return Err("history changed while reading; original files preserved".into()); }
    let session: Session = serde_json::from_slice(&bytes).map_err(|e| format!("invalid history; original file preserved: {e}"))?;
    session.validate()?; Ok((session, before))
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        if !root.is_absolute() { return Err("history needs an absolute directory".into()); }
        fs::DirBuilder::new().recursive(true).mode(0o700).create(&root).map_err(|e| e.to_string())?;
        let metadata = fs::symlink_metadata(&root).map_err(|e| e.to_string())?;
        if !metadata.is_dir() || metadata.uid() != unsafe { libc::geteuid() } || metadata.permissions().mode() & 0o077 != 0 {
            return Err("history directory must be private, owned by this user and not a symlink".into());
        }
        let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true).mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(root.join(".lock")).map_err(|e| e.to_string())?;
        private_file(&lock.metadata().map_err(|e| e.to_string())?)?;
        lock.try_lock().map_err(|_| "history is already open by another writer")?;
        let mut store = Self { root, lock, sessions: BTreeMap::new(), revisions: BTreeMap::new(), total_bytes: 0, poisoned: false };
        let mut entries = 0;
        for item in fs::read_dir(&store.root).map_err(|e| e.to_string())? {
            entries += 1;
            if entries > MAX_SESSIONS + 128 { return Err("history directory entry limit reached; files preserved".into()); }
            let item = item.map_err(|e| e.to_string())?;
            let name = item.file_name(); let name = name.to_str().ok_or("invalid history filename")?;
            if name == ".lock" { continue; }
            let metadata = fs::symlink_metadata(item.path()).map_err(|e| e.to_string())?; private_file(&metadata)?;
            store.total_bytes = store.total_bytes.checked_add(metadata.len()).ok_or("history storage total overflow")?;
            if store.total_bytes > MAX_ROOT_BYTES { return Err("history exceeds its 256 MiB storage cap; files preserved".into()); }
            // Crash-left private staging files count toward the storage bound
            // and remain untouched. They are never promoted into a session.
            if name.starts_with(".history-") && name.ends_with(".tmp") { continue; }
            let id = name.strip_suffix(".json").ok_or("unexpected file in history directory; preserved")?;
            valid_id(id)?;
            if store.sessions.len() >= MAX_SESSIONS { return Err("history reached its 1024-session limit".into()); }
            let (mut session, revision) = read(&item.path())?;
            if session.id != id { return Err("history filename and session identity disagree".into()); }
            session.recover_unclean();
            store.sessions.insert(id.to_owned(), session); store.revisions.insert(id.to_owned(), revision);
        }
        Ok(store)
    }
    pub fn save(&mut self, session: &Session) -> Result<Commit, String> { self.save_checked(session, None, |_| Ok(())) }
    pub fn save_optional(&mut self, session: &Session, permit: &crate::engine::performance::WorkPermit) -> Result<Commit, String> { self.save_checked(session, Some(permit), |_| Ok(())) }
    fn save_checked(&mut self, session: &Session, permit: Option<&crate::engine::performance::WorkPermit>, mut checkpoint: impl FnMut(u8) -> Result<(), String>) -> Result<Commit, String> {
        if self.poisoned { return Err("history commit identity is unknown; reopen before another save".into()); }
        session.validate()?;
        if !self.revisions.contains_key(&session.id) && self.revisions.len() >= MAX_SESSIONS { return Err("history reached its 1024-session limit".into()); }
        let bytes = serde_json::to_vec(session).map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_SESSION_BYTES { return Err("history session exceeds 16 MiB; previous save preserved".into()); }
        let old = self.revisions.get(&session.id).cloned();
        let total = self.total_bytes.checked_sub(old.as_ref().map_or(0, |r| r.len))
            .and_then(|size| size.checked_add(bytes.len() as u64)).ok_or("history storage total overflow")?;
        if total > MAX_ROOT_BYTES || self.total_bytes.checked_add(bytes.len() as u64).is_none_or(|staged| staged > MAX_ROOT_BYTES) {
            return Err("history needs staging space within its 256 MiB cap; no sessions were deleted".into());
        }
        let path = self.root.join(format!("{}.json", session.id));
        if revision(&path)? != old { return Err("history changed on disk; reload before saving".into()); }
        let (temp, mut file) = staging(&self.root)?;
        file.write_all(&bytes).map_err(|e| e.to_string())?; file.sync_all().map_err(|e| e.to_string())?;
        checkpoint(0)?;
        let _commit = permit.map(|permit| permit.commit().map_err(|e| e.to_string())).transpose()?;
        if revision(&path)? != old { return Err("history changed before publication; old file preserved".into()); }
        if old.is_some() { fs::rename(&temp.0, &path).map_err(|e| e.to_string())?; }
        else { fs::hard_link(&temp.0, &path).map_err(|e| e.to_string())?; }
        // Publication has happened. Later failures are warnings, not rejected
        // edits that the user might accidentally duplicate by retrying them.
        let cleanup = if old.is_none() { fs::remove_file(&temp.0).err().map(|e| e.to_string()) } else { None };
        self.sessions.insert(session.id.clone(), session.clone()); self.total_bytes = total;
        let identity = file.metadata().map(|m| Revision::of(&m)).map_err(|e| e.to_string());
        match identity {
            Ok(revision) => { self.revisions.insert(session.id.clone(), revision); },
            Err(error) => { self.poisoned = true; return Ok(Commit::CommittedUnconfirmed(error)); },
        }
        if let Some(error) = cleanup { return Ok(Commit::CommittedUnconfirmed(error)); }
        if let Err(error) = checkpoint(1).and_then(|_| File::open(&self.root).and_then(|dir| dir.sync_all()).map_err(|e| e.to_string())) {
            return Ok(Commit::CommittedUnconfirmed(error));
        }
        Ok(Commit::Durable)
    }
}

struct Temp(PathBuf);
impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_file(&self.0); } }
pub(crate) fn new_id() -> Result<String, String> {
    let mut bytes = [0_u8; 16]; File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut bytes)).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn staging(parent: &Path) -> Result<(Temp, File), String> {
    let temp = Temp(parent.join(format!(".history-{}.tmp", new_id()?)));
    let file = OpenOptions::new().create_new(true).write(true).mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK).open(&temp.0).map_err(|e| e.to_string())?;
    Ok((temp, file))
}
impl Store {
    pub fn export(&self, session: &Session, path: &Path, permit: &crate::engine::performance::WorkPermit) -> Result<Commit, String> { self.export_as(session,path,super::export::Format::Json,false,None,permit) }
    /// Publish a selected session as a new private setlist.
    /// Takes the session, destination, format, location consent, catalog and work permit; returns durable publication or a committed durability warning.
    pub fn export_as(&self, session: &Session, path: &Path, format: super::export::Format, locations: bool, catalog: Option<&crate::library::Catalog>, permit: &crate::engine::performance::WorkPermit) -> Result<Commit, String> {
        if !path.is_absolute() { return Err("choose an absolute setlist export path".into()); }
        let parent = path.parent().ok_or("export path has no parent")?;
        if parent.canonicalize().map_err(|e|e.to_string())?.starts_with(self.root.canonicalize().map_err(|e|e.to_string())?) { return Err("export outside the managed history directory".into()); }
        let bytes = super::export::encode(session, format, locations, catalog, permit)?;
        export_bytes(&bytes, path, permit)
    }
}
fn export_bytes(bytes: &[u8], path: &Path, permit: &crate::engine::performance::WorkPermit) -> Result<Commit, String> {
    let parent = path.parent().ok_or("export path has no parent")?;
    if bytes.len() as u64 > MAX_SESSION_BYTES { return Err("history export exceeds 16 MiB".into()); }
    let (temp, mut file) = staging(parent)?;
    file.write_all(bytes).map_err(|e| e.to_string())?; file.sync_all().map_err(|e| e.to_string())?;
    let _commit = permit.commit().map_err(|e| e.to_string())?;
    // hard_link atomically refuses any existing destination, including symlinks.
    fs::hard_link(&temp.0, path).map_err(|e| e.to_string())?;
    if let Err(error) = fs::remove_file(&temp.0).and_then(|_| File::open(parent)?.sync_all()) {
        return Ok(Commit::CommittedUnconfirmed(error.to_string()));
    }
    Ok(Commit::Durable)
}

#[cfg(test)]
mod tests;
