use crate::{
    engine::media_source::{FileFingerprint, LibSource},
    media_location::Snapshot,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashSet, VecDeque},
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawFd, FromRawFd},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const MAX_REQUEST: usize = 2 * 1024 * 1024;
const MAX_RESPONSE: usize = 8 * 1024 * 1024;
const PAGE_ENTRIES: usize = 10_000;
const MAX_PENDING: usize = 4096;
const MAX_CANDIDATES: usize = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Purpose { #[default] Dj, Producer }

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    #[serde(default)]
    pub purpose: Purpose,
    pub roots: Vec<PathBuf>,
    pub all_mounts: bool,
    pub cursor: Option<Cursor>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Directory {
    path: PathBuf,
    offset: usize,
    depth: u8,
    fingerprint: Option<FileFingerprint>,
    #[serde(default)]
    restarts: u8,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cursor {
    #[serde(default)]
    purpose: Purpose,
    mount_digest: [u8; 32],
    pending: VecDeque<Directory>,
    seen: HashSet<(u64, u64)>,
    pub visited: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Candidate {
    pub path: PathBuf,
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
    pub format: String,
    pub reviewable: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Notice {
    pub path: PathBuf,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Page {
    pub candidates: Vec<Candidate>,
    pub notices: Vec<Notice>,
    pub cursor: Option<Cursor>,
    pub visited: usize,
    pub elapsed_millis: u64,
    pub complete: bool,
}

fn valid_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .to_str()
            .is_some_and(|p| p.len() <= 4096 && !p.chars().any(char::is_control))
        && !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
}
fn excluded(path: &Path) -> bool {
    ["/proc", "/sys", "/dev", "/run", "/tmp", "/var/tmp"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
        && !path.starts_with("/run/media")
        || path.components().any(|part| {
            matches!(
                part.as_os_str().to_str(),
                Some(".git" | "node_modules" | "target" | ".cache")
            )
        })
}
fn notice(notices: &mut Vec<Notice>, path: &Path, state: impl Into<String>) {
    if notices.len() < 127 {
        notices.push(Notice {
            path: if valid_path(path) {
                path.into()
            } else {
                "/".into()
            },
            state: state.into().chars().take(512).collect(),
        });
    } else if notices.len() == 127 {
        notices.push(Notice {path:"/".into(),state:"Additional exclusions and failures are omitted after 127 notices in this page; narrow the search roots to inspect them.".into()});
    }
}
fn mount_bytes() -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    File::open("/proc/self/mountinfo")
        .and_then(|f| f.take((MAX_REQUEST + 1) as u64).read_to_end(&mut bytes))
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_REQUEST {
        return Err("Mount inventory exceeds 2 MiB".into());
    }
    Ok(bytes)
}
fn pending_bytes(cursor: &Cursor) -> usize {
    cursor
        .pending
        .iter()
        .map(|d| d.path.as_os_str().len() + std::mem::size_of::<Directory>() + 32)
        .sum::<usize>()
        + cursor.seen.len() * 48
}

impl Request {
    /// Validate a resumable discovery request. Takes custom/home roots and its cursor; refuses oversized or untrusted traversal state before filesystem access.
    pub fn validate(&self) -> Result<(), String> {
        if self.roots.len() > 64 || self.roots.iter().any(|p| !valid_path(p)) {
            return Err("DJ discovery accepts at most 64 absolute UTF-8 roots".into());
        }
        if let Some(cursor) = &self.cursor {
            if cursor.pending.len() > MAX_PENDING
                || cursor.seen.len() > 65536
                || pending_bytes(cursor) > 1024 * 1024
                || cursor
                    .pending
                    .iter()
                    .any(|d| !valid_path(&d.path) || d.depth > 64 || d.offset > 1_000_000 || d.restarts > 2)
            {
                return Err("DJ discovery cursor exceeds its path, memory or depth bounds".into());
            }
        }
        Ok(())
    }
}

fn probe(path: &Path, snapshot: &Snapshot, purpose: Purpose) -> Result<Option<Candidate>, String> {
    if purpose == Purpose::Producer { return crate::producer_library::probe(path, snapshot); }
    let extension = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let basename = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
    if matches!(extension.as_str(),"wav"|"wave"|"aif"|"aiff"|"flac"|"mp3"|"m4a"|"aac"|"ogg"|"opus"|"wma"|"mp4"|"mov"|"mkv"|"jpg"|"jpeg"|"png") {
        return Ok(None);
    }
    let vendor_db = matches!(
        basename,
        "database V2"
            | "master.db"
            | "master.backup.db"
            | "export.pdb"
            | "exportLibrary.db"
            | "library.db"
    );
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Ok(None);
    }
    if metadata.len() > 32 * 1024 * 1024 {
        return if vendor_db
            || matches!(extension.as_str(), "xml" | "nml" | "crate" | "m3u" | "m3u8")
        {
            Err("Source exceeds the 32 MiB import limit; export a smaller selection".into())
        } else {
            Ok(None)
        };
    }
    let fingerprint = FileFingerprint::from_metadata(&metadata);
    let mut prefix = [0u8; 8192];
    let count = file.read(&mut prefix).map_err(|e| e.to_string())?;
    let prefix = &prefix[..count];
    let text = if prefix.starts_with(&[0xff,0xfe]) || prefix.starts_with(&[0xfe,0xff]) {
        let little = prefix[0] == 0xff;
        std::borrow::Cow::Owned(String::from_utf16_lossy(&prefix[2..].chunks_exact(2).map(|b|if little{u16::from_le_bytes([b[0],b[1]])}else{u16::from_be_bytes([b[0],b[1]])}).collect::<Vec<_>>()))
    } else { String::from_utf8_lossy(prefix) };
    let (format, reviewable) = if prefix.starts_with(b"vrsn") {
        ("Serato crate (version checked during review)", true)
    } else if text.contains("<DJ_PLAYLISTS") {
        ("rekordbox XML (version checked during review)", true)
    } else if text.contains("<NML") {
        ("Traktor NML (version checked during review)", true)
    } else if text.contains("<plist") && text.contains("Tracks") {
        ("Apple XML candidate (content checked during review)", true)
    } else if (text.trim_start_matches('\u{feff}').starts_with("#EXTM3U")
        || matches!(extension.as_str(), "m3u" | "m3u8"))
        && std::str::from_utf8(prefix).map_or_else(|error| error.error_len().is_none(), |_| true)
    {
        ("UTF-8 M3U", true)
    } else if vendor_db {
        (
            "Unsupported vendor database; export M3U, rekordbox XML or Traktor NML 19",
            false,
        )
    } else if matches!(extension.as_str(), "nml" | "crate") {
        (
            "Unknown/corrupt DJ format; review the source version or export M3U",
            false,
        )
    } else {
        return Ok(None);
    };
    let location = snapshot.identify(path).map_err(|e| e.to_string())?;
    location
        .verify_file(&file, fingerprint)
        .map_err(|e| e.to_string())?;
    Ok(Some(Candidate {
        path: location.path,
        source: location.source,
        fingerprint,
        format: format.into(),
        reviewable,
    }))
}

/// Discover one bounded page of DJ library sources.
/// Takes custom roots, retained traversal state and cancellation predicate; returns source candidates, explicit exclusions/failures and resumable progress without changing any catalog or source file.
pub(crate) fn scan(request: Request, active: &impl Fn() -> bool) -> Result<Page, String> {
    scan_with_watch(request, active, None)
}
fn scan_with_watch(
    request: Request,
    active: &impl Fn() -> bool,
    watch: Option<i32>,
) -> Result<Page, String> {
    request.validate()?;
    let started = Instant::now();
    if !active() {
        return Err("DJ discovery cancelled".into());
    }
    let mounts = mount_bytes()?;
    let digest = Sha256::digest(&mounts).into();
    let purpose = request.purpose;
    let explicit = request.roots.clone();
    let excluded = |path: &Path| {
        let virtual_tree = ["/proc", "/sys", "/dev", "/run", "/tmp", "/var/tmp"]
            .iter()
            .any(|p| path.starts_with(p))
            && !path.starts_with("/run/media");
        virtual_tree
            || excluded(path)
                && !explicit.iter().any(|root| {
                    excluded(root)
                        && !["/proc", "/sys", "/dev", "/run", "/tmp", "/var/tmp"]
                            .iter()
                            .any(|p| root.starts_with(p))
                        && path.starts_with(root)
                })
    };
    let snapshot = Snapshot::discover().map_err(|e| e.to_string())?;
    let mut notices = Vec::new();
    let mut cursor = if let Some(cursor) = request.cursor {
        if cursor.mount_digest != digest || cursor.purpose != purpose {
            return Err(
                "Mounts changed during discovery; restart with the current volume inventory".into(),
            );
        }
        cursor
    } else {
        let mut roots = request.roots;
        if request.all_mounts {
            for known in snapshot.mountpoints() {
                if roots.len() >= 256 {
                    notice(
                        &mut notices,
                        &known,
                        "Mount root inventory exceeds 256; add an explicit custom root",
                    );
                    break;
                }
                if !roots.contains(&known) {
                    roots.push(known);
                }
            }
            if !roots.iter().any(|r| r == Path::new("/")) {
                roots.push("/".into());
            }
        }
        let mut pending = VecDeque::new();
        for root in roots {
            if excluded(&root) {
                notice(&mut notices,&root,"Excluded virtual, runtime, build or cache tree; choose its exact library subdirectory explicitly to review a source file");
                continue;
            }
                for known in [
                    root.join("Music/_Serato_/Subcrates"),
                    root.join("_Serato_/Subcrates"),
                    root.join("Documents/Native Instruments"),
                ] {
                    pending.push_back(Directory {
                        path: known,
                        offset: 0,
                        depth: 0,
                        fingerprint: None,
                        restarts: 0,
                    });
                }
            pending.push_back(Directory {
                path: root,
                offset: 0,
                depth: 0,
                fingerprint: None,
                restarts: 0,
            });
        }
        Cursor {
            purpose,
            mount_digest: digest,
            pending,
            seen: HashSet::new(),
            visited: 0,
        }
    };
    let mut candidates = Vec::new();
    let mut page_entries = 0;
    while let Some(mut directory) = cursor.pending.pop_front() {
        if !active() {
            return Err("DJ discovery cancelled".into());
        }
        if page_entries >= PAGE_ENTRIES || candidates.len() >= MAX_CANDIDATES || started.elapsed() >= Duration::from_secs(3) {
            cursor.pending.push_front(directory);
            break;
        }
        let metadata = match std::fs::symlink_metadata(&directory.path) {
            Ok(m) => m,
            Err(e) => {
                notice(
                    &mut notices,
                    &directory.path,
                    format!("Unavailable/unreadable root: {e}"),
                );
                continue;
            }
        };
        if metadata.is_symlink() || !metadata.is_dir() {
            notice(
                &mut notices,
                &directory.path,
                "Symlink/non-directory root excluded",
            );
            continue;
        }
        let fingerprint = FileFingerprint::from_metadata(&metadata);
        if directory.fingerprint.is_some_and(|fp| fp != fingerprint) {
            retry_directory(directory, &mut cursor, &mut notices, (metadata.dev(),metadata.ino()));
            continue;
        }
        if directory.offset == 0 {
            if cursor.seen.contains(&(metadata.dev(), metadata.ino())) {
                continue;
            }
            if cursor.seen.len() >= 16384 || pending_bytes(&cursor) + 64 > 1024 * 1024 {
                notice(
                    &mut notices,
                    &directory.path,
                    "Reached the directory/memory limit; narrow the configured roots and restart",
                );
                cursor.pending.clear();
                break;
            }
            cursor.seen.insert((metadata.dev(), metadata.ino()));
        }
        let mut entries = match std::fs::read_dir(&directory.path) {
            Ok(entries) => entries,
            Err(e) => {
                notice(
                    &mut notices,
                    &directory.path,
                    format!("Cannot enumerate: {e}"),
                );
                continue;
            }
        };
        if entries.by_ref().take(directory.offset).count() != directory.offset {
            retry_directory(directory, &mut cursor, &mut notices, (metadata.dev(),metadata.ino()));
            continue;
        }
        let mut exhausted = true;
        for entry in entries {
            if !active() {
                return Err("DJ discovery cancelled".into());
            }
            directory.offset += 1;
            cursor.visited += 1;
            page_entries += 1;
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    notice(
                        &mut notices,
                        &directory.path,
                        format!("Unreadable entry: {e}"),
                    );
                    continue;
                }
            };
            let path = entry.path();
            if !valid_path(&path) || excluded(&path) {
                notice(
                    &mut notices,
                    &path,
                    "Excluded virtual, runtime, build, cache or unrepresentable path",
                );
            } else {
                match entry.file_type() {
                    Ok(kind) if kind.is_symlink() => {
                        notice(&mut notices, &path, "Symlink excluded")
                    }
                    Ok(kind) if kind.is_dir() => {
                        if directory.depth >= 64
                            || cursor.pending.len() >= MAX_PENDING - 1
                            || pending_bytes(&cursor) + path.as_os_str().len() + 8192 > 1024 * 1024
                        {
                            notice(&mut notices,&path,"Directory depth/queue/memory limit reached; add this library as a custom root");
                        } else {
                            cursor.pending.push_back(Directory {
                                path,
                                offset: 0,
                                depth: directory.depth + 1,
                                fingerprint: None,
                                restarts: 0,
                            });
                        }
                    }
                    Ok(kind) if kind.is_file() => match probe(&path, &snapshot, purpose) {
                        Ok(Some(candidate)) => {
                            if let Some(fd) = watch {
                                let parent = candidate.path.parent().unwrap_or(Path::new("/"));
                                if let Ok(path) =
                                    std::ffi::CString::new(parent.as_os_str().as_encoded_bytes())
                                {
                                    let flags = libc::IN_CLOSE_WRITE
                                        | libc::IN_CREATE
                                        | libc::IN_DELETE
                                        | libc::IN_MOVED_FROM
                                        | libc::IN_MOVED_TO
                                        | libc::IN_DELETE_SELF
                                        | libc::IN_MOVE_SELF;
                                    if unsafe { libc::inotify_add_watch(fd, path.as_ptr(), flags) }
                                        < 0
                                    {
                                        notice(&mut notices,parent,"Database notifications unavailable; use Rescan libraries after changes");
                                    }
                                }
                            }
                            candidates.push(candidate)
                        }
                        Ok(None) => {}
                        Err(e) => notice(
                            &mut notices,
                            &path,
                            format!("Source unavailable/changed: {e}"),
                        ),
                    },
                    Ok(_) => {}
                    Err(e) => notice(&mut notices, &path, format!("Cannot inspect entry: {e}")),
                }
            }
            if page_entries >= PAGE_ENTRIES
                || candidates.len() >= MAX_CANDIDATES
                || started.elapsed() >= Duration::from_secs(3)
            {
                exhausted = false;
                break;
            }
        }
        if !std::fs::symlink_metadata(&directory.path)
            .is_ok_and(|m| m.is_dir() && FileFingerprint::from_metadata(&m) == fingerprint)
        {
            retry_directory(directory, &mut cursor, &mut notices, (metadata.dev(),metadata.ino()));
            continue;
        }
        if !exhausted {
            directory.fingerprint = Some(fingerprint);
            cursor.pending.push_front(directory);
            break;
        }
    }
    if Sha256::digest(mount_bytes()?).as_slice() != cursor.mount_digest {
        return Err("Mount inventory changed before discovery publication".into());
    }
    let complete = cursor.pending.is_empty();
    let visited = cursor.visited;
    Ok(Page {
        candidates,
        notices,
        cursor: (!complete).then_some(cursor),
        visited,
        elapsed_millis: started.elapsed().as_millis() as u64,
        complete,
    })
}

static RETIRING: std::sync::OnceLock<std::sync::Mutex<Vec<Child>>> = std::sync::OnceLock::new();
/// Retry one changing directory without starving the other volumes.
/// Takes its traversal state, pending scan, notices and physical directory identity; restarts at the end of the queue at most twice, then records an explicit exclusion. Already verified files remain subject to source review before import.
fn retry_directory(mut directory: Directory, cursor: &mut Cursor, notices: &mut Vec<Notice>, identity: (u64,u64)) {
    if directory.restarts >= 2 {
        notice(notices,&directory.path,"Directory kept changing after two retries; other roots continue. Rescan this directory to inspect its current membership");
        return;
    }
    notice(notices,&directory.path,"Directory changed; restarting its bounded scan while other roots continue");
    directory.restarts += 1;
    directory.offset = 0;
    directory.fingerprint = None;
    cursor.seen.remove(&identity);
    cursor.pending.push_back(directory);
}
fn retiring() -> &'static std::sync::Mutex<Vec<Child>> {
    RETIRING.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}
fn reap_retiring() -> usize {
    let mut pending = retiring().lock().unwrap_or_else(|p| p.into_inner());
    pending.retain_mut(|child| !matches!(child.try_wait(), Ok(Some(_))));
    pending.len()
}
struct Process(Option<Child>);
impl Process {
    fn child(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            if !matches!(child.try_wait(), Ok(Some(_))) {
                retiring()
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .push(child);
            }
        }
    }
}
fn nonblocking(fd: i32) -> Result<(), String> {
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

/// Run filesystem discovery in a disposable app worker process.
/// Takes the request and cancellation predicate; returns a bounded validated result or kills a crashed, cancelled or stalled worker after ten seconds.
pub(crate) fn isolated(request: &Request, active: &impl Fn() -> bool) -> Result<Page, String> {
    isolated_with_watch(request, active, None)
}
fn isolated_with_watch(
    request: &Request,
    active: &impl Fn() -> bool,
    watch: Option<i32>,
) -> Result<Page, String> {
    isolated_executable(
        request,
        active,
        watch,
        &std::env::current_exe().map_err(|e| e.to_string())?,
    )
}
fn isolated_executable(
    request: &Request,
    active: &impl Fn() -> bool,
    watch: Option<i32>,
    executable: &Path,
) -> Result<Page, String> {
    request.validate()?;
    let bytes = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_REQUEST {
        return Err("DJ discovery request exceeds 2 MiB".into());
    }
    if reap_retiring() >= 4 {
        return Err("Four stopped filesystem workers are still awaiting kernel I/O retirement; retained libraries remain available".into());
    }
    let mut command = Command::new(executable);
    command
        .arg("dj-discover-worker")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_remove("OMATAINER_DISCOVERY_WATCH_FD");
    if let Some(fd) = watch {
        command.env("OMATAINER_DISCOVERY_WATCH_FD", "3");
        unsafe {
            command.pre_exec(move || {
                if libc::dup2(fd, 3) < 0 || libc::fcntl(3, libc::F_SETFD, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = Process(Some(command.spawn().map_err(|e| e.to_string())?));
    let mut input = Some(
        child
            .child()
            .stdin
            .take()
            .ok_or("Discovery worker has no input")?,
    );
    let mut output = child
        .child()
        .stdout
        .take()
        .ok_or("Discovery worker has no output")?;
    nonblocking(input.as_ref().unwrap().as_raw_fd())?;
    nonblocking(output.as_raw_fd())?;
    let started = Instant::now();
    let mut written = 0;
    let mut result = Vec::new();
    let mut buffer = [0u8; 65536];
    loop {
        if !active() {
            return Err("DJ discovery cancelled; its worker was stopped".into());
        }
        if started.elapsed() > Duration::from_secs(10) {
            return Err("DJ discovery worker timed out on an inaccessible/stalled filesystem; previous libraries remain available. Narrow the roots and retry".into());
        }
        if let Some(stream) = input.as_mut() {
            match stream.write(&bytes[written..]) {
                Ok(count) => written += count,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e.to_string()),
            };
            if written == bytes.len() {
                input = None;
            }
        }
        loop {
            match output.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    if result.len() + count > MAX_RESPONSE {
                        return Err("Discovery worker output exceeds 8 MiB".into());
                    }
                    result.extend_from_slice(&buffer[..count]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e.to_string()),
            }
        }
        if let Some(status) = child.child().try_wait().map_err(|e| e.to_string())? {
            if !status.success() {
                return Err(
                    "DJ discovery worker failed; retained libraries remain available".into(),
                );
            }
            loop {
                match output.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => {
                        if result.len() + count > MAX_RESPONSE {
                            return Err("Discovery worker output exceeds 8 MiB".into());
                        }
                        result.extend_from_slice(&buffer[..count]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => return Err(e.to_string()),
                }
            }
            let page: Result<Page, String> =
                serde_json::from_slice(&result).map_err(|e| e.to_string())?;
            let page = page?;
            if page.complete != page.cursor.is_none()
                || page.candidates.len() > MAX_CANDIDATES
                || page.notices.len() > 128
                || page
                    .notices
                    .iter()
                    .any(|n| !valid_path(&n.path) || n.state.len() > 2048)
                || page.candidates.iter().any(|c| {
                    !valid_path(&c.path)
                        || c.format.len() > 512
                        || crate::media_location::validate_root_source(&c.source).is_err()
                })
            {
                return Err("Discovery worker returned an invalid inventory".into());
            }
            Request {
                purpose: request.purpose,
                roots: vec![],
                all_mounts: false,
                cursor: page.cursor.clone(),
            }
            .validate()?;
            return Ok(page);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Serve one isolated discovery request before GUI, audio or MIDI startup.
/// Takes standard input; writes a bounded result to standard output and returns without opening devices or catalog writers.
pub(crate) fn worker() -> Result<(), String> {
    unsafe {
        libc::nice(10);
    }
    let mut bytes = Vec::new();
    std::io::stdin()
        .take((MAX_REQUEST + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_REQUEST {
        return Err("Discovery request exceeds 2 MiB".into());
    }
    let request: Request = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let watch = std::env::var("OMATAINER_DISCOVERY_WATCH_FD")
        .ok()
        .filter(|fd| fd == "3")
        .map(|_| 3);
    let result = scan_with_watch(request, &|| true, watch);
    let bytes = serde_json::to_vec(&result).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_RESPONSE {
        return Err("Discovery result exceeds 8 MiB".into());
    }
    std::io::stdout()
        .write_all(&bytes)
        .map_err(|e| e.to_string())
}

struct Notifications {
    file: Option<Arc<File>>,
    changed: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Notifications {
    fn start() -> Result<Self, String> {
        let fd = unsafe { libc::inotify_init1(libc::IN_NONBLOCK | libc::IN_CLOEXEC) };
        let file = (fd >= 0).then(|| Arc::new(unsafe { File::from_raw_fd(fd) }));
        let changed = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let worker_file = file.clone();
        let worker_changed = changed.clone();
        let worker_stop = stop.clone();
        let thread = std::thread::Builder::new()
            .name("dj-library-notifications".into())
            .spawn(move || {
                let mut previous = mount_bytes().ok().map(Sha256::digest);
                let mut inspected = Instant::now();
                let mut last_event = None;
                let mut buffer = [0u8; 65536];
                while !worker_stop.load(Ordering::Acquire) {
                    if let Some(file) = &worker_file {
                        let mut file = &**file;
                        if matches!(file.read(&mut buffer),Ok(count) if count>0) {
                            last_event = Some(Instant::now());
                        }
                    }
                    if inspected.elapsed() >= Duration::from_secs(2) {
                        let current = mount_bytes().ok().map(Sha256::digest);
                        if current != previous {
                            previous = current;
                            last_event = Some(Instant::now());
                        }
                        inspected = Instant::now();
                    }
                    if last_event.is_some_and(|event| event.elapsed() >= Duration::from_millis(500))
                    {
                        worker_changed.store(true, Ordering::Release);
                        last_event = None;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            file,
            changed,
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Notifications {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            let _ = self.thread.take().unwrap().join();
        }
    }
}
struct Pending {
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<Result<Page, String>>,
    thread: JoinHandle<()>,
}

#[derive(Default)]
pub(crate) struct Discovery {
    pending: Option<Pending>,
    retired: Vec<JoinHandle<()>>,
    notifications: Option<Notifications>,
    pub candidates: Vec<Candidate>,
    pub notices: Vec<Notice>,
    pub cursor: Option<Cursor>,
    pub message: String,
    pub initialized: bool,
    pub refresh: bool,
    pub paused: bool,
    pub visited: usize,
    pub elapsed_millis: u64,
}
impl Discovery {
    /// Queue one admitted background discovery page.
    /// Takes accessible roots, continuation choice and performance policy; returns immediately after spawning a broker, keeping prior results intact.
    pub fn start(
        &mut self,
        roots: Vec<PathBuf>,
        all_mounts: bool,
        continuation: bool,
        performance: &crate::engine::performance::Handle,
    ) -> Result<(), String> {
        self.start_for(Purpose::Dj, roots, all_mounts, continuation, performance)
    }
    /// Discover libraries with the same bounded mount-aware worker.
    /// Takes the library purpose, roots, resume choice and performance policy; returns after admitting one page without changing source files.
    pub fn start_for(&mut self, purpose: Purpose, roots: Vec<PathBuf>, all_mounts: bool, continuation: bool, performance: &crate::engine::performance::Handle) -> Result<(), String> {
        if self.pending.is_some() {
            return Err("Library discovery is already running".into());
        }
        let request = Request {
            purpose,
            roots,
            all_mounts,
            cursor: if continuation {
                self.cursor.clone()
            } else {
                None
            },
        };
        request.validate()?;
        let work = Arc::new(performance.optional_work().map_err(|e| e.to_string())?);
        let ticket = work.background(
            crate::background::Kind::Index,
            "DJ library discovery".into(),
            256 * crate::background::MIB,
        )?;
        if !continuation || self.notifications.is_none() {
            self.notifications = Some(Notifications::start()?);
        }
        let watch = self.notifications.as_ref().and_then(|n| n.file.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let (sender, result) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("dj-library-discovery".into())
            .spawn(move || {
                let active = || !worker_cancel.load(Ordering::Acquire) && !work.cancelled();
                let result = (|| {
                    let _running = ticket.enter(|| !active())?;
                    let page = isolated_with_watch(
                        &request,
                        &active,
                        watch.as_ref().map(|f| f.as_raw_fd()),
                    )?;
                    ticket.progress(page.visited as u64, None);
                    if !active() {
                        return Err("DJ discovery cancelled before its result was accepted".into());
                    }
                    Ok(page)
                })();
                let _ = sender.send(result);
            })
            .map_err(|e| e.to_string())?;
        self.pending = Some(Pending {
            cancel,
            result,
            thread,
        });
        self.initialized = true;
        self.refresh = false;
        self.paused = false;
        if !continuation {
            self.cursor = None;
            self.visited = 0;
            self.elapsed_millis = 0;
        }
        self.message = "Discovering libraries in the background…".into();
        Ok(())
    }
    /// Poll completed pages and volume/database hints without filesystem work.
    /// Takes no arguments; retains previous candidates on cancellation/failure and requests a debounced fresh scan after watched sources change.
    pub fn poll(&mut self) {
        reap_retiring();
        self.retired.retain_mut(|thread| !thread.is_finished());
        if self
            .notifications
            .as_ref()
            .is_some_and(|n| n.changed.swap(false, Ordering::AcqRel))
        {
            self.cancel();
            self.refresh = true;
            self.cursor = None;
        }
        let result = self
            .pending
            .as_ref()
            .and_then(|p| match p.result.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(
                    "DJ discovery broker stopped before returning its inventory".into(),
                )),
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = result {
            let pending = self.pending.take().unwrap();
            let cancelled = pending.cancel.load(Ordering::Acquire);
            self.retired.push(pending.thread);
            match result {
                Ok(page) if !cancelled => {
                    for candidate in page.candidates {
                        if let Some(existing) = self
                            .candidates
                            .iter_mut()
                            .find(|c| c.source == candidate.source)
                        {
                            *existing = candidate;
                        } else if self.candidates.len() < 4096 {
                            self.candidates.push(candidate);
                        } else {
                            self.paused = true;
                            self.message =
                                "Library source capacity reached (4096); narrow roots and rescan"
                                    .into();
                            return;
                        }
                    }
                    self.visited = page.visited;
                    self.elapsed_millis = self.elapsed_millis.saturating_add(page.elapsed_millis);
                    self.cursor = page.cursor;
                    self.notices = page.notices;
                    self.message=format!("{} library sources · {} entries inspected · {} exclusions/failures in this page{}",self.candidates.len(),self.visited,self.notices.len(),if page.complete {" · finished within the reported limits"}else{" · more paths remain"});
                }
                Ok(_) => {
                    self.message = "Discovery cancelled; previous library sources retained".into()
                }
                Err(error) => {
                    self.paused = true;
                    self.message = error;
                }
            }
        }
    }
    pub fn active(&self) -> bool {
        self.pending.is_some()
    }
    pub fn cancel(&mut self) {
        self.paused = true;
        if let Some(pending) = &self.pending {
            pending.cancel.store(true, Ordering::Release);
        }
    }
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::media_analysis::tests::Files;
    #[test]
    fn unusual_readonly_paths_content_and_unknown_databases_are_discovered_without_writes() {
        let files = Files::new();
        let nested = files.0.join("Unusual/東京 library");
        std::fs::create_dir_all(&nested).unwrap();
        let xml=b"<DJ_PLAYLISTS Version=\"1.0.0\"><COLLECTION Entries=\"0\"/><PLAYLISTS><NODE Type=\"0\" Name=\"ROOT\" Count=\"0\"/></PLAYLISTS></DJ_PLAYLISTS>";
        std::fs::write(nested.join("export.unusual"), xml).unwrap();
        std::fs::write(nested.join("master.db"), b"encrypted vendor DB").unwrap();
        std::fs::write(nested.join("unrelated.xml"), b"<BUILD/>").unwrap();
        std::os::unix::fs::symlink(&files.0, nested.join("loop")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            nested.join("export.unusual"),
            std::fs::Permissions::from_mode(0o444),
        )
        .unwrap();
        let before = FileFingerprint::read(&nested.join("export.unusual")).unwrap();
        let result = scan(
            Request {
                purpose: Purpose::Dj,
                roots: vec![files.0.clone()],
                all_mounts: false,
                cursor: None,
            },
            &|| true,
        )
        .unwrap();
        assert!(result.complete);
        assert_eq!(result.candidates.len(), 2);
        assert_eq!(result.candidates.iter().filter(|c| c.reviewable).count(), 1);
        assert!(result.notices.iter().any(|n| n.state.contains("Symlink")));
        assert_eq!(std::fs::read(nested.join("export.unusual")).unwrap(), xml);
        assert_eq!(
            FileFingerprint::read(&nested.join("export.unusual")),
            Some(before)
        );
        assert_eq!(
            std::fs::metadata(nested.join("export.unusual"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o444
        );
        assert!(scan(
            Request {
                purpose: Purpose::Dj,
                roots: vec![files.0.clone()],
                all_mounts: false,
                cursor: None
            },
            &|| false
        )
        .is_err());
    }
    #[test]
    fn discovery_resumes_large_directories_retries_changes_and_refuses_changed_mounts() {
        let files = Files::new();
        for index in 0..10020 {
            std::fs::write(files.0.join(format!("file-{index}.bin")), b"").unwrap();
        }
        let first = scan(
            Request {
                purpose: Purpose::Dj,
                roots: vec![files.0.clone()],
                all_mounts: false,
                cursor: None,
            },
            &|| true,
        )
        .unwrap();
        assert!(!first.complete);
        assert_eq!(first.visited, 10000);
        let next = scan(
            Request {
                purpose: Purpose::Dj,
                roots: vec![files.0.clone()],
                all_mounts: false,
                cursor: first.cursor.clone(),
            },
            &|| true,
        )
        .unwrap();
        assert!(next.complete);
        assert_eq!(next.visited, 10020);
        let mut wrong = first.cursor.clone().unwrap();
        wrong.mount_digest = [0; 32];
        assert!(scan(
            Request {
                purpose: Purpose::Dj,
                roots: vec![],
                all_mounts: false,
                cursor: Some(wrong)
            },
            &|| true
        )
        .unwrap_err()
        .contains("Mounts changed"));
        std::fs::write(files.0.join("new.bin"), b"").unwrap();
        let other = Files::new();
        std::fs::write(other.0.join("other.m3u"), b"#EXTM3U\n").unwrap();
        let mut changed = first.cursor.unwrap();
        changed.pending.push_back(Directory {path:other.0.clone(),offset:0,depth:0,fingerprint:None,restarts:0});
        let retry = scan(
            Request {
                purpose: Purpose::Dj,
                roots: vec![files.0.clone(),other.0.clone()],
                all_mounts: false,
                cursor: Some(changed)
            },
            &|| true
        )
        .unwrap();
        assert!(retry.notices.iter().any(|n|n.state.contains("Directory changed")));
        assert!(retry.candidates.iter().any(|c|c.path==other.0.join("other.m3u")));
        let mut cursor=retry.cursor;
        let mut restarts=0;
        while let Some(next)=cursor {
            let page=scan(Request {purpose:Purpose::Dj,roots:vec![files.0.clone(),other.0.clone()],all_mounts:false,cursor:Some(next)},&||true).unwrap();
            cursor=page.cursor;
            restarts+=1;
            assert!(restarts<4,"Stable retry did not finish within its entry bounds");
        }
    }
    #[test]
    fn apple_library_and_utf16_detection_exclude_system_plists_and_media() {
        let files=Files::new();
        std::fs::write(files.0.join("system.plist"),b"<?xml version=\"1.0\"?><plist><dict><key>ProductVersion</key><string>1</string></dict></plist>").unwrap();
        let library="<?xml version=\"1.0\" encoding=\"UTF-16\"?><plist><dict><key>Tracks</key><dict/></dict></plist>";
        let mut bytes=vec![0xff,0xfe];for c in library.encode_utf16(){bytes.extend_from_slice(&c.to_le_bytes());}
        std::fs::write(files.0.join("owned-apple.xml"),bytes).unwrap();
        std::fs::write(files.0.join("audio.wav"),b"<NML></NML>").unwrap();
        std::fs::write(files.0.join("notes.txt"),[0xff,0xfe,b'x',0]).unwrap();
        let page=scan(Request{purpose:Purpose::Dj,roots:vec![files.0.clone()],all_mounts:false,cursor:None},&||true).unwrap();
        assert!(page.complete);
        assert_eq!(page.candidates.len(),1);
        assert_eq!(page.candidates[0].path,files.0.join("owned-apple.xml"));
        assert!(page.candidates[0].reviewable);
    }
    #[test]
    fn request_bounds_and_virtual_roots_are_explicit() {
        assert!(Request {
            purpose: Purpose::Dj,
            roots: vec!["relative".into()],
            all_mounts: false,
            cursor: None
        }
        .validate()
        .is_err());
        assert!(Request {
            purpose: Purpose::Dj,
            roots: vec!["/home/../proc".into()],
            all_mounts: false,
            cursor: None
        }
        .validate()
        .is_err());
        assert!(Request {
            purpose: Purpose::Dj,
            roots: vec!["/home".into(); 65],
            all_mounts: false,
            cursor: None
        }
        .validate()
        .is_err());
        let result = scan(
            Request {
                purpose: Purpose::Dj,
                roots: vec!["/proc".into()],
                all_mounts: false,
                cursor: None,
            },
            &|| true,
        )
        .unwrap();
        assert_eq!(result.visited, 0);
        assert!(result
            .notices
            .iter()
            .any(|n| n.state.contains("Excluded virtual")));
    }

    #[test]
    fn notice_limits_and_cancel_prevent_automatic_restart() {
        let mut notices = Vec::new();
        for _ in 0..1000 {
            notice(&mut notices, Path::new("/known/root"), "Unavailable");
        }
        assert_eq!(notices.len(), 128);
        assert!(notices.last().unwrap().state.contains("omitted"));
        let mut discovery = Discovery::default();
        discovery.initialized = true;
        discovery.cancel();
        discovery.poll();
        assert!(discovery.paused);
        assert!(!discovery.active());
    }

    #[test]
    #[ignore = "Requires a private read-only block-volume remount driver, OMATAINER_VOLUME_DIR and OMATAINER_TEST_BIN"]
    fn native_read_only_volume_remount_retains_uuid_and_discovers_new_mount_path() {
        use crate::media_location::{Failure, Snapshot};
        let directory = PathBuf::from(std::env::var_os("OMATAINER_VOLUME_DIR").unwrap());
        assert!(directory.starts_with(Path::new(env!("CARGO_MANIFEST_DIR")).join("t")));
        let binary = PathBuf::from(std::env::var_os("OMATAINER_TEST_BIN").unwrap());
        let first = directory.join("first");
        let second = directory.join("second");
        let wave = first.join("music/track.wav");
        let bytes = std::fs::read(&wave).unwrap();
        let enrolled = Snapshot::fixture_local_volume(&wave).unwrap();
        let original = enrolled.identify(&wave).unwrap();
        assert!(matches!(original.source, LibSource::Removable { .. }));
        let source = original.source.clone();
        let fingerprint = FileFingerprint::read(&original.path).unwrap();
        let production = Snapshot::discover().unwrap();
        assert_eq!(production.resolve(&source).unwrap().path, original.path);
        let first_page = isolated_executable(&Request {
            purpose: Purpose::Dj,
            roots: vec![first.clone()],
            all_mounts: false,
            cursor: None,
        }, &|| true, None, &binary).unwrap();
        assert!(first_page.complete);
        assert_eq!(first_page.candidates.len(), 1);
        assert_eq!(first_page.candidates[0].path, first.join("owned.m3u8"));
        assert!(first_page.candidates[0].reviewable);
        let request = serde_json::to_vec(&source).unwrap();
        std::fs::write(directory.join("source.json"), &request).unwrap();
        std::fs::write(directory.join("first.ready"), b"Enrolled real filesystem UUID; no physical USB classification claim").unwrap();
        let wait = |name: &str| {
            let deadline = Instant::now() + Duration::from_secs(20);
            while !directory.join(name).exists() {
                assert!(Instant::now() < deadline, "Volume driver did not acknowledge {name}");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        wait("offline.ready");
        let offline = Snapshot::discover().unwrap();
        assert_eq!(offline.resolve(&source).unwrap_err(), Failure::Offline);
        assert_eq!(offline.inspect(&original).unwrap_err(), Failure::Changed);
        let absent = isolated_executable(&Request {
            purpose: Purpose::Dj,
            roots: vec![first.clone()],
            all_mounts: false,
            cursor: None,
        }, &|| true, None, &binary).unwrap();
        assert!(absent.candidates.is_empty());
        std::fs::write(directory.join("offline.checked"), b"Offline is distinct from deleted").unwrap();
        wait("remounted.ready");
        let reloaded: LibSource = serde_json::from_slice(&std::fs::read(directory.join("source.json")).unwrap()).unwrap();
        assert_eq!(reloaded, source);
        let mounted = Snapshot::discover().unwrap();
        let resolved = mounted.resolve(&reloaded).unwrap();
        assert_eq!(resolved.path, second.join("music/track.wav"));
        assert_eq!(resolved.source, source);
        assert_eq!(FileFingerprint::read(&resolved.path).unwrap(), fingerprint);
        assert_eq!(std::fs::read(&resolved.path).unwrap(), bytes);
        resolved.recheck().unwrap();
        assert_eq!(mounted.inspect(&original).unwrap_err(), Failure::Changed);
        let page = isolated_executable(&Request {
            purpose: Purpose::Dj,
            roots: vec![second.clone()],
            all_mounts: false,
            cursor: None,
        }, &|| true, None, &binary).unwrap();
        assert!(page.complete);
        assert_eq!(page.candidates.len(), 1);
        assert_eq!(page.candidates[0].path, second.join("owned.m3u8"));
        assert!(page.candidates[0].reviewable);
        let report = serde_json::json!({"status":"pass","persisted_source":source,"before_path":original.path,"after_path":resolved.path,"fingerprint_retained":true,"original_location_refused":true,"offline_distinct_from_missing":true,"native_worker_before":first_page,"native_worker_offline":absent,"native_worker_after":page,"scope":"Real Linux filesystem UUID, read-only mounts, offline/remount and native disposable DJ workers. Initial local loop-volume removable classification is injected only for enrollment. No physical USB removal, catalog publication, audio-load impact or listening claim."});
        std::fs::write(directory.join("volume-remount.json"), serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }

    #[test]
    #[ignore = "Requires OMATAINER_TEST_BIN pointing at the native app built from this checkout"]
    fn native_worker_ipc_cancellation_and_crash_keep_previous_sources() {
        let binary = PathBuf::from(
            std::env::var_os("OMATAINER_TEST_BIN").expect("Set the exact native app path"),
        );
        let files = Files::new();
        std::fs::write(files.0.join("library.m3u8"), b"#EXTM3U\nmissing.flac\n").unwrap();
        let request = Request {
            purpose: Purpose::Dj,
            roots: vec![files.0.clone()],
            all_mounts: false,
            cursor: None,
        };
        let notifications = Notifications::start().unwrap();
        let first = isolated_executable(
            &request,
            &|| true,
            notifications.file.as_ref().map(|f| f.as_raw_fd()),
            &binary,
        )
        .unwrap();
        assert!(first.complete);
        assert_eq!(first.candidates.len(), 1);
        assert!(first.candidates[0].reviewable);
        std::fs::write(files.0.join("library.m3u8"), b"#EXTM3U\nchanged.flac\n").unwrap();
        let started = Instant::now();
        while !notifications.changed.load(Ordering::Acquire)
            && started.elapsed() < Duration::from_secs(3)
        {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            notifications.changed.load(Ordering::Acquire),
            "The worker must enroll the database parent into native notifications"
        );
        assert!(isolated_executable(&request, &|| false, None, &binary)
            .unwrap_err()
            .contains("cancelled"));
        use std::os::unix::fs::PermissionsExt;
        let crash = files.0.join("crash-worker");
        std::fs::write(&crash, b"#!/bin/sh\ncat >/dev/null\nexit 31\n").unwrap();
        std::fs::set_permissions(&crash, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(isolated_executable(&request, &|| true, None, &crash)
            .unwrap_err()
            .contains("failed"));
        let stalled = files.0.join("stalled-worker");
        std::fs::write(&stalled, b"#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&stalled, std::fs::Permissions::from_mode(0o700)).unwrap();
        let started = Instant::now();
        let cancel = || started.elapsed() < Duration::from_millis(150);
        assert!(isolated_executable(&request, &cancel, None, &stalled)
            .unwrap_err()
            .contains("cancelled"));
        assert!(started.elapsed() < Duration::from_secs(2));
        let mut discovery = Discovery::default();
        discovery.candidates = first.candidates;
        let (sender, result) = mpsc::sync_channel(1);
        sender.send(Err("injected scan failure".into())).unwrap();
        discovery.pending = Some(Pending {
            cancel: Arc::new(AtomicBool::new(false)),
            result,
            thread: std::thread::spawn(|| {}),
        });
        discovery.poll();
        assert_eq!(discovery.candidates.len(), 1);
        assert!(discovery.paused);
        assert_eq!(discovery.message, "injected scan failure");
    }
}
