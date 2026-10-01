//! One low-priority durability worker. Callback capture uses the existing bounded
//! project handoff; encoding, checksums, media retirement and fsync stay here.
use super::*;
use arc_swap::{ArcSwap, ArcSwapOption};
use crossbeam_channel::{bounded, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone)]
pub(super) struct Update {
    pub epoch: u64,
    pub view_revision: u64,
    pub view: project::UiState,
    pub identities: Vec<project::WatchIdentity>,
    pub saved_path: Option<PathBuf>,
    pub config: crate::recovery::Config,
}
struct Control {
    paused: bool,
    cancel: Arc<AtomicBool>,
}
impl Control {
    fn new(paused: bool) -> Self {
        Self {
            paused,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}
#[derive(Clone, Default)]
pub(super) struct Status {
    pub busy: bool,
    pub warning: bool,
    pub message: String,
    pub durable: Option<Durable>,
    pub usage_bytes: Option<u64>,
}
#[derive(Clone)]
pub(super) struct Durable {
    pub session: [u8;32],
    pub epoch: u64,
    pub revision: u64,
    pub view_revision: u64,
    pub sequence: u64,
    pub captured_unix_ms: u64,
    pub committed_unix_ms: u64,
}
pub(super) struct Preview {
    pub candidate: crate::recovery::Candidate,
    pub notes: usize,
    pub media: usize,
    pub bpm: f32,
    pub report: Vec<String>,
}
pub(super) enum Job {
    List(crate::engine::performance::WorkPermit),
    Inspect(
        crate::recovery::Candidate,
        crate::engine::performance::WorkPermit,
    ),
    Remove(
        crate::recovery::Candidate,
        crate::engine::performance::WorkPermit,
    ),
    Retire,
}
pub(super) enum Event {
    Listed(
        Result<crate::recovery::Inventory, String>,
        Option<crate::engine::performance::WorkPermit>,
    ),
    Inspected(Result<(Preview, crate::engine::performance::WorkPermit), String>),
    Removed(Result<Option<String>, String>),
    Retired(Result<Option<String>, String>),
}
pub(super) struct Output {
    pub id: u64,
    pub event: Event,
}
#[cfg(test)]
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Stage {
    BeforeAppend,
    BeforeInspect,
    AfterRetire,
    AfterRemove,
}
#[cfg(test)]
type Hooks = Arc<parking_lot::Mutex<Option<(Stage, Sender<()>, Receiver<()>)>>>;
#[cfg(test)]
fn pause_at(hooks: &Hooks, stage: Stage) {
    let wait = {
        let mut value = hooks.lock();
        if value.as_ref().is_some_and(|value| value.0 == stage) {
            value.take()
        } else {
            None
        }
    };
    if let Some((_, entered, resume)) = wait {
        let _ = entered.send(());
        let _ = resume.recv_timeout(std::time::Duration::from_secs(10));
    }
}
#[cfg(test)]
struct Finish(Sender<()>);
#[cfg(test)]
impl Drop for Finish {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
pub(super) struct Worker {
    #[cfg(test)]
    hooks: Hooks,
    #[cfg(test)]
    finished: Receiver<()>,
    latest: Arc<ArcSwapOption<Update>>,
    control: Arc<ArcSwap<Control>>,
    force: Arc<AtomicBool>,
    reset: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    capture_active: Arc<AtomicBool>,
    jobs: Sender<(u64, Job, Arc<Control>)>,
    events: Receiver<Output>,
    next_job: std::cell::Cell<u64>,
    status: Arc<ArcSwap<Status>>,
}
impl Worker {
    pub fn start(
        root: PathBuf,
        handle: crate::engine::project::Handle,
        performance: &crate::engine::performance::Handle,
    ) -> std::io::Result<Self> {
        Self::start_with_scan(root,handle,performance,true)
    }
    pub fn start_with_scan(root:PathBuf,handle:crate::engine::project::Handle,performance:&crate::engine::performance::Handle,scan:bool)->std::io::Result<Self> {
        #[cfg(test)]
        let hooks = Hooks::default();
        #[cfg(test)]
        let thread_hooks = hooks.clone();
        #[cfg(test)]
        let (finished, finish) = bounded(1);
        let latest = Arc::new(ArcSwapOption::<Update>::empty());
        let startup_work = scan.then(||performance.optional_work());
        let startup_cancel = startup_work
            .as_ref().and_then(|work|work.as_ref().ok())
            .map(|work| work.cancel())
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let control = Arc::new(ArcSwap::from_pointee(Control {
            paused: false,
            cancel: startup_cancel,
        }));
        let force = Arc::new(AtomicBool::new(false));
        let reset = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let capture_active = Arc::new(AtomicBool::new(false));
        let status = Arc::new(ArcSwap::from_pointee(Status::default()));
        let (jobs, incoming) = bounded(1);
        let (outgoing, events) = bounded(2);
        let worker = Self {
            #[cfg(test)]
            hooks,
            #[cfg(test)]
            finished: finish,
            latest: latest.clone(),
            control: control.clone(),
            force: force.clone(),
            reset: reset.clone(),
            stop: stop.clone(),
            capture_active: capture_active.clone(),
            jobs,
            events,
            next_job: std::cell::Cell::new(1),
            status: status.clone(),
        };
        std::thread::Builder::new()
            .name("omatainer-recovery".into())
            .spawn(move || {
                #[cfg(test)]
                let _finish = Finish(finished);
                #[cfg(target_os = "linux")]
                // Lower only this application worker's scheduling priority. Failure
                // cannot compromise correctness or prevent a durable write.
                unsafe {
                    libc::setpriority(
                        libc::PRIO_PROCESS,
                        libc::syscall(libc::SYS_gettid) as u32,
                        10,
                    );
                }
                let mut sessions = Sessions::new(root.clone());
                let mut current = Status::default();
                let mut next = Instant::now();
                let mut last_captured: Option<(u64, u64, u64)> = None;
                current.busy = true;
                current.message = "Checking recovery availability; the first new durable record is pending".into();
                status.store(Arc::new(current.clone()));
                let startup_token = control.load_full();
                let (startup, startup_work) = match startup_work {
                    Some(Ok(work)) => (crate::recovery::discover(&root, &work.cancel()).map_err(|e| e.to_string()), Some(work)),
                    Some(Err(error)) => {
                        // A single metadata lookup is the essential startup notice.
                        // Full journal/PCM verification is optional guarded work.
                        let warnings = if std::fs::symlink_metadata(&root).is_ok() {
                            vec![format!("Recovery scan deferred by performance protection: {error}. Copies may exist. Leave protection and Refresh recovery list to verify them.")]
                        } else { Vec::new() };
                        (Ok(crate::recovery::Inventory { warnings, ..Default::default() }), None)
                    }
                    None => (Ok(crate::recovery::Inventory{warnings:vec!["Safe mode: recovery discovery is deferred. Use Refresh recovery list to explicitly verify retained copies.".into()],..Default::default()}),None),
                };
                control.rcu(|current| if Arc::ptr_eq(current, &startup_token) { Arc::new(Control::new(current.paused)) } else { current.clone() });
                if outgoing.send(Output { id: 0, event: Event::Listed(startup, startup_work) }).is_err() { return; }
                current.busy = false;
                current.message = "Waiting for the next coherent edit-state record".into();
                status.store(Arc::new(current.clone()));
                loop {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    match incoming.recv_timeout(std::time::Duration::from_millis(25)) {
                        Ok((id, job, token)) => {
                            let event = match job {
                                Job::List(work) => Event::Listed(
                                    crate::recovery::discover(&root, &token.cancel).map_err(|e| e.to_string()), Some(work)),
                                Job::Inspect(candidate, work) => {
                                    #[cfg(test)] pause_at(&thread_hooks, Stage::BeforeInspect);
                                    Event::Inspected(inspect(candidate, &token.cancel).map(|preview| (preview, work)))
                                }
                                Job::Remove(candidate, work) => {
                                    let result = work.commit().map_err(|e| e.to_string()).and_then(|_publication|
                                        crate::recovery::discard(&candidate, &token.cancel).map_err(|e| e.to_string()));
                                    #[cfg(test)] pause_at(&thread_hooks, Stage::AfterRemove);
                                    Event::Removed(result)
                                }
                                Job::Retire => {
                                    let result = sessions.retire(&token.cancel);
                                    #[cfg(test)]
                                    pause_at(&thread_hooks, Stage::AfterRetire);
                                    last_captured = None;
                                    if result.is_ok() {
                                        current.durable = None;
                                        current.message =
                                            "Previous recovery session intentionally retired"
                                                .into();
                                        status.store(Arc::new(current.clone()));
                                    }
                                    Event::Retired(result)
                                }
                            };
                            control.rcu(|current| {
                                if Arc::ptr_eq(current, &token) { Arc::new(Control::new(current.paused)) }
                                else { current.clone() }
                            });
                            if outgoing.send(Output { id, event }).is_err() {
                                return;
                            }
                            continue;
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    }
                    if reset.swap(false, Ordering::AcqRel) {
                        sessions.current = None;
                        sessions.prior = None;
                        last_captured = None;
                        current.durable = None;
                        current.message = "Recovery resumed in a new session".into();
                        status.store(Arc::new(current.clone()));
                    }
                    let token = control.load_full();
                    if token.paused || token.cancel.load(Ordering::Acquire) {
                        continue;
                    }
                    let Some(update) = latest.load_full() else {
                        continue;
                    };
                    let requested = (update.epoch, handle.revision(), update.view_revision);
                    let forced = force.swap(false, Ordering::AcqRel);
                    if !forced && (Instant::now() < next || last_captured == Some(requested)) {
                        continue;
                    }
                    current.busy = true;
                    current.message =
                        "Capturing and journaling edit state in the background".into();
                    status.store(Arc::new(current.clone()));
                    let result = capture_and_append(
                        &handle,
                        &update,
                        &mut sessions,
                        &token.cancel,
                        &capture_active,
                        #[cfg(test)]
                        &thread_hooks,
                    );
                    current.busy = false;
                    next = Instant::now() + crate::recovery::JOURNAL_INTERVAL;
                    match result {
                        Ok((commit, revision, captured_unix_ms)) => {
                            // Cancellation after publication does not erase a real
                            // commit; the durability outcome comes from the writer.
                            current.usage_bytes = Some(commit.usage_bytes);
                            current.warning = !commit.durable || commit.warning.is_some();
                            if commit.durable {
                                current.durable = Some(Durable {
                                    session: crate::recovery::session_digest(sessions.current.as_ref().unwrap().1.session_id()),
                                    epoch: update.epoch,
                                    revision,
                                    view_revision: update.view_revision,
                                    sequence: commit.sequence,
                                    captured_unix_ms,
                                    committed_unix_ms: commit.committed_unix_ms,
                                });
                                last_captured =
                                    Some((update.epoch, revision, update.view_revision));
                                current.message = "Edit-state journal confirmed durable".into();
                            } else {
                                current.message =
                                    "Journal committed; storage durability could not be confirmed"
                                        .into();
                                next = Instant::now() + std::time::Duration::from_secs(10);
                            }
                            if let Some(warning) = commit.warning {
                                current.message.push_str(&format!(". {warning}"));
                            }
                        }
                        Err(_) if token.cancel.load(Ordering::Acquire) => {
                            current.message = "Recovery capture paused for the requested operation; prior durable state retained".into();
                            next = Instant::now();
                        }
                        Err(error) => {
                            current.warning = true;
                            current.message =
                                format!("Recovery has not saved newer edits: {error}");
                            next = Instant::now() + std::time::Duration::from_secs(10);
                        }
                    }
                    status.store(Arc::new(current.clone()));
                }
            })?;
        Ok(worker)
    }
    #[cfg(test)]
    pub fn pause_next(&self, stage: Stage) -> (Receiver<()>, Sender<()>) {
        let (entered, observed) = bounded(1);
        let (resume, waiting) = bounded(1);
        *self.hooks.lock() = Some((stage, entered, waiting));
        (observed, resume)
    }
    #[cfg(test)]
    pub fn stop_for_test(&self) {
        self.stop.store(true, Ordering::Release);
        self.cancel();
        self.finished
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("recovery worker did not stop");
    }
    pub fn update(&self, update: Update) {
        self.latest.store(Some(Arc::new(update)));
    }
    pub fn pause(&self, paused: bool) {
        self.control.rcu(|old| {
            if old.paused == paused && !old.cancel.load(Ordering::Acquire) {
                return old.clone();
            }
            old.cancel.store(true, Ordering::SeqCst);
            Arc::new(Control::new(paused))
        });
    }
    pub fn capture_fence(&self) -> Arc<AtomicBool> {
        self.capture_active.clone()
    }
    pub fn cancel(&self) {
        self.control.load().cancel.store(true, Ordering::SeqCst);
    }
    pub fn request(&self, job: Job) -> Result<u64, String> {
        let cancel = match &job {
            Job::List(work) | Job::Inspect(_, work) | Job::Remove(_, work) => work.cancel(),
            _ => Arc::new(AtomicBool::new(false)),
        };
        self.control.rcu(|old| {
            old.cancel.store(true, Ordering::SeqCst);
            Arc::new(Control {
                paused: old.paused,
                cancel: cancel.clone(),
            })
        });
        let token = self.control.load_full();
        let id = self.next_job.get();
        self.next_job.set(
            id.checked_add(1)
                .ok_or("Recovery request identity exhausted")?,
        );
        self.jobs
            .try_send((id, job, token))
            .map(|_| id)
            .map_err(|_| {
                "Recovery worker is busy or unavailable; retry after the current result".into()
            })
    }
    pub fn resume_after_close(&self) {
        self.cancel();
        self.reset.store(true, Ordering::Release);
        self.pause(false);
        self.force.store(true, Ordering::Release);
    }
    pub fn force(&self) {
        self.pause(false);
        self.force.store(true, Ordering::Release);
    }
    pub fn poll(&self) -> Option<Output> {
        self.events.try_recv().ok()
    }
    pub fn status(&self) -> Arc<Status> {
        self.status.load_full()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.cancel();
    }
}
struct Sessions {
    root: PathBuf,
    current: Option<(u64, crate::recovery::Store)>,
    prior: Option<(u64, crate::recovery::Store)>,
}
impl Sessions {
    fn new(root: PathBuf) -> Self {
        Self {
            root,
            current: None,
            prior: None,
        }
    }
    fn store(&mut self, epoch: u64) -> Result<&mut crate::recovery::Store, String> {
        if self
            .current
            .as_ref()
            .is_none_or(|(current, _)| *current != epoch)
        {
            // Keep one previously useful session while a first write for the
            // newer epoch is still uncommitted. Never retire it just to rotate.
            if self.prior.is_none() {
                self.prior = self.current.take();
            } else {
                self.current.take();
            }
            self.current = Some((
                epoch,
                crate::recovery::Store::open(&self.root).map_err(|e| e.to_string())?,
            ));
        }
        Ok(&mut self.current.as_mut().unwrap().1)
    }
    fn restart_session(&mut self) {
        // Preserve any useful prefix of a failed journal. Releasing an older
        // extra session only makes it discoverable; it never deletes its data.
        if self.prior.is_none() {
            self.prior = self.current.take();
        } else {
            self.current = None;
        }
    }
    fn retire_prior(&mut self, cancel: &AtomicBool) -> Result<Option<String>, String> {
        let warning = if let Some((epoch, store)) = &mut self.prior {
            store
                .retire_epoch(*epoch, cancel)
                .map_err(|e| e.to_string())?
        } else {
            None
        };
        if warning.is_none() {
            self.prior = None;
        }
        Ok(warning)
    }
    fn retire(&mut self, cancel: &AtomicBool) -> Result<Option<String>, String> {
        let warning = if let Some((epoch, store)) = &mut self.current {
            store
                .retire_epoch(*epoch, cancel)
                .map_err(|e| e.to_string())?
        } else {
            None
        };
        let prior_warning = self.retire_prior(cancel)?;
        let warning = match (warning, prior_warning) {
            (Some(a), Some(b)) => Some(format!("{a}; {b}")),
            (a, b) => a.or(b),
        };
        // A warning can be retried before exit. Cancel-close uses reset() to
        // open a fresh session even when its retirement marker was committed.
        if warning.is_none() {
            self.current = None;
        }
        Ok(warning)
    }
}
fn capture_and_append(
    handle: &crate::engine::project::Handle,
    update: &Update,
    sessions: &mut Sessions,
    cancel: &AtomicBool,
    capture_active: &AtomicBool,
    #[cfg(test)] hooks: &Hooks,
) -> Result<(crate::recovery::Commit, u64, u64), String> {
    // SC ordering pairs GUI cancellation with its subsequent fence observation:
    // either it waits for this claim or this claim observes cancellation before
    // entering the engine's single capture exchange.
    let captured_unix_ms = unix_ms();
    capture_active.store(true, Ordering::SeqCst);
    let captured = if cancel.load(Ordering::SeqCst) {
        Err(crate::engine::project::Error::Cancelled)
    } else {
        handle.capture(cancel)
    };
    capture_active.store(false, Ordering::SeqCst);
    let captured = captured.map_err(|e| e.to_string())?;
    if captured.checkpoint.epoch != update.epoch {
        return Err("Document epoch changed; stale UI state was not journaled".into());
    }
    let mut view = update.view.clone();
    view.deck_identities = std::array::from_fn(|deck| {
        captured.playback_receipts[deck]
            .as_ref()
            .and_then(|receipt| {
                update
                    .identities
                    .iter()
                    .find(|identity| identity.receipt.same_request(receipt))
                    .map(|identity| identity.identity.clone())
            })
    });
    let state = project::Document {
        engine: captured.state,
        view,
        mapping_schema: project::FACTORY_MAPPING_SCHEMA,
    };
    state.validate()?;
    let revision = captured.revision;
    let metadata = crate::recovery::RecordMeta {
        epoch: update.epoch,
        revision,
        view_revision: update.view_revision,
        saved_path: update.saved_path.clone(),
        captured_unix_ms,
    };
    #[cfg(test)]
    pause_at(hooks, Stage::BeforeAppend);
    let store = sessions.store(update.epoch)?;
    let result = store.append(
        &crate::project_file::Bundle {
            state,
            media: captured.media,
        },
        metadata,
        &update.config,
        cancel,
    );
    let needs_new_session = store.needs_new_session();
    let mut commit = match result {
        Ok(commit) => commit,
        Err(error) => {
            if needs_new_session {
                sessions.restart_session();
            }
            return Err(error.to_string());
        }
    };
    if commit.durable {
        // The newer session is already durable even if retirement of its
        // predecessor fails; keep that failure as a warning, never a false save failure.
        let warning = match sessions.retire_prior(cancel) {
            Ok(warning) => warning,
            Err(error) => Some(format!("Prior recovery session retained: {error}")),
        };
        if let Some(warning) = warning {
            commit.warning = Some(
                commit
                    .warning
                    .map_or(warning.clone(), |old| format!("{old}; {warning}")),
            );
        }
    }
    Ok((commit, revision, captured_unix_ms))
}
fn inspect(candidate: crate::recovery::Candidate, cancel: &AtomicBool) -> Result<Preview, String> {
    let recovered = crate::recovery::recover::<project::Document>(&candidate, cancel)
        .map_err(|e| e.to_string())?;
    recovered.bundle.state.validate()?;
    recovered
        .bundle
        .state
        .engine
        .validate(&recovered.bundle.media)?;
    Ok(Preview {
        candidate,
        notes: recovered
            .bundle
            .state
            .engine
            .tracks
            .iter()
            .flat_map(|track| &track.clips)
            .map(|clip| clip.notes.len())
            .sum(),
        media: recovered.bundle.media.len(),
        bpm: recovered.bundle.state.engine.bpm,
        report: recovered.report,
    })
}
pub(super) fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
