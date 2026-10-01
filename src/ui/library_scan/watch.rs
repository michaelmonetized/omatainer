//! One scanner-owned Linux notification descriptor, bounded recursive watches,
//! and a periodic full-scan hint for missed/new directories and offline mounts.
//! Hints never carry rows and never bypass the GUI's current admission/catalog.
use super::*;
use crate::media_location::{Failure, Location, Snapshot};
use std::io::Read;
use std::os::fd::FromRawFd;
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::time::{Duration, Instant};

pub(super) const MAX_DIRECTORIES: usize = 4096;
const MOUNT_POLL: Duration = Duration::from_secs(2);
const FALLBACK: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct Watcher {
    pending: bool,
    file: Option<std::fs::File>,
    sources: Vec<LibSource>,
    signatures: Vec<Result<(Location, (u64, u64)), Failure>>,
    performance: Handle,
    inspected: Option<Instant>,
    fallback: Option<Instant>,
}
fn signature(snapshot: &Snapshot, source: &LibSource) -> Result<(Location, (u64, u64)), Failure> {
    let location = snapshot.resolve(source)?;
    let metadata = snapshot.inspect(&location)?;
    if !metadata.is_dir() {
        return Err(Failure::Unsupported);
    }
    Ok((location, (metadata.dev(), metadata.ino())))
}
impl Watcher {
    pub fn configure(
        &mut self,
        request: &Request,
        receipt: &ScanRoots,
        inventory: &mut impl FnMut() -> Result<Snapshot, Failure>,
    ) {
        let Some(watch) = &request.watch else {
            return;
        };
        let Some(batch) = &receipt.book else {
            return;
        };
        self.file = None;
        self.sources.clear();
        self.signatures.clear();
        self.performance = request.performance.clone();
        let mut book = watch.catalog.watched_roots.clone();
        if !book.matches(batch) && book.apply(batch).is_err() {
            return;
        }
        for input in &batch.configured {
            self.sources.push(
                book.binding(&batch.profile, input)
                    .map_or_else(|| LibSource::File(input.clone()), |b| b.source.clone()),
            );
        }
        let now = Instant::now();
        self.inspected = Some(now);
        self.fallback = Some(now);
        if request.work.cancelled() {
            return;
        }
        if let Ok(snapshot) = inventory() {
            self.signatures = self
                .sources
                .iter()
                .map(|s| signature(&snapshot, s))
                .collect();
        }
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        if fd < 0 {
            return;
        }
        let file = unsafe { std::fs::File::from_raw_fd(fd) };
        let flags = libc::IN_CREATE
            | libc::IN_DELETE
            | libc::IN_CLOSE_WRITE
            | libc::IN_MOVED_FROM
            | libc::IN_MOVED_TO
            | libc::IN_ATTRIB
            | libc::IN_DELETE_SELF
            | libc::IN_MOVE_SELF
            | libc::IN_UNMOUNT
            | libc::IN_ONLYDIR
            | libc::IN_DONT_FOLLOW
            | libc::IN_EXCL_UNLINK;
        for directory in receipt.directories.iter().take(MAX_DIRECTORIES) {
            if request.work.cancelled() {
                return;
            }
            if let Ok(path) = std::ffi::CString::new(directory.as_os_str().as_bytes()) {
                // A failed watch is covered by the periodic full-scan hint.
                unsafe { libc::inotify_add_watch(fd, path.as_ptr(), flags) };
            }
        }
        self.file = Some(file);
    }
    pub fn poll(&mut self, inventory: &mut impl FnMut() -> Result<Snapshot, Failure>) -> bool {
        if self.sources.is_empty() {
            return false;
        }
        let Ok(work) = self.performance.optional_work() else {
            return false;
        };
        let mut changed = false;
        if let Some(file) = &mut self.file {
            let mut buffer = [0u8; 16 * 1024];
            for _ in 0..4 {
                match file.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => changed |= events(&buffer[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        self.file = None;
                        changed = true;
                        break;
                    }
                }
            }
        }
        let now = Instant::now();
        if self
            .inspected
            .is_some_and(|last| now.duration_since(last) >= MOUNT_POLL)
        {
            self.inspected = Some(now);
            let signatures = match inventory() {
                Ok(snapshot) => self
                    .sources
                    .iter()
                    .map(|s| signature(&snapshot, s))
                    .collect(),
                Err(error) => self.sources.iter().map(|_| Err(error.clone())).collect(),
            };
            changed |= self.signatures != signatures;
            self.signatures = signatures;
        }
        if self
            .fallback
            .is_some_and(|last| now.duration_since(last) >= FALLBACK)
        {
            self.fallback = Some(now);
            changed = true;
        }
        self.pending |= changed;
        if work.cancelled() {
            false
        } else {
            std::mem::take(&mut self.pending)
        }
    }
}

fn events(bytes: &[u8]) -> bool {
    let mut offset = 0;
    let mut changed = false;
    while offset < bytes.len() {
        let Some(header) = bytes.get(offset..offset + 16) else {
            return true;
        };
        let mask = u32::from_ne_bytes(header[4..8].try_into().unwrap());
        let len = u32::from_ne_bytes(header[12..16].try_into().unwrap()) as usize;
        let Some(end) = offset.checked_add(16).and_then(|o| o.checked_add(len)) else {
            return true;
        };
        let Some(name) = bytes.get(offset + 16..end) else {
            return true;
        };
        let name = &name[..name.iter().position(|b| *b == 0).unwrap_or(name.len())];
        let extension = name.rsplit(|b| *b == b'.').next().unwrap_or(&[]);
        let media = [
            b"wav".as_slice(),
            b"mp3",
            b"flac",
            b"ogg",
            b"aiff",
            b"aif",
            b"m4a",
            b"aac",
        ]
        .iter()
        .any(|e| extension.eq_ignore_ascii_case(e));
        changed |= media
            || mask
                & (libc::IN_ISDIR
                    | libc::IN_Q_OVERFLOW
                    | libc::IN_UNMOUNT
                    | libc::IN_DELETE_SELF
                    | libc::IN_MOVE_SELF
                    | libc::IN_IGNORED)
                != 0;
        offset = end;
    }
    changed
}
