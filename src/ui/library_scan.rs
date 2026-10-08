//! One cooperative filesystem worker; the UI only polls and swaps completed data.
use super::{builtin_crate_items, parse_tags, sort_crate, split_artist_title, LibItem, LibSource};
use super::{Bpm, FileFingerprint};
use crate::engine::performance::{Handle, WorkPermit};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;

#[cfg(test)]
mod tests;
mod traversal;
mod watch;
mod replacement;
mod tags;
pub(super) mod tag_jobs;
pub(super) use replacement::SearchHandle;

const MAX_INPUTS: usize = 64;
const MAX_DEPTH: usize = 64;
const MAX_VISITED: usize = 1_000_000;
const MAX_ITEMS: usize = 100_000;
const MAX_SKIP_SAMPLES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SkipReason { Unsupported, Unreadable, MissingRoot, Symlink, DepthLimit, Capacity, Duplicate, InvalidPath }
impl SkipReason {
    pub fn label(self) -> &'static str { match self {
        Self::Unsupported => "unsupported file type", Self::Unreadable => "unreadable entry", Self::MissingRoot => "unavailable input",
        Self::Symlink => "symlink not traversed", Self::DepthLimit => "depth limit", Self::Capacity => "inventory limit",
        Self::Duplicate => "overlapping input", Self::InvalidPath => "path cannot be persisted",
    } }
}
#[derive(Clone, Debug)]
pub(super) struct Skipped { pub path: String, pub reason: SkipReason, pub detail: String }
#[derive(Clone, Debug, Default)]
pub(super) struct Summary { pub skipped: [usize; 8], pub samples: Vec<Skipped>, pub truncated: bool,
    pub availability:HashMap<LibSource,String>,pub watch_limited:bool }
impl Summary { pub fn skipped_count(&self) -> usize { self.skipped.iter().sum() } }

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind { Roots, Import }

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ScanState {
    Idle,
    Tags,
    TagsFinished,
    TagsCancelling,
    Scanning,
    Searching,
    SearchCancelling,
    SearchFinished(String),
    Cancelling,
    Complete(usize),
    Cancelled,
    Failed(String),
}

#[derive(Default)]
struct Progress {
    reporter: std::sync::Mutex<crate::background::Reporter>,
    visited: AtomicUsize,
    found: AtomicUsize,
    phase: AtomicU8,
    skipped: AtomicUsize,
    reasons: [AtomicUsize; 8],
}

#[derive(Clone, Default)]
pub(super) struct Options {
    #[cfg(test)]
    pub before_entry: Option<Arc<dyn Fn(&Path) + Send + Sync>>,

}

#[derive(Clone)]
struct Watch { profile: String, catalog: Arc<crate::library::Catalog> }
#[derive(Debug)]
pub(super) struct ScanRoots {
    pub tags: Vec<tags::Observed>,
    pub book:Option<crate::library::watch_roots::Batch>,
    pub adoptions:Vec<crate::library::watch_roots::Adoption>,
    pub directories:Vec<PathBuf>,
}
type RootBatch = Arc<ScanRoots>;
struct Request {
    tags: Option<tag_jobs::Job>,
    replacement: Option<replacement::Task>,
    performance:Handle,
    watch_enabled:bool,
    watch: Option<Watch>,
    work: Arc<WorkPermit>,
    roots: Vec<PathBuf>,
    kind: Kind,
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
    pub(super) work: Arc<WorkPermit>,
    pub items: Arc<Vec<LibItem>>,
    pub summary: Arc<Summary>,
    pub roots: Option<RootBatch>,
    retirement: mpsc::SyncSender<Retired>,
}

impl Publication {
    pub fn publish(self, library: &mut Arc<Vec<LibItem>>) {
        let Ok(_commit) = self.work.commit() else {
            self.discard();
            return;
        };
        let previous = std::mem::replace(library, self.items);
        // Exactly one acknowledgment fits the reserved slot. The worker keeps
        // both baseline and candidate alive until this handoff, so neither a
        // large replaced crate nor a cancelled result is destroyed on the UI.
        let _ = self.retirement.try_send(Retired {
            _library: previous,
            published: true,
        });
    }

    pub(super) fn stage(
        self,
        baseline: &Arc<Vec<LibItem>>,
    ) -> Option<(Arc<Vec<LibItem>>, Arc<WorkPermit>, Option<RootBatch>)> {
        if self.work.cancel().load(Ordering::Acquire) {
            self.discard();
            return None;
        }
        let _ = self.retirement.try_send(Retired {
            _library: baseline.clone(),
            published: true,
        });
        Some((self.items, self.work, self.roots))
    }

    pub(super) fn discard(self) {
        let _ = self.retirement.try_send(Retired {
            _library: self.items,
            published: false,
        });
    }
}

enum Completion {
    Tags(u64, Arc<tag_jobs::Reply>),
    Replacement(u64, Result<Arc<crate::library::relocation_search::Receipt>, String>),
    Ready(Publication),
    Cancelled,
    Failed(String),
}

pub(super) struct LibraryScan {
    tag_result: Option<(u64, Arc<tag_jobs::Reply>)>,
    #[cfg(test)]
    tag_hook: Arc<std::sync::Mutex<Option<Arc<dyn Fn(&tag_jobs::Task) + Send + Sync>>>>,
    next_search: u64,
    replacement: Option<(u64, Result<Arc<crate::library::relocation_search::Receipt>, String>)>,
    watch_enabled:bool,
    watch_ready:Arc<AtomicBool>,
    changed:Arc<AtomicBool>,
    performance: Handle,
    requests: Option<mpsc::SyncSender<Request>>,
    completion: mpsc::Receiver<Completion>,
    worker: Option<JoinHandle<()>>,
    cancel: Arc<AtomicBool>,
    progress: Arc<Progress>,
    pub state: ScanState,
    pub summary: Option<Arc<Summary>>,
}

impl Default for LibraryScan {
    fn default() -> Self { Self::with_inventory(crate::media_location::Snapshot::discover) }
}
impl LibraryScan {
    #[cfg(test)]
    pub(super) fn pending_description(&self) -> String {
        format!(
            "state={:?} phase={} visited={} found={} cancelled={} worker_finished={} progress={:?} protection={:?} jobs={:?}",
            self.state, self.progress.phase.load(Ordering::Acquire),
            self.progress.visited.load(Ordering::Relaxed), self.progress.found.load(Ordering::Relaxed),
            self.cancel.load(Ordering::Acquire), self.worker.as_ref().is_none_or(JoinHandle::is_finished),
            self.progress.reporter.lock().unwrap().values(), self.performance.status(), self.performance.jobs().snapshot(),
        )
    }

    pub(super) fn with_inventory(mut inventory: impl FnMut() -> Result<crate::media_location::Snapshot, crate::media_location::Failure> + Send + 'static) -> Self {
        #[cfg(test)]
        let tag_hook = Arc::new(std::sync::Mutex::new(None::<Arc<dyn Fn(&tag_jobs::Task) + Send + Sync>>));
        #[cfg(test)]
        let worker_tag_hook = tag_hook.clone();
        let (requests, jobs) = mpsc::sync_channel::<Request>(1);
        let (finished, completion) = mpsc::sync_channel(1);
        let changed=Arc::new(AtomicBool::new(false));let worker_changed=changed.clone();
        let watch_ready=Arc::new(AtomicBool::new(false));let worker_watch_ready=watch_ready.clone();
        let worker = std::thread::Builder::new()
            .name("omatainer-library".into())
            .spawn(move || {
                let mut fingerprints = HashMap::new();
                let mut search_pins = Vec::new();
                let mut tag_pins: Vec<Arc<tag_jobs::Reply>> = Vec::new();
                let mut summary_pin:Option<Arc<Summary>>=None;
                let mut watcher=watch::Watcher::default();
                loop {
                    search_pins.retain(|receipt: &Arc<crate::library::relocation_search::Receipt>| Arc::strong_count(receipt) > 1);
                    tag_pins.retain(|reply| Arc::strong_count(reply) > 1);
                    let mut request=match jobs.recv_timeout(std::time::Duration::from_millis(250)) {
                        Ok(request)=>request,
                        Err(mpsc::RecvTimeoutError::Timeout)=>{if watcher.poll(&mut inventory) {worker_changed.store(true,Ordering::Release);}continue;},
                        Err(mpsc::RecvTimeoutError::Disconnected)=>break,
                    };
                    let key=crate::background::identity(&(&request.roots,request.kind==Kind::Import,request.tags.as_ref().map(|job|job.id),request.replacement.as_ref().map(|job|job.id)));
                    let admitted=key.and_then(|key|request.work.background(crate::background::Kind::Index,key,256*crate::background::MIB))
                        .and_then(|ticket|ticket.enter(||request.work.cancelled()).map(|running|(ticket,Some(running))));
                    let (ticket,mut running)=match admitted {
                        Ok(admitted)=>admitted,
                        Err(error)=>{let completion=if let Some(job)=&request.tags {Completion::Tags(job.id,Arc::new(tag_jobs::Reply::Failed{message:error,record:None}))}
                            else if let Some(job)=&request.replacement {Completion::Replacement(job.id,Err(error))}else{Completion::Failed(error)};
                            drop(request);
                            if finished.send(completion).is_err(){break;}continue;},
                    };
                    *request.progress.reporter.lock().unwrap()=ticket.reporter();
                    if let Some(job) = request.tags.take() {
                        #[cfg(test)]
                        {
                            let hook = worker_tag_hook.lock().unwrap().clone();
                            if let Some(hook) = hook { hook(&job.task); }
                        }
                        let result = Arc::new(if tag_pins.len() >= 3 {
                            tag_jobs::Reply::Failed { message: "Close an earlier tag review before continuing".into(), record: None }
                        } else { tag_jobs::run(job.task, &request.work) });
                        if tag_pins.len() < 3 { tag_pins.push(result.clone()); }
                        ticket.progress(1,Some(1));drop(running.take());
                        if finished.send(Completion::Tags(job.id, result)).is_err() { break; }
                        continue;
                    }
                    if let Some(task) = &request.replacement {
                        let result = if search_pins.len() >= 2 {
                            Err("Close an earlier replacement review before searching again".into())
                        } else {
                            crate::library::relocation_search::search_with_inventory(
                                &task.catalog, &task.target, &request.roots, &task.progress,
                                || !request.work.cancelled(), &mut inventory,
                            ).map(Arc::new)
                        };
                        if let Ok(receipt) = &result { search_pins.push(receipt.clone()); }
                        ticket.progress(1,Some(1));drop(running.take());
                        if finished.send(Completion::Replacement(task.id, result)).is_err() { break; }
                        continue;
                    }
                    match traversal::scan_with_inventory(&request, &fingerprints, &mut inventory) {
                        Ok((items, next_fingerprints, summary, roots)) => {
                            let items = Arc::new(items);let roots=roots.map(Arc::new);let summary=Arc::new(summary);
                            drop(running.take());
                            let (retirement, retired) = mpsc::sync_channel(1);
                            if finished
                                .send(Completion::Ready(Publication {
                                    items: items.clone(),
                                    summary: summary.clone(),
                                    roots: roots.clone(),
                                    retirement,
                                    work: request.work.clone(),
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
                                    if request.watch_enabled && request.kind==Kind::Roots {
                                        if let Some(roots)=&roots {watcher.configure(&request,roots,&mut inventory);worker_watch_ready.store(!request.roots.is_empty(),Ordering::Release);}
                                    }
                                }
                                drop(retired);
                            }
                            // The GUI replaces its prior summary before this
                            // acknowledgment; large maps retire only here.
                            let old=summary_pin.replace(summary);drop(old);
                        }
                        Err(ScanFailure::Cancelled) => {
                            drop(running.take());
                            drop(request);
                            if finished.send(Completion::Cancelled).is_err() {
                                break;
                            }
                        }
                        Err(ScanFailure::Io(error)) => {
                            drop(running.take());
                            drop(request);
                            if finished.send(Completion::Failed(error)).is_err() {
                                break;
                            }
                        }
                    }
                }
                drop(summary_pin);
                drop(watcher);
                drop(fingerprints);
                // Receipt rows retire on this worker even when App fields are
                // dropped in a different order during shutdown.
                while !tag_pins.is_empty() {
                    tag_pins.retain(|reply| Arc::strong_count(reply) > 1);
                    if !tag_pins.is_empty() { std::thread::sleep(std::time::Duration::from_millis(10)); }
                }
                while !search_pins.is_empty() {
                    search_pins.retain(|receipt| Arc::strong_count(receipt) > 1);
                    if !search_pins.is_empty() { std::thread::sleep(std::time::Duration::from_millis(10)); }
                }
            });
        match worker {
            Ok(worker) => Self {
                next_search: 0, replacement: None, tag_result: None,
                #[cfg(test)] tag_hook,
                watch_enabled:false,
                watch_ready,
                changed,
                performance: Handle::default(),
                requests: Some(requests),
                completion,
                worker: Some(worker),
                cancel: Arc::new(AtomicBool::new(false)),
                progress: Arc::new(Progress::default()),
                state: ScanState::Idle,
                summary: None,
            },
            Err(error) => Self {
                next_search: 0, replacement: None, tag_result: None,
                #[cfg(test)] tag_hook,
                watch_enabled:false,
                watch_ready,
                changed,
                performance: Handle::default(),
                requests: None,
                completion,
                worker: None,
                cancel: Arc::new(AtomicBool::new(false)),
                progress: Arc::new(Progress::default()),
                state: ScanState::Failed(format!("Could not start library scan: {error}")),
                summary: None,
            },
        }
    }
}

impl LibraryScan {
    #[cfg(test)]
    pub fn set_tag_hook(&mut self, hook: impl Fn(&tag_jobs::Task) + Send + Sync + 'static) {
        *self.tag_hook.lock().unwrap() = Some(Arc::new(hook));
    }
    pub fn enable_watching(&mut self) {self.watch_enabled=true;}
    pub fn take_watch_hint(&self)->bool {self.changed.swap(false,Ordering::AcqRel)}
    pub fn set_performance(&mut self, performance: Handle) {
        self.performance = performance;
    }
    pub fn active(&self) -> bool {
        matches!(self.state, ScanState::Scanning | ScanState::Cancelling | ScanState::Searching | ScanState::SearchCancelling | ScanState::Tags | ScanState::TagsCancelling)
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
        self.admit(roots, baseline, options, Kind::Roots, None)
    }

    pub fn import(&mut self, paths: Vec<PathBuf>, baseline: Arc<Vec<LibItem>>) -> bool {
        self.admit(paths, baseline, Options::default(), Kind::Import, None)
    }

    pub fn import_with_catalog(&mut self, paths:Vec<PathBuf>,baseline:Arc<Vec<LibItem>>,catalog:Arc<crate::library::Catalog>)->bool {
        self.admit(paths,baseline,Options::default(),Kind::Import,Some(Watch {profile:"Explicit import".into(),catalog}))
    }
    pub fn start_watched(&mut self, roots: Vec<PathBuf>, baseline: Arc<Vec<LibItem>>, profile: String, catalog: Arc<crate::library::Catalog>) -> bool {
        self.admit(roots, baseline, Options::default(), Kind::Roots, Some(Watch {profile,catalog}))
    }
    fn admit(&mut self, roots: Vec<PathBuf>, baseline: Arc<Vec<LibItem>>, options: Options, kind: Kind, watch: Option<Watch>) -> bool {
        if self.active() {
            return false;
        }
        if roots.len() > MAX_INPUTS || baseline.len() > MAX_ITEMS {
            self.state = ScanState::Failed("Import/scan exceeds 64 inputs or 100,000 library entries".into());
            return false;
        }
        let work = match self.performance.optional_work() {
            Ok(work) => Arc::new(work),
            Err(error) => {
                self.state = ScanState::Failed(error.to_string());
                return false;
            }
        };
        // A failed worker can be retried explicitly by the Scan button.
        if self.requests.is_none() {
            let performance = self.performance.clone();let watching=self.watch_enabled;let next_search=self.next_search;
            *self = Self::default();
            self.performance = performance;self.watch_enabled=watching;self.next_search=next_search;
        }
        let Some(requests) = &self.requests else {
            return false;
        };
        self.cancel = work.cancel();
        self.progress = Arc::new(Progress::default());
        self.summary = None;
        let request = Request {
            replacement: None, tags: None,
            performance:self.performance.clone(),watch_enabled:self.watch_enabled,
            watch,
            work,
            roots,
            kind,
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
            self.state = if matches!(self.state, ScanState::Tags | ScanState::TagsCancelling) { ScanState::TagsCancelling } else if matches!(self.state, ScanState::Searching | ScanState::SearchCancelling) { ScanState::SearchCancelling } else { ScanState::Cancelling };
        }
    }

    pub fn poll(&mut self) -> Option<Publication> {
        match self.completion.try_recv() {
            Ok(Completion::Tags(id, result)) => {
                // File replacement can already be committed when cancellation
                // arrives. Preserve the actual result for catalog recovery.
                self.state = ScanState::TagsFinished;
                self.tag_result = Some((id, result));
            }
            Ok(Completion::Replacement(id, result)) => {
                let result = if self.cancel.load(Ordering::Acquire) {
                    Err("Replacement search cancelled; library unchanged".into())
                } else { result };
                self.state = ScanState::SearchFinished(match &result {
                    Ok(receipt) => format!("Replacement search: {} matches{}", receipt.matches.len(), if receipt.complete { "" } else { " · incomplete coverage" }),
                    Err(error) => error.clone(),
                });
                self.replacement = Some((id, result));
            }
            Ok(Completion::Ready(publication)) => {
                if self.cancel.load(Ordering::Acquire) {
                    publication.discard();
                    self.state = ScanState::Cancelled;
                } else {
                    self.state = ScanState::Complete(publication.items.len());
                    self.summary = Some(publication.summary.clone());
                    return Some(publication);
                }
            }
            Ok(Completion::Cancelled) => self.state = ScanState::Cancelled,
            Ok(Completion::Failed(error)) => self.state = ScanState::Failed(error),
            Err(mpsc::TryRecvError::Disconnected) if self.active() => {
                if matches!(self.state, ScanState::Tags | ScanState::TagsCancelling) {
                    self.tag_result = Some((self.next_search, Arc::new(tag_jobs::Reply::Failed {
                        message: "Tag filesystem worker stopped; media outcome is unconfirmed. Recovery records and originals are retained.".into(), record: None })));
                }
                self.state = ScanState::Failed("Library scan worker stopped; retry scan".into());
                self.requests = None;
            }
            Err(_) => {}
        }
        None
    }

    pub fn label(&self) -> String {
        match &self.state {
            ScanState::Tags => "Inspecting/saving audio tags…".into(),
            ScanState::TagsCancelling => "Cancelling tag work; any claimed write still reports its result…".into(),
            ScanState::TagsFinished => "Tag operation finished".into(),
            ScanState::Searching => "Searching replacement folders…".into(),
            ScanState::SearchCancelling => "Cancelling replacement search…".into(),
            ScanState::SearchFinished(message) => message.clone(),
            ScanState::Idle => "Ready to scan".into(),
            ScanState::Scanning => {
                let phase = match self.progress.phase.load(Ordering::Relaxed) {
                    1 => "Sorting",
                    2 => "Finishing",
                    _ => "Scanning",
                };
                let reasons = [SkipReason::Unsupported, SkipReason::Unreadable, SkipReason::MissingRoot, SkipReason::Symlink, SkipReason::DepthLimit, SkipReason::Capacity, SkipReason::Duplicate, SkipReason::InvalidPath]
                    .into_iter().filter_map(|reason| { let count = self.progress.reasons[reason as usize].load(Ordering::Relaxed);
                        (count > 0).then(|| format!("{count} {}", reason.label())) }).collect::<Vec<_>>().join(", ");
                format!(
                    "{phase}: {} entries · {} audio files · {} skipped entries{}",
                    self.progress.visited.load(Ordering::Relaxed),
                    self.progress.found.load(Ordering::Relaxed),
                    self.progress.skipped.load(Ordering::Relaxed),
                    if reasons.is_empty() { String::new() } else { format!(" ({reasons})") },
                )
            }
            ScanState::Cancelling => "Cancelling scan…".into(),
            ScanState::Complete(count) => format!("Scan/import complete · {count} tracks · {} skipped entries{}", self.summary.as_ref().map_or(0, |s| s.skipped_count()),
                if self.summary.as_ref().is_some_and(|s| s.truncated) { " · incomplete coverage" } else { "" }) + if self.summary.as_ref().is_some_and(|s|s.watch_limited) {" · directory notifications limited; periodic rescan active"} else {""} + if self.watch_enabled && self.watch_ready.load(Ordering::Acquire) {" · watching folders (30s fallback)"} else {""},
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
) -> Result<(Vec<LibItem>, HashMap<PathBuf, Fingerprint>, Summary), ScanFailure> {
    traversal::scan_with_inventory(request, previous_fingerprints, &mut crate::media_location::Snapshot::discover)
        .map(|(items, fingerprints, summary, _)| (items, fingerprints, summary))
}

fn preserve_metadata(item: &mut LibItem, old: &LibItem) {
    if !old.title.is_empty() {
        item.title.clone_from(&old.title);
    }
    if !old.artist.is_empty() {
        item.artist.clone_from(&old.artist);
    }
    if old.fingerprint == item.fingerprint {
        item.bpm = old.bpm;
    }
    if !old.key.is_empty() && old.key != "—" {
        item.key.clone_from(&old.key);
    }
    if old.fingerprint == item.fingerprint {
        item.length = old
            .length
            .filter(|value| value.is_finite() && *value >= 0.0);
    }
    // The worker's previous-scan cache can be empty (first scan, or a source
    // returning after removal). A pathname alone must never transfer history
    // to different bytes; require the same verified identity in the baseline.
    let same_media = item.source == old.source
        && match item.source {
            LibSource::Builtin(_) => true,
            LibSource::Removable { .. } | LibSource::Provider { .. } => false,
            LibSource::File(_) => item.fingerprint.is_some() && item.fingerprint == old.fingerprint,
        };
    if same_media {
        item.last_play = old.last_play;
    }
}
