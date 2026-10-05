//! One worker owns this small reference-only store. A successful rename and a
//! confirmed durable write are separate outcomes; cancellation cannot undo a
//! committed replacement or make it look like a precommit rejection.
use super::*;
use crate::engine::media_source::FileFingerprint;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Commit {
    pub durable: bool,
    pub warning: Option<String>,
    pub revision: u64,
}

pub(crate) struct Store {
    path: PathBuf,
    _lock: File,
    identity: Option<FileFingerprint>,
    pub collection: Collection,
    durable: bool,
    identity_unknown: bool,
}
impl Drop for Store {
    fn drop(&mut self) {
        // Close alone can leave an inherited open-description flock alive
        // until a concurrent fork child execs. Explicit unlock ends this
        // writer's authority before a subsequent session attempts to open.
        let _ = self._lock.unlock();
    }
}
fn identity(path: &Path) -> Result<Option<FileFingerprint>, String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(Some(FileFingerprint::from_metadata(&meta))),
        Ok(_) => Err("sampler bank path is not a regular file; preserved".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}
fn read(path: &Path) -> Result<(Collection, Vec<u8>, FileFingerprint), String> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| error.to_string())?;
    let before = file.metadata().map_err(|error| error.to_string())?;
    if !before.is_file() || before.len() > MAX_BYTES {
        return Err("sampler bank store must be a regular file no larger than 4 MiB".into());
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    Read::by_ref(&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES
        || FileFingerprint::from_metadata(&before)
            != FileFingerprint::from_metadata(&file.metadata().map_err(|error| error.to_string())?)
    {
        return Err("sampler bank store changed during reading or exceeded its bound".into());
    }
    let mut raw: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid sampler bank store; original preserved: {error}"))?;
    if raw["schema"].as_u64() == Some(1) {
        if raw.get("banks").and_then(serde_json::Value::as_array).into_iter().flatten()
            .flat_map(|bank| bank.get("slots").and_then(serde_json::Value::as_array).into_iter().flatten())
            .any(|slot| slot.get("playback").is_some()) {
            return Err("legacy sampler banks cannot contain playback modes; original preserved".into());
        }
        raw["schema"] = serde_json::json!(SCHEMA);
    }
    let value: Collection = serde_json::from_value(raw)
        .map_err(|error| format!("invalid sampler bank store; original preserved: {error}"))?;
    value.validate()?;
    Ok((value, bytes, FileFingerprint::from_metadata(&before)))
}
fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("sampler bank save cancelled before commit".into())
    } else {
        Ok(())
    }
}
impl Store {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        let parent = path.parent().ok_or("sampler bank path has no parent")?;
        if !parent.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .map_err(|error| error.to_string())?;
        }
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open(parent)
            .map_err(|error| error.to_string())?;
        let metadata = directory.metadata().map_err(|error| error.to_string())?;
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o077 != 0 {
            return Err(
                "sampler bank directory must be owned by this user and private (0700)".into(),
            );
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path.with_extension("lock"))
            .map_err(|error| error.to_string())?;
        if !lock
            .metadata()
            .map_err(|error| error.to_string())?
            .is_file()
        {
            return Err("sampler bank lock is not a regular file".into());
        }
        lock.try_lock()
            .map_err(|_| "sampler banks are already open by another writer")?;
        let (collection, current) = match identity(&path)? {
            Some(_) => {
                let (value, _, current) = read(&path)?;
                (value, Some(current))
            }
            None => {
                if path
                    .with_extension("backup.json")
                    .symlink_metadata()
                    .is_ok()
                {
                    return Err("sampler bank primary is missing; preserved backup requires explicit recovery".into());
                }
                (Collection::default(), None)
            }
        };
        Ok(Self {
            path,
            _lock: lock,
            identity: current,
            collection,
            durable: true,
            identity_unknown: false,
        })
    }
    pub fn save(&mut self, next: Collection, cancel: &AtomicBool) -> Result<Commit, String> {
        self.save_with(next, cancel, |_| Ok(()))
    }
    fn save_with(
        &mut self,
        next: Collection,
        cancel: &AtomicBool,
        mut checkpoint: impl FnMut(u8) -> Result<(), String>,
    ) -> Result<Commit, String> {
        next.validate()?;
        cancelled(cancel)?;
        if self.identity_unknown {
            return Err(
                "committed sampler bank identity is unavailable; reopen the store before saving"
                    .into(),
            );
        }
        if next.revision < self.collection.revision {
            return Err("stale reusable bank collection revision".into());
        }
        let bytes = serde_json::to_vec(&next).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err("sampler bank collection exceeds 4 MiB".into());
        }
        if identity(&self.path)? != self.identity {
            return Err("sampler bank store changed outside this writer; preserved".into());
        }
        if next == self.collection && self.identity.is_some() {
            let sync = if self.durable {
                Ok(())
            } else {
                self.sync_directory()
            };
            self.durable = sync.is_ok();
            return Ok(Commit {
                durable: self.durable,
                warning: sync.err().map(|error| {
                    format!("sampler bank remains committed; durability unconfirmed: {error}")
                }),
                revision: self.collection.revision,
            });
        }
        if self.identity.is_some() && next.revision <= self.collection.revision {
            return Err("changed sampler collection requires a newer revision".into());
        }
        let temporary = self
            .path
            .with_extension(format!("tmp-{}", std::process::id()));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)
            .map_err(|error| format!("sampler temporary write: {error}"))?;
        let owned = file.metadata().map_err(|error| error.to_string())?;
        let result = (|| {
            for chunk in bytes.chunks(64 * 1024) {
                cancelled(cancel)?;
                file.write_all(chunk).map_err(|error| error.to_string())?;
            }
            checkpoint(0)?;
            file.sync_all().map_err(|error| error.to_string())?;
            checkpoint(1)?;
            cancelled(cancel)?;
            if identity(&self.path)? != self.identity {
                return Err("sampler bank store changed during save; preserved".into());
            }
            if self.identity.is_some() {
                let original = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(&self.path)
                    .map_err(|error| error.to_string())?;
                let before = original.metadata().map_err(|error| error.to_string())?;
                if Some(FileFingerprint::from_metadata(&before)) != self.identity {
                    return Err("sampler bank store changed before backup; preserved".into());
                }
                let backup_temp = self
                    .path
                    .with_extension(format!("backup-{}", std::process::id()));
                fs::hard_link(&self.path, &backup_temp).map_err(|error| error.to_string())?;
                let backup_owned =
                    fs::symlink_metadata(&backup_temp).map_err(|error| error.to_string())?;
                let renamed = (|| {
                    checkpoint(4)?;
                    let current =
                        fs::symlink_metadata(&self.path).map_err(|error| error.to_string())?;
                    if !same_content_identity(&before, &backup_owned)
                        || !same_content_identity(&before, &current)
                    {
                        return Err(
                            "sampler bank store changed during backup; external bytes preserved"
                                .into(),
                        );
                    }
                    fs::rename(&backup_temp, self.path.with_extension("backup.json"))
                        .map_err(|error| error.to_string())
                })();
                // rename(old,new) is a no-op if both links identify the same
                // inode after a previous interrupted save. Remove only ours.
                let cleanup = remove_owned(&backup_temp, &backup_owned);
                let refresh: Result<(), String> = (|| {
                    let owned = original.metadata().map_err(|error| error.to_string())?;
                    let current =
                        fs::symlink_metadata(&self.path).map_err(|error| error.to_string())?;
                    if !same_content_identity(&before, &owned)
                        || !same_content_identity(&before, &current)
                        || FileFingerprint::from_metadata(&owned)
                            != FileFingerprint::from_metadata(&current)
                    {
                        return Err(
                            "sampler bank store changed during backup; external bytes preserved"
                                .into(),
                        );
                    }
                    // Only this same original inode's expected link-related
                    // ctime change is accepted. Never adopt a replacement.
                    self.identity = Some(FileFingerprint::from_metadata(&owned));
                    Ok(())
                })();
                renamed?;
                cleanup?;
                refresh?;
                self.sync_directory()?;
            }
            checkpoint(2)?;
            cancelled(cancel)?;
            if identity(&self.path)? != self.identity {
                return Err("sampler bank store changed before commit; preserved".into());
            }
            fs::rename(&temporary, &self.path).map_err(|error| error.to_string())?;
            // From this point cancellation no longer means rejection. Preserve
            // the actual committed state even if directory sync is uncertain.
            // Observe our descriptor, not a path another writer may already
            // have replaced. A failure here is postcommit, never rejection.
            let committed_identity = file
                .metadata()
                .map(|metadata| FileFingerprint::from_metadata(&metadata))
                .map_err(|error| format!("committed file identity unavailable: {error}"));
            self.identity = committed_identity.as_ref().ok().copied();
            self.identity_unknown = committed_identity.is_err();
            self.collection = next;
            self.durable = false;
            let sync = checkpoint(3)
                .and_then(|_| self.sync_directory())
                .and_then(|_| committed_identity.map(|_| ()));
            self.durable = sync.is_ok();
            Ok(Commit {
                durable: self.durable,
                revision: self.collection.revision,
                warning: sync.err().map(|error| {
                    format!("sampler bank replacement committed; durability unconfirmed: {error}")
                }),
            })
        })();
        let cleanup = remove_owned(&temporary, &owned);
        match (result, cleanup) {
            (Ok(mut result), Err(error)) => {
                result.warning = Some(match result.warning {
                    Some(previous) => format!("{previous}; temporary cleanup: {error}"),
                    None => format!("sampler bank committed; temporary cleanup: {error}"),
                });
                Ok(result)
            }
            (Ok(result), Ok(())) => Ok(result),
            (Err(error), Ok(())) => Err(error),
            (Err(error), Err(cleanup)) => Err(format!("{error}; temporary cleanup: {cleanup}")),
        }
    }
    fn sync_directory(&self) -> Result<(), String> {
        File::open(self.path.parent().unwrap())
            .and_then(|file| file.sync_all())
            .map_err(|error| error.to_string())
    }
}
fn same_content_identity(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    a.is_file()
        && b.is_file()
        && a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.mode() == b.mode()
        && a.uid() == b.uid()
        && a.gid() == b.gid()
}
fn remove_owned(path: &Path, owned: &fs::Metadata) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(current) if current.dev() == owned.dev() && current.ino() == owned.ino() => {
            fs::remove_file(path).map_err(|error| error.to_string())
        }
        Ok(_) => Err("sampler temporary path changed; preserved".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}
pub(crate) fn default_path() -> PathBuf {
    crate::library::default_path()
        .parent()
        .unwrap()
        .join("sampler-banks/banks.json")
}

#[cfg(test)]
mod tests;
