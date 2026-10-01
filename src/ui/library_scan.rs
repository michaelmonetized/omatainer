//! One cooperative filesystem worker; the UI only polls and swaps completed data.
use super::{builtin_crate_items, parse_tags, sort_crate, split_artist_title, LibItem, LibSource};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use super::{Bpm, FileFingerprint};
use walkdir::WalkDir;

#[cfg(test)]
mod tests;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ScanState {
    Idle,
    Scanning,
    Cancelling,
    Complete(usize),
    Cancelled,
    Failed(String),
}

#[derive(Default)]
struct Progress {
    visited: AtomicUsize,
    found: AtomicUsize,
    phase: AtomicU8,
}

#[derive(Clone, Default)]
pub(super) struct Options {
    #[cfg(test)]
    pub before_entry: Option<Arc<dyn Fn(&Path) + Send + Sync>>,
}

struct Request {
    roots: Vec<PathBuf>,
    baseline: Arc<Vec<LibItem>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Progress>,
    #[cfg_attr(not(test), allow(dead_code))]
    options: Options,
}

struct Retired {
    _library: Arc<Vec<LibItem>>,
    published: bool,
}

pub(super) struct Publication {
    pub items: Arc<Vec<LibItem>>,
    retirement: mpsc::SyncSender<Retired>,
}

impl Publication {
    pub fn publish(self, library: &mut Arc<Vec<LibItem>>) {
        let previous = std::mem::replace(library, self.items);
        // Exactly one acknowledgment fits the reserved slot. The worker keeps
        // both baseline and candidate alive until this handoff, so neither a
        // large replaced crate nor a cancelled result is destroyed on the UI.
        let _ = self.retirement.try_send(Retired {
            _library: previous,
            published: true,
        });
    }

    fn discard(self) {
        let _ = self.retirement.try_send(Retired {
            _library: self.items,
            published: false,
        });
    }
}

enum Completion {
    Ready(Publication),
    Cancelled,
    Failed(String),
}

pub(super) struct LibraryScan {
    requests: Option<mpsc::SyncSender<Request>>,
    completion: mpsc::Receiver<Completion>,
    worker: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Progress>,
    pub state: ScanState,
}

impl Default for LibraryScan {
    fn default() -> Self {
        let (requests, jobs) = mpsc::sync_channel::<Request>(1);
        let (finished, completion) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("omatainer-library".into())
            .spawn(move || {
                let mut fingerprints = HashMap::new();
                while let Ok(request) = jobs.recv() {
                    match scan(&request, &fingerprints) {
                        Ok((items, next_fingerprints)) => {
                            let items = Arc::new(items);
                            let (retirement, retired) = mpsc::sync_channel(1);
                            if finished
                                .send(Completion::Ready(Publication {
                                    items: items.clone(),
                                    retirement,
                                }))
                                .is_err()
                            {
                                break;
                            }
                            request.progress.phase.store(2, Ordering::Release);
                            // Wait for a publication decision, never for UI
                            // work while holding a filesystem or shared lock.
                            if let Ok(retired) = retired.recv() {
                                if retired.published {
                                    fingerprints = next_fingerprints;
                                }
                                drop(retired);
                            }
                        }
                        Err(ScanFailure::Cancelled) => {
                            if finished.send(Completion::Cancelled).is_err() {
                                break;
                            }
                        }
                        Err(ScanFailure::Io(error)) => {
                            if finished.send(Completion::Failed(error)).is_err() {
                                break;
                            }
                        }
                    }
                }
            });
        match worker {
            Ok(worker) => Self {
                requests: Some(requests),
                completion,
                worker: Some(worker),
                cancel: Arc::new(AtomicBool::new(false)),
                progress: Arc::new(Progress::default()),
                state: ScanState::Idle,
            },
            Err(error) => Self {
                requests: None,
                completion,
                worker: None,
                cancel: Arc::new(AtomicBool::new(false)),
                progress: Arc::new(Progress::default()),
                state: ScanState::Failed(format!("Could not start library scan: {error}")),
            },
        }
    }
}

impl LibraryScan {
    pub fn active(&self) -> bool {
        matches!(self.state, ScanState::Scanning | ScanState::Cancelling)
    }

    pub fn start(&mut self, roots: Vec<PathBuf>, baseline: Arc<Vec<LibItem>>) -> bool {
        self.start_with(roots, baseline, Options::default())
    }

    pub fn start_with(
        &mut self,
        roots: Vec<PathBuf>,
        baseline: Arc<Vec<LibItem>>,
        options: Options,
    ) -> bool {
        if self.active() {
            return false;
        }
        // A failed worker can be retried explicitly by the Scan button.
        if self.requests.is_none() {
            *self = Self::default();
        }
        let Some(requests) = &self.requests else {
            return false;
        };
        self.cancel = Arc::new(AtomicBool::new(false));
        self.progress = Arc::new(Progress::default());
        let request = Request {
            roots,
            baseline,
            cancel: self.cancel.clone(),
            progress: self.progress.clone(),
            options,
        };
        match requests.try_send(request) {
            Ok(()) => {
                self.state = ScanState::Scanning;
                true
            }
            Err(_) => {
                self.state =
                    ScanState::Failed("Library scan worker unavailable; retry scan".into());
                self.requests = None;
                false
            }
        }
    }

    pub fn cancel(&mut self) {
        if self.active() {
            self.cancel.store(true, Ordering::Release);
            self.state = ScanState::Cancelling;
        }
    }

    pub fn poll(&mut self) -> Option<Publication> {
        match self.completion.try_recv() {
            Ok(Completion::Ready(publication)) => {
                if self.cancel.load(Ordering::Acquire) {
                    publication.discard();
                    self.state = ScanState::Cancelled;
                } else {
                    self.state = ScanState::Complete(publication.items.len());
                    return Some(publication);
                }
            }
            Ok(Completion::Cancelled) => self.state = ScanState::Cancelled,
            Ok(Completion::Failed(error)) => self.state = ScanState::Failed(error),
            Err(mpsc::TryRecvError::Disconnected) if self.active() => {
                self.state = ScanState::Failed("Library scan worker stopped; retry scan".into());
                self.requests = None;
            }
            Err(_) => {}
        }
        None
    }

    pub fn label(&self) -> String {
        match &self.state {
            ScanState::Idle => "Ready to scan".into(),
            ScanState::Scanning => {
                let phase = match self.progress.phase.load(Ordering::Relaxed) {
                    1 => "Sorting",
                    2 => "Finishing",
                    _ => "Scanning",
                };
                format!(
                    "{phase}: {} entries · {} audio files",
                    self.progress.visited.load(Ordering::Relaxed),
                    self.progress.found.load(Ordering::Relaxed),
                )
            }
            ScanState::Cancelling => "Cancelling scan…".into(),
            ScanState::Complete(count) => format!("Scan complete · {count} tracks"),
            ScanState::Cancelled => "Scan cancelled · crate unchanged".into(),
            ScanState::Failed(error) => format!("Scan failed · {error}"),
        }
    }
}

impl Drop for LibraryScan {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.requests.take();
        // Filesystem syscalls cannot be forcibly cancelled portably. Do not
        // block the UI or shutdown on one; the sole worker exits at its next
        // cancellation check and owns all of its data independently of App.
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

type Fingerprint = FileFingerprint;

enum ScanFailure {
    Cancelled,
    Io(String),
}

fn check_cancel(request: &Request) -> Result<(), ScanFailure> {
    if request.cancel.load(Ordering::Acquire) {
        Err(ScanFailure::Cancelled)
    } else {
        Ok(())
    }
}

fn io_error(path: &Path, error: impl std::fmt::Display) -> ScanFailure {
    let message = format!("{}: {error}", path.display());
    ScanFailure::Io(message.chars().take(256).collect())
}

fn scan(
    request: &Request,
    previous_fingerprints: &HashMap<PathBuf, Fingerprint>,
) -> Result<(Vec<LibItem>, HashMap<PathBuf, Fingerprint>), ScanFailure> {
    check_cancel(request)?;
    let previous: HashMap<_, _> = request
        .baseline
        .iter()
        .map(|item| (&item.source, item))
        .collect();
    let mut items = builtin_crate_items();
    for item in &mut items {
        if let Some(old) = previous.get(&item.source) {
            preserve_metadata(item, old);
        }
    }
    let mut fingerprints = HashMap::new();
    let mut seen = HashSet::new();
    for root in &request.roots {
        check_cancel(request)?;
        match std::fs::metadata(root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_error(root, error)),
            Ok(metadata) if !metadata.is_dir() => {
                return Err(io_error(root, "scan root is not a directory"));
            }
            Ok(_) => {}
        }
        for entry in WalkDir::new(root).max_depth(6) {
            check_cancel(request)?;
            let entry = entry.map_err(|error| io_error(root, error))?;
            #[cfg(test)]
            if let Some(hook) = &request.options.before_entry {
                hook(entry.path());
            }
            check_cancel(request)?;
            request.progress.visited.fetch_add(1, Ordering::Relaxed);
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let extension = path
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if !matches!(
                extension.as_str(),
                "wav" | "mp3" | "flac" | "ogg" | "aiff" | "aif" | "m4a" | "aac"
            ) || !seen.insert(path.to_path_buf())
            {
                continue;
            }
            let metadata = entry.metadata().map_err(|error| io_error(path, error))?;
            let fingerprint = FileFingerprint::from_metadata(&metadata);
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("track");
            let (artist, title) = split_artist_title(stem);
            let (bpm, key) = parse_tags(stem);
            let mut item = LibItem {
                title,
                artist,
                bpm: Bpm::hint(bpm),
                fingerprint: Some(fingerprint),
                key,
                length: 0.0,
                last_play: None,
                source: LibSource::File(path.to_path_buf()),
            };
            if previous_fingerprints
                .get(path)
                .is_none_or(|previous| *previous == fingerprint)
            {
                if let Some(old) = previous.get(&item.source) {
                    preserve_metadata(&mut item, old);
                }
            }
            fingerprints.insert(path.to_path_buf(), fingerprint);
            items.push(item);
            request.progress.found.fetch_add(1, Ordering::Relaxed);
        }
    }
    check_cancel(request)?;
    request.progress.phase.store(1, Ordering::Relaxed);
    sort_crate(&mut items);
    check_cancel(request)?;
    Ok((items, fingerprints))
}

fn preserve_metadata(item: &mut LibItem, old: &LibItem) {
    if !old.title.is_empty() {
        item.title.clone_from(&old.title);
    }
    if !old.artist.is_empty() {
        item.artist.clone_from(&old.artist);
    }
    if old.fingerprint == item.fingerprint { item.bpm = old.bpm; }
    if !old.key.is_empty() && old.key != "—" {
        item.key.clone_from(&old.key);
    }
    if old.length.is_finite() && old.length > 0.0 {
        item.length = old.length;
    }
    item.last_play = old.last_play;
}
