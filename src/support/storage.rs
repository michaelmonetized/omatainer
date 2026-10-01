//! Worker/startup-only private evidence storage. No media or recovery files are
//! read, copied, pruned or renamed here. A run lock outlives its panic-hook FD.
use super::*;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileExt, MetadataExt, OpenOptionsExt};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

const MAGIC: &[u8; 8] = b"OMASUP01";
const MARKER_BYTES: usize = 40;
const FILE_LIMIT: usize = 128;
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Published {
    Durable,
    CommittedWarning,
}
#[derive(Debug)]
pub struct Previous {
    pub report: Report,
    /// A retained report is a prior observation, not a post-mortem engine dump.
    pub last_snapshot_unix_ms: u64,
}
#[derive(Debug, Default)]
pub struct Inventory {
    pub previous: Vec<Previous>,
    /// Fixed codes only: never propagate arbitrary filenames into a bundle.
    pub skipped_active: usize,
    pub unreadable: usize,
    pub incomplete: usize,
}

pub struct Run {
    root: PathBuf,
    dir: PathBuf,
    marker: Arc<File>,
    panicked: Arc<AtomicBool>,
    pub id: Id,
    pub started_unix_ms: u64,
    pub safe_mode: bool,
}
pub fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
fn nonce() -> Id {
    // Unique application-owned identity, not a security token or device ID.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut bytes = [0; 28];
    bytes[..16].copy_from_slice(&now.to_le_bytes());
    bytes[16..20].copy_from_slice(&std::process::id().to_le_bytes());
    bytes[20..].copy_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    Id::digest(&bytes)
}
fn private_dir(path: &Path, create: bool) -> Result<(), Error> {
    if create {
        match fs::symlink_metadata(path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let parent = path
                    .parent()
                    .ok_or(Error::Invalid("private directory needs a parent"))?;
                fs::create_dir_all(parent).map_err(|e| Error::io("create parent", e))?;
                match fs::DirBuilder::new().mode(0o700).create(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(Error::io("create private directory", e)),
                }
            }
            Err(e) => return Err(Error::io("inspect directory", e)),
        }
    }
    let metadata = fs::symlink_metadata(path).map_err(|e| Error::io("inspect directory", e))?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(Error::Invalid(
            "storage must be a real user-owned private directory (0700)",
        ));
    }
    Ok(())
}
fn open(path: &Path, write: bool, create: bool) -> Result<File, Error> {
    let file = OpenOptions::new()
        .read(true)
        .write(write)
        .create_new(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .map_err(|e| Error::io("open private file", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| Error::io("inspect private file", e))?;
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(Error::Invalid(
            "storage files must be regular user-owned private files (0600)",
        ));
    }
    Ok(file)
}
fn try_lock(file: &File) -> Result<bool, Error> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(std::fs::TryLockError::WouldBlock) => Ok(false),
        Err(std::fs::TryLockError::Error(e)) => Err(Error::io("lock evidence", e)),
    }
}
fn root_lock(root: &Path) -> Result<File, Error> {
    let path = root.join("storage.lock");
    let file = match open(&path, true, true) {
        Ok(file) => file,
        Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            open(&path, true, false)?
        }
        Err(error) => return Err(error),
    };
    if !try_lock(&file)? {
        return Err(Error::Busy);
    }
    Ok(file)
}
fn sync(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}
fn entries(root: &Path) -> Result<Vec<PathBuf>, Error> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| Error::io("list evidence", e))? {
        if entries.len() == FILE_LIMIT {
            return Err(Error::Invalid(
                "evidence entry bound exceeded; files preserved",
            ));
        }
        entries.push(
            entry
                .map_err(|e| Error::io("read evidence entry", e))?
                .path(),
        );
    }
    Ok(entries)
}
fn named_id(path: &Path, prefix: &str) -> Option<Id> {
    Id::from_hex(path.file_name()?.to_str()?.strip_prefix(prefix)?)
}
fn run_id(path: &Path) -> Option<Id> {
    named_id(path, "run-")
}
fn owned_directory(path: &Path) -> bool {
    ["run-", ".creating-", ".retired-"]
        .iter()
        .any(|p| named_id(path, p).is_some())
}
fn known_file(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n == "marker" || n == "report.json")
        || named_id(path, ".pending-").is_some()
}
/// Cleanup namespaces are entered only by an atomic rename under the root
/// mutation lock. Every interruption leaves either the original run or a
/// resumable cleanup directory; no partial run is advertised as recoverable.
fn clean_directory(dir: &Path) -> Result<(), Error> {
    private_dir(dir, false)?;
    let files = entries(dir)?;
    for path in &files {
        if !known_file(path) {
            return Err(Error::Invalid(
                "unknown retained evidence file; files preserved",
            ));
        }
        let _ = open(path, false, false)?;
    }
    for path in files {
        fs::remove_file(&path).map_err(|e| Error::io("prune evidence", e))?;
    }
    fs::remove_dir(dir).map_err(|e| Error::io("prune evidence directory", e))?;
    Ok(())
}
fn finish_cleanup(root: &Path) -> Result<(), Error> {
    for dir in entries(root)? {
        if named_id(&dir, ".creating-").is_some() || named_id(&dir, ".retired-").is_some() {
            clean_directory(&dir)?;
        }
    }
    Ok(())
}
fn usage(root: &Path) -> Result<u64, Error> {
    let mut total = 0u64;
    for path in entries(root)? {
        let metadata =
            fs::symlink_metadata(&path).map_err(|e| Error::io("inspect evidence size", e))?;
        if owned_directory(&path) {
            private_dir(&path, false)?;
            for child in entries(&path)? {
                let file = open(&child, false, false)?;
                total = total
                    .checked_add(
                        file.metadata()
                            .map_err(|e| Error::io("inspect evidence size", e))?
                            .len(),
                    )
                    .ok_or(Error::Invalid("evidence size overflow"))?;
            }
        } else if metadata.is_file() && path.file_name().is_some_and(|n| n == "storage.lock") {
            total = total.saturating_add(metadata.len());
        } else {
            return Err(Error::Invalid("unknown evidence entry; files preserved"));
        }
    }
    Ok(total)
}
fn parse_marker(file: &File, id: Id) -> Result<(Exit, u64, bool), Error> {
    if file
        .metadata()
        .map_err(|e| Error::io("inspect marker", e))?
        .len()
        != MARKER_BYTES as u64
    {
        return Err(Error::Invalid("incomplete crash marker"));
    }
    let mut bytes = [0; MARKER_BYTES];
    file.read_exact_at(&mut bytes, 0)
        .map_err(|e| Error::io("read marker", e))?;
    if &bytes[..8] != MAGIC || bytes[9] > 1 || bytes[10..16] != [0; 6] || bytes[16..32] != id.0 {
        return Err(Error::Invalid("unrecognized crash marker"));
    }
    let exit = match bytes[8] {
        0 => Exit::Unclean,
        1 => Exit::Clean,
        2 => Exit::ObservedRustPanic,
        3 => Exit::StartupFailed,
        _ => return Err(Error::Invalid("unrecognized exit marker")),
    };
    Ok((
        exit,
        u64::from_le_bytes(bytes[32..40].try_into().unwrap()),
        bytes[9] != 0,
    ))
}

impl Run {
    pub fn begin(root: &Path, safe_mode: bool) -> Result<Self, Error> {
        private_dir(root, true)?;
        let _guard = root_lock(root)?;
        finish_cleanup(root)?;
        let mut candidates = Vec::new();
        for dir in entries(root)? {
            if let Some(id) = run_id(&dir) {
                private_dir(&dir, false)?;
                let marker = open(&dir.join("marker"), true, false)?;
                if try_lock(&marker)? {
                    if let Ok((_, started, _)) = parse_marker(&marker, id) {
                        candidates.push((started, dir, marker));
                    }
                }
            }
        }
        candidates.sort_by_key(|(started, _, _)| *started);
        // Pruning touches only inactive support evidence, never journals or
        // explicit saves. Corrupt/unrecognized directories are preserved.
        let run_count = entries(root)?
            .iter()
            .filter(|p| run_id(p).is_some())
            .count();
        let prune = run_count.saturating_add(1).saturating_sub(MAX_RUNS);
        if prune > candidates.len() {
            return Err(Error::Busy);
        }
        for (_, dir, _marker) in candidates.iter().take(prune) {
            for path in entries(dir)? {
                if !known_file(&path) {
                    return Err(Error::Invalid(
                        "unknown retained evidence file; files preserved",
                    ));
                }
                let _ = open(&path, false, false)?;
            }
            let retired = root.join(format!(".retired-{}", run_id(dir).unwrap().hex()));
            fs::rename(dir, &retired).map_err(|e| Error::io("retire old evidence", e))?;
            sync(root).map_err(|e| Error::io("commit retired evidence", e))?;
            clean_directory(&retired)?;
        }
        if usage(root)?.saturating_add(MARKER_BYTES as u64) > MAX_STORAGE_BYTES {
            return Err(Error::Invalid("support storage cap reached"));
        }
        let id = nonce();
        let mut dir = root.join(format!(".creating-{}", id.hex()));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| Error::io("create run", e))?;
        let marker = open(&dir.join("marker"), true, true)?;
        if !try_lock(&marker)? {
            return Err(Error::Busy);
        }
        let started_unix_ms = unix_ms();
        let mut bytes = [0; MARKER_BYTES];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[9] = u8::from(safe_mode);
        bytes[16..32].copy_from_slice(&id.0);
        bytes[32..].copy_from_slice(&started_unix_ms.to_le_bytes());
        marker
            .write_all_at(&bytes, 0)
            .map_err(|e| Error::io("write initial marker", e))?;
        marker
            .sync_all()
            .and_then(|_| sync(&dir))
            .map_err(|e| Error::io("commit initial marker", e))?;
        let published = root.join(format!("run-{}", id.hex()));
        fs::rename(&dir, &published).map_err(|e| Error::io("publish initial marker", e))?;
        dir = published;
        sync(root).map_err(|e| Error::io("commit initial marker directory", e))?;
        Ok(Self {
            root: root.into(),
            dir,
            marker: Arc::new(marker),
            panicked: Arc::new(AtomicBool::new(false)),
            id,
            started_unix_ms,
            safe_mode,
        })
    }
    pub fn report(&self) -> Report {
        Report::new(self.id, self.safe_mode, self.started_unix_ms)
    }
    /// One process-wide hook, installed only during native application startup.
    /// No payload, backtrace, heap allocation, locks, fsync or signal handler.
    /// pwrite is best effort: disk failure/abrupt signals may leave only Unclean.
    pub fn install_panic_hook(&self) {
        let file = self.marker.clone();
        let observed = self.panicked.clone();
        std::panic::set_hook(Box::new(move |_| {
            observed.store(true, Ordering::Release);
            let byte = [2u8];
            unsafe {
                libc::pwrite(file.as_raw_fd(), byte.as_ptr().cast(), 1, 8);
            }
        }));
    }
    pub fn mark_clean(&self) -> Result<(), Error> {
        self.finish(1)
    }
    pub fn mark_startup_failed(&self) -> Result<(), Error> {
        self.finish(3)
    }
    fn finish(&self, value: u8) -> Result<(), Error> {
        if self.panicked.load(Ordering::Acquire) {
            return Ok(());
        }
        self.marker
            .write_all_at(&[value], 8)
            .and_then(|_| self.marker.sync_all())
            .map_err(|e| Error::io("commit exit marker", e))
    }
    pub fn persist(&self, report: &Report, cancel: &AtomicBool) -> Result<Published, Error> {
        if report.run != self.id
            || report.started_unix_ms != self.started_unix_ms
            || report.safe_mode != self.safe_mode
        {
            return Err(Error::Invalid("report belongs to a different run"));
        }
        let bytes = encode(report)?;
        check(cancel)?;
        let _guard = root_lock(&self.root)?;
        // Space for old committed data AND staging counts toward the cap.
        if usage(&self.root)?.saturating_add(bytes.len() as u64) > MAX_STORAGE_BYTES {
            return Err(Error::Invalid(
                "support storage cap reached; previous report preserved",
            ));
        }
        publish(&self.dir.join("report.json"), &bytes, true, cancel)
    }
}
pub fn discover(root: &Path, cancel: &AtomicBool) -> Result<Inventory, Error> {
    match fs::symlink_metadata(root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Inventory::default()),
        _ => {}
    }
    private_dir(root, false)?;
    let mut result = Inventory::default();
    for dir in entries(root)? {
        check(cancel)?;
        let Some(id) = run_id(&dir) else { continue };
        let parsed = (|| {
            private_dir(&dir, false)?;
            let marker = open(&dir.join("marker"), true, false)?;
            if !try_lock(&marker)? {
                return Ok(None);
            }
            let (exit, started, safe_mode) = parse_marker(&marker, id)?;
            let mut report = match reopen(&dir.join("report.json"), cancel) {
                Ok(report)
                    if report.run == id
                        && report.started_unix_ms == started
                        && report.safe_mode == safe_mode =>
                {
                    report
                }
                Ok(_) => return Err(Error::Invalid("stored report identity mismatch")),
                Err(Error::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
                    result.incomplete += 1;
                    let mut report = Report::new(id, safe_mode, started);
                    // No report survived: the executable now reading the marker
                    // is not evidence of the crashed executable's build.
                    report.build = None;
                    report
                }
                Err(error) => return Err(error),
            };
            report.exit = exit;
            Ok(Some(Previous {
                last_snapshot_unix_ms: report.collected_unix_ms,
                report,
            }))
        })();
        match parsed {
            Ok(Some(previous)) => result.previous.push(previous),
            Ok(None) => result.skipped_active += 1,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            Err(_) => result.unreadable += 1,
        }
    }
    result
        .previous
        .sort_by_key(|p| std::cmp::Reverse(p.report.started_unix_ms));
    if result.previous.len() > MAX_RUNS {
        result.incomplete += result.previous.len() - MAX_RUNS;
        result.previous.truncate(MAX_RUNS);
    }
    Ok(result)
}
pub fn encode(report: &Report) -> Result<Vec<u8>, Error> {
    report.validate()?;
    let bytes = serde_json::to_vec_pretty(report)
        .map_err(|_| Error::Invalid("report cannot be encoded"))?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::Invalid("report exceeds 4 MiB"));
    }
    Ok(bytes)
}
pub fn export(path: &Path, report: &Report, cancel: &AtomicBool) -> Result<Published, Error> {
    let bytes = encode(report)?;
    publish(path, &bytes, false, cancel)
}
fn publish(
    path: &Path,
    bytes: &[u8],
    replace: bool,
    cancel: &AtomicBool,
) -> Result<Published, Error> {
    check(cancel)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let temporary = parent.join(format!(".pending-{}", nonce().hex()));
    let mut file = open(&temporary, true, true)?;
    let result = (|| {
        for chunk in bytes.chunks(4096) {
            check(cancel)?;
            file.write_all(chunk)
                .map_err(|e| Error::io("write report", e))?;
        }
        file.sync_all().map_err(|e| Error::io("sync report", e))?;
        check(cancel)?;
        if replace {
            match fs::symlink_metadata(path) {
                Ok(_) => {
                    let _ = open(path, false, false)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(Error::io("inspect destination", e)),
            }
            fs::rename(&temporary, path).map_err(|e| Error::io("publish report", e))?;
        } else {
            fs::hard_link(&temporary, path).map_err(|e| Error::io("publish report", e))?;
        }
        // Publication wins late cancellation. Sync failure is committed with
        // warning, never a false precommit failure or an instruction to retry.
        Ok(if sync(parent).is_ok() {
            Published::Durable
        } else {
            Published::CommittedWarning
        })
    })();
    drop(file);
    let _ = fs::remove_file(&temporary);
    result
}
pub fn reopen(path: &Path, cancel: &AtomicBool) -> Result<Report, Error> {
    check(cancel)?;
    let mut file = open(path, false, false)?;
    if file
        .metadata()
        .map_err(|e| Error::io("inspect report", e))?
        .len()
        > MAX_BYTES as u64
    {
        return Err(Error::Invalid("report exceeds 4 MiB"));
    }
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        check(cancel)?;
        let count = file
            .read(&mut chunk)
            .map_err(|e| Error::io("read report", e))?;
        if count == 0 {
            break;
        }
        if bytes.len() + count > MAX_BYTES {
            return Err(Error::Invalid("report grew beyond 4 MiB"));
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    let report: Report = serde_json::from_slice(&bytes)
        .map_err(|_| Error::Invalid("malformed or unsupported report"))?;
    report.validate()?;
    check(cancel)?;
    Ok(report)
}
