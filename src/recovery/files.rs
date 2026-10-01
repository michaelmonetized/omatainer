use super::*;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::Path;

pub(super) fn private_dir(path: &Path, create: bool) -> Result<(), Error> {
    if create && !path.exists() {
        let parent = path
            .parent()
            .ok_or_else(|| Error::invalid("recovery directory has no parent"))?;
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| Error::io("create recovery parent", e))?;
        }
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(Error::io("create private directory", error)),
        }
    }
    let stat = fs::symlink_metadata(path).map_err(|e| Error::io("inspect private directory", e))?;
    if !stat.is_dir() || stat.uid() != unsafe { libc::geteuid() } || stat.mode() & 0o077 != 0 {
        return Err(Error::invalid(
            "recovery directories must be real, current-user-owned and private (0700)",
        ));
    }
    Ok(())
}
pub(super) fn open(path: &Path, write: bool, create: bool) -> Result<File, Error> {
    let file = OpenOptions::new()
        .read(true)
        .write(write)
        .create_new(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| Error::io("open private file", e))?;
    let stat = file
        .metadata()
        .map_err(|e| Error::io("inspect private file", e))?;
    if !stat.is_file() || stat.uid() != unsafe { libc::geteuid() } || stat.mode() & 0o077 != 0 {
        return Err(Error::invalid(
            "recovery files must be regular, current-user-owned and private (0600)",
        ));
    }
    Ok(file)
}
/// Owns a recovery transaction/session lock, without exposing clonable handles.
/// On Linux a forked child's inherited descriptor shares the flock even with
/// CLOEXEC: closing only the parent's descriptor need not release it. Explicit
/// unlock ends this owner's critical section before closing the descriptor.
pub(super) struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub(super) fn lock(path: &Path, create: bool) -> Result<Option<Lock>, Error> {
    let file = open(path, true, create)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(Lock(file))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(Error::io("lock session", error)),
    }
}
pub(super) fn sync(path: &Path) -> Result<(), Error> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|e| Error::io("sync directory", e))
}
pub(super) fn entries(
    path: &Path,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<Vec<PathBuf>, Error> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(path).map_err(|e| Error::io("list recovery directory", e))? {
        check(cancel)?;
        if paths.len() == limit {
            return Err(Error::invalid(
                "recovery directory entry limit exceeded; no files were discarded",
            ));
        }
        paths.push(
            entry
                .map_err(|e| Error::io("read recovery entry", e))?
                .path(),
        );
    }
    paths.sort();
    Ok(paths)
}
pub(super) fn usage(root: &Path, cancel: &AtomicBool) -> Result<u64, Error> {
    let mut pending = vec![(root.to_owned(), 0usize)];
    let mut seen = HashSet::new();
    let mut total = 0u64;
    let mut count = 0;
    while let Some((dir, depth)) = pending.pop() {
        private_dir(&dir, false)?;
        for path in entries(&dir, FILES_LIMIT.saturating_sub(count), cancel)? {
            count += 1;
            let stat =
                fs::symlink_metadata(&path).map_err(|e| Error::io("inspect storage usage", e))?;
            if stat.is_dir() && depth < 2 {
                pending.push((path, depth + 1));
            } else if stat.is_file()
                && stat.uid() == unsafe { libc::geteuid() }
                && stat.mode() & 0o077 == 0
            {
                if seen.insert((stat.dev(), stat.ino())) {
                    total = total
                        .checked_add(stat.len())
                        .ok_or_else(|| Error::invalid("storage usage overflow"))?;
                }
            } else {
                return Err(Error::invalid(
                    "unexpected link, directory depth or non-private file in recovery storage",
                ));
            }
        }
    }
    Ok(total)
}
pub(super) fn digest(path: &Path, limit: u64, cancel: &AtomicBool) -> Result<[u8; 32], Error> {
    let mut file = open(path, false, false)?;
    digest_file(&mut file, limit, cancel)
}
pub(super) fn digest_file(
    file: &mut File,
    limit: u64,
    cancel: &AtomicBool,
) -> Result<[u8; 32], Error> {
    file.seek(SeekFrom::Start(0))
        .map_err(|e| Error::io("rewind hash input", e))?;
    let before = file
        .metadata()
        .map_err(|e| Error::io("inspect hash input", e))?;
    if before.len() > limit {
        return Err(Error::invalid("recovery file exceeds byte limit"));
    }
    let mut hash = Sha256::new();
    let mut bytes = [0; IO_CHUNK];
    let mut total = 0u64;
    loop {
        check(cancel)?;
        let allowed = limit
            .saturating_sub(total)
            .saturating_add(1)
            .min(IO_CHUNK as u64) as usize;
        let count = file
            .read(&mut bytes[..allowed])
            .map_err(|e| Error::io("hash recovery file", e))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > limit {
            return Err(Error::invalid("recovery file grew beyond byte limit"));
        }
        hash.update(&bytes[..count]);
    }
    let after = file
        .metadata()
        .map_err(|e| Error::io("recheck hash input", e))?;
    if identity(&before) != identity(&after) || total != before.len() {
        return Err(Error::invalid("recovery file changed while being verified"));
    }
    Ok(hash.finalize().into())
}
pub(super) fn identity(m: &fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    (
        m.dev(),
        m.ino(),
        m.len(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    )
}
pub(super) fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub(super) fn valid_hash(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn write(file: &mut File, bytes: &[u8], cancel: &AtomicBool) -> Result<(), Error> {
    for chunk in bytes.chunks(IO_CHUNK) {
        check(cancel)?;
        file.write_all(chunk)
            .map_err(|e| Error::io("write recovery record", e))?;
    }
    Ok(())
}
pub(super) struct Temporary(pub PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(super) fn root_lock(root: &Path) -> Result<Lock, Error> {
    let path = root.join("storage.lock");
    let file = match open(&path, true, true) {
        Ok(file) => file,
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            open(&path, true, false)?
        }
        Err(error) => return Err(error),
    };
    match file.try_lock() {
        Ok(()) => Ok(Lock(file)),
        Err(std::fs::TryLockError::WouldBlock) => Err(Error::invalid(
            "another recovery storage transaction is active; retry after it finishes",
        )),
        Err(std::fs::TryLockError::Error(error)) => Err(Error::io("lock recovery storage", error)),
    }
}
