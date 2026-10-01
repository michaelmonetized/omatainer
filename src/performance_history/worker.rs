//! Sole history writer and bounded consumer of renderer-confirmed observations.
use super::{Session, Source, State, storage::{self, Commit, Store}};
use crate::engine::{history_measurement::{Observation, control::{Action, Handle, Outcome, Progress}}, performance::{self, WorkPermit}};
use crossbeam_channel::{Receiver, Sender};
use std::{collections::{BTreeSet, HashMap, VecDeque}, path::PathBuf,
    sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{Duration, Instant}};

const REGISTRATIONS: usize = 8192;
#[derive(Clone, Debug)]
pub(crate) struct Summary { pub id: String, pub started_ns: u64, pub state: State, pub entries: usize }
#[derive(Clone, Debug)]
pub(crate) struct Receipt { pub id: u64, pub applied: bool, pub message: String }
#[derive(Clone, Debug, Default)]
pub(crate) struct View {
    pub sessions: Vec<Summary>, pub selected: Option<Session>, pub active: Option<String>,
    pub ready: bool, pub durable: bool, pub durable_at_ns: Option<u64>,
    pub message: String, pub receipt: Option<Receipt>,
}
pub(crate) enum Job {
    Start, End, Select(String), Retry, Close, KeepWorking,
    Mark { session: String, revision: u64, entry: u32, played: Option<bool> },
    External { session: String, revision: u64, title: String, artist: String },
    Export { session: String, path: PathBuf },
}
impl Job { fn optional(&self) -> bool { matches!(self, Self::Mark { .. } | Self::External { .. } | Self::Export { .. }) } }
struct Request { id: u64, job: Job, permit: Option<WorkPermit> }
enum Boundary { Start { id: String, nonce: u64 }, End { id: String, closing: bool } }
struct Pending { request: u64, renderer_request: u64, boundary: Boundary }

pub(crate) struct Worker {
    jobs: Sender<Request>, registrations: Sender<(u64, Source)>, views: Receiver<Arc<View>>,
    retirement: Sender<Arc<View>>, pending_retirement: Option<Arc<View>>,
    view: Arc<View>, pending: Option<u64>, next: u64, show: performance::Handle,
    stop: Arc<AtomicBool>, alive: Arc<AtomicBool>,
}
impl Worker {
    pub fn start(root: PathBuf, renderer: Option<Handle>, show: performance::Handle) -> Result<Self, String> {
        let observations = renderer.as_ref().map(|h| h.take_observations().ok_or("history observer already has a consumer")).transpose()?;
        let (jobs, requests) = crossbeam_channel::bounded(1);
        let (registrations, incoming) = crossbeam_channel::bounded(256);
        let (published, views) = crossbeam_channel::bounded(1);
        let (retirement, retired) = crossbeam_channel::bounded(4);
        let stop = Arc::new(AtomicBool::new(false)); let alive = Arc::new(AtomicBool::new(true));
        let stopped = stop.clone(); let connected = alive.clone();
        std::thread::Builder::new().name("performance-history".into()).spawn(move || {
            struct Alive(Arc<AtomicBool>); impl Drop for Alive { fn drop(&mut self) { self.0.store(false, Ordering::Release); } }
            let _alive = Alive(connected);
            let mut owner = Owner::new(root, renderer, observations);
            let mut publish_at = Instant::now();
            while !stopped.load(Ordering::Acquire) {
                for old in retired.try_iter() { drop(old); }
                for (key, source) in incoming.try_iter().take(256) { owner.register(key, source); }
                owner.collect();
                match requests.recv_timeout(Duration::from_millis(10)) {
                    Ok(request) => owner.request(request),
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {},
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                }
                owner.save_due(false);
                if owner.publish && (publish_at.elapsed() >= Duration::from_millis(250) || owner.urgent) && !published.is_full() {
                    if published.try_send(Arc::new(owner.view())).is_ok() { owner.publish = false; owner.urgent = false; publish_at = Instant::now(); }
                }
            }
            // Abrupt owner loss can retain only the already-confirmed prefix.
            // Normal app exit uses Close and waits for its durable receipt.
            owner.collect();
            if let Some(id) = owner.active.take() {
                if let Some(store) = &mut owner.store {
                    if let Some(session) = store.sessions.get_mut(&id) { session.recover_unclean(); owner.dirty.insert(id); }
                }
            }
            owner.save_due(true);
            for old in retired.try_iter() { drop(old); }
        }).map_err(|e| e.to_string())?;
        Ok(Self { jobs, registrations, views, retirement, pending_retirement: None,
            view: Arc::new(View { message: "Opening performance history…".into(), ..Default::default() }),
            pending: None, next: 1, show, stop, alive })
    }
    pub fn submit(&mut self, job: Job) -> Result<u64, String> {
        if self.pending.is_some() { return Err("another history action is pending".into()); }
        if !self.alive.load(Ordering::Acquire) { return Err("history worker disconnected".into()); }
        if let Job::External { title, artist, .. } = &job { Source::External { title: title.clone(), artist: artist.clone() }.validate()?; }
        let permit = if job.optional() { Some(self.show.optional_work().map_err(|e| e.to_string())?) } else { None };
        let id = self.next; self.next = self.next.checked_add(1).ok_or("history request IDs exhausted")?;
        self.jobs.try_send(Request { id, job, permit }).map_err(|_| "history worker request unavailable")?;
        self.pending = Some(id); Ok(id)
    }
    pub fn register(&self, key: u64, source: Source) -> Result<(), String> {
        if key == 0 || matches!(source, Source::External { .. }) { return Err("invalid history load registration".into()); }
        source.validate()?;
        self.registrations.try_send((key, source)).map_err(|_| "history registration queue unavailable".into())
    }
    pub fn poll(&mut self) -> &View {
        if let Some(old) = self.pending_retirement.take() {
            if let Err(error) = self.retirement.try_send(old) { self.pending_retirement = Some(error.into_inner()); return &self.view; }
        }
        if let Ok(view) = self.views.try_recv() {
            let old = std::mem::replace(&mut self.view, view);
            if let Err(error) = self.retirement.try_send(old) { self.pending_retirement = Some(error.into_inner()); }
            if self.view.receipt.as_ref().is_some_and(|receipt| Some(receipt.id) == self.pending) { self.pending = None; }
        }
        &self.view
    }
    pub fn view(&self) -> &View { &self.view }
    pub fn pending(&self) -> bool { self.pending.is_some() }
    pub fn alive(&self) -> bool { self.alive.load(Ordering::Acquire) }
}
impl Drop for Worker { fn drop(&mut self) { self.stop.store(true, Ordering::Release); } }

struct Owner {
    root: PathBuf, store: Option<Store>, renderer: Option<Handle>, observations: Option<Receiver<Observation>>,
    registrations: HashMap<u64, Source>, unresolved: HashMap<u64, BTreeSet<(String, u8)>>, staged: VecDeque<Observation>, pending: Option<Pending>,
    active: Option<String>, selected: Option<String>, dirty: BTreeSet<String>,
    message: String, receipt: Option<Receipt>, durable_at_ns: Option<u64>, save_at: Instant,
    publish: bool, urgent: bool, closing: bool,
}
impl Owner {
    fn new(root: PathBuf, renderer: Option<Handle>, observations: Option<Receiver<Observation>>) -> Self {
        let (store, message) = match Store::open(root.clone()) {
            Ok(store) => (Some(store), "Performance history ready".into()), Err(error) => (None, format!("History unavailable: {error}")),
        };
        let selected = store.as_ref().and_then(|s| s.sessions.values().max_by_key(|s| s.started_ns).map(|s| s.id.clone()));
        Self { root, store, renderer, observations, registrations: HashMap::new(), unresolved: HashMap::new(), staged: VecDeque::with_capacity(4096),
            pending: None, active: None, selected, dirty: BTreeSet::new(), message, receipt: None,
            durable_at_ns: None, save_at: Instant::now(), publish: true, urgent: true, closing: false }
    }
    fn register(&mut self, key: u64, source: Source) {
        if self.registrations.get(&key).is_some_and(|known| *known != Source::Unresolved && source != Source::Unresolved && known != &source) {
            self.message = "Conflicting history source registration was rejected".into(); self.publish = true; return;
        }
        if self.registrations.len() >= REGISTRATIONS && !self.registrations.contains_key(&key) {
            self.message = "History source-registration limit reached; further sources remain unresolved".into(); self.publish = true; return;
        }
        if source == Source::Unresolved && self.registrations.contains_key(&key) { return; }
        self.registrations.insert(key, source.clone());
        if source != Source::Unresolved {
            if let Some(waiting) = self.unresolved.remove(&key) {
                if let Some(store) = &mut self.store {
                    for (id, deck) in waiting {
                        if let Some(session) = store.sessions.get_mut(&id) {
                            if session.loaded(key, deck, source.clone()).is_ok() { self.dirty.insert(id); self.publish = true; }
                        }
                    }
                }
            }
        }
    }
    fn remember_unresolved(&mut self, key: u64, deck: u8, id: &str, source: &Source) {
        if *source != Source::Unresolved { return; }
        if self.unresolved.len() < REGISTRATIONS || self.unresolved.contains_key(&key) {
            self.unresolved.entry(key).or_default().insert((id.to_owned(), deck));
        }
    }
    fn source(&self, key: u64) -> Source { self.registrations.get(&key).cloned().unwrap_or(Source::Unresolved) }
    fn finish(&mut self, id: u64, result: Result<String, String>) {
        let (applied, message) = match result { Ok(message) => (true, message), Err(error) => (false, error) };
        self.message = message.clone(); self.receipt = Some(Receipt { id, applied, message }); self.publish = true; self.urgent = true;
    }
    fn request(&mut self, request: Request) {
        if self.pending.is_some() { self.finish(request.id, Err("a renderer history boundary is still pending".into())); return; }
        let result = self.apply_request(request.id, request.job, request.permit);
        if let Some(result) = result { self.finish(request.id, result); }
    }
    fn apply_request(&mut self, request: u64, job: Job, permit: Option<WorkPermit>) -> Option<Result<String, String>> {
        let result = (|| -> Result<Option<String>, String> {
            if self.closing && matches!(job, Job::Start | Job::Mark { .. } | Job::External { .. }) { return Err("history is closing".into()); }
            match job {
                Job::KeepWorking => { self.closing = false; return Ok(Some("History close cancelled; ended sessions remain ended".into())); }
                Job::Retry => {
                    if self.store.is_none() { self.store = Some(Store::open(self.root.clone())?); }
                    self.save_due(true);
                    if !self.dirty.is_empty() { return Err(self.message.clone()); }
                    return Ok(Some("History save confirmed".into()));
                }
                Job::Select(id) => {
                    if !self.store.as_ref().is_some_and(|s| s.sessions.contains_key(&id)) { return Err("history session no longer exists".into()); }
                    self.selected = Some(id); return Ok(Some("History session selected".into()));
                }
                Job::Start => {
                    if self.closing { return Err("history is closing".into()); }
                    if self.active.is_some() { return Err("a performance session is already active".into()); }
                    if !self.dirty.is_empty() { return Err("confirm the pending history save before starting another session".into()); }
                    let store = self.store.as_ref().ok_or_else(|| self.message.clone())?;
                    if store.sessions.len() >= storage::MAX_SESSIONS { return Err("history reached its 1024-session limit".into()); }
                    let renderer = self.renderer.as_ref().ok_or("history measurement unavailable")?;
                    let nonce = renderer.new_session().map_err(str::to_owned)?;
                    let id = storage::new_id()?;
                    let renderer_request = renderer.submit(Action::Start(nonce)).map_err(str::to_owned)?;
                    self.pending = Some(Pending { request, renderer_request, boundary: Boundary::Start { id, nonce } });
                    self.message = "Waiting for audio output to confirm the start of history…".into(); self.publish = true; return Ok(None);
                }
                Job::End | Job::Close => {
                    let closing = matches!(job, Job::Close); self.closing |= closing;
                    if let Some(id) = self.active.clone() {
                        let session = &self.store.as_ref().ok_or("history store unavailable")?.sessions[&id];
                        let renderer = self.renderer.as_ref().ok_or("history measurement unavailable")?;
                        let renderer_request = renderer.submit(Action::End(session.renderer_session)).map_err(str::to_owned)?;
                        self.pending = Some(Pending { request, renderer_request, boundary: Boundary::End { id, closing } });
                        self.message = "Waiting for audio output to confirm the end of history…".into(); self.publish = true; return Ok(None);
                    }
                    self.save_due(true);
                    if !self.dirty.is_empty() { return Err(self.message.clone()); }
                    return Ok(Some(if self.store.is_some() { "History is ended and saved" } else { "History storage unavailable; no session was accepted" }.into()));
                }
                Job::Mark { session, revision, entry, played } => {
                    let store = self.store.as_mut().ok_or("history store unavailable")?;
                    let mut candidate = store.sessions.get(&session).ok_or("history session no longer exists")?.clone();
                    candidate.mark(revision, entry, played)?;
                    let result = store.save_optional(&candidate, permit.as_ref().ok_or("history edit permit missing")?)?;
                    self.commit_result(&session, result);
                }
                Job::External { session, revision, title, artist } => {
                    let store = self.store.as_mut().ok_or("history store unavailable")?;
                    let mut candidate = store.sessions.get(&session).ok_or("history session no longer exists")?.clone();
                    candidate.external(revision, title, artist)?;
                    let result = store.save_optional(&candidate, permit.as_ref().ok_or("history edit permit missing")?)?;
                    self.commit_result(&session, result);
                }
                Job::Export { session, path } => {
                    let store = self.store.as_ref().ok_or("history store unavailable")?;
                    let session = store.sessions.get(&session).ok_or("history session no longer exists")?;
                    return Ok(Some(match store.export(session, &path, permit.as_ref().ok_or("history export permit missing")?)? {
                        Commit::Durable => "History exported; existing files were preserved".into(),
                        Commit::CommittedUnconfirmed(warning) => format!("History export committed, durability unconfirmed: {warning}"),
                    }));
                }
            }
            Ok(Some(self.message.clone()))
        })();
        match result { Ok(None) => None, Ok(Some(message)) => Some(Ok(message)), Err(error) => Some(Err(error)) }
    }
    fn collect(&mut self) {
        let progress = self.renderer.as_ref().and_then(Handle::progress);
        let ack = self.pending.as_ref().and_then(|pending| self.renderer.as_ref().map(|renderer| renderer.poll(pending.renderer_request)));
        let mut ending = None;
        if let Some(result) = ack {
            match result {
                Ok(Some(ack)) => {
                    let pending = self.pending.take().unwrap();
                    match (&pending.boundary, ack.outcome) {
                        (Boundary::Start { id, nonce }, Outcome::Started) => {
                            let result = Session::new(id.clone(), *nonce, ack.wall_ns, ack.frame).and_then(|mut session| {
                                for (deck, key) in ack.current.into_iter().enumerate().filter(|(_, key)| *key != 0) {
                                    let source = self.source(key); self.remember_unresolved(key, deck as u8, id, &source);
                                    session.loaded(key, deck as u8, source)?;
                                }
                                session.incomplete |= ack.incomplete;
                                self.store.as_mut().ok_or("history store unavailable")?.sessions.insert(id.clone(), session);
                                self.active = Some(id.clone()); self.selected = Some(id.clone()); self.dirty.insert(id.clone()); Ok("Performance session started".into())
                            });
                            self.finish(pending.request, result);
                        }
                        (Boundary::End { .. }, Outcome::Ended) => ending = Some((pending, ack)),
                        _ => self.finish(pending.request, Err(match ack.outcome {
                            Outcome::NoOutput => "History did not start: no audio output callback is running",
                            Outcome::Unavailable => "History measurement is unavailable for this audio output",
                            Outcome::AlreadyActive => "A history session is already recording",
                            Outcome::WrongSession => "The requested history session is no longer recording",
                            _ => "The requested history boundary was not applied",
                        }.into())),
                    }
                }
                Err(error) => { let pending = self.pending.take().unwrap(); self.finish(pending.request, Err(error.into())); },
                Ok(None) => {},
            }
        }
        let fence = ending.as_ref().map(|(_, ack)| Progress { session: ack.session, wall_ns: ack.wall_ns,
            frame: ack.frame, rate: ack.rate, incomplete: ack.incomplete, dropped: ack.dropped }).or(progress);
        if let (Some(fence), Some(id)) = (fence, self.active.clone()) {
            let matching = self.store.as_ref().and_then(|store| store.sessions.get(&id)).is_some_and(|session| session.renderer_session == fence.session);
            if matching {
                // At most one future event is held ahead of the coherent
                // prefix. Drain all earlier events before publishing coverage;
                // a queued End includes its flushed partial window as well.
                for _ in 0..8192 {
                    let observation = match self.staged.pop_front().or_else(|| self.observations.as_ref()?.try_recv().ok()) {
                        Some(observation) => observation, None => break,
                    };
                    if observation.session != fence.session { continue; }
                    if observation.first_frame.checked_add(u64::from(observation.frames)).is_some_and(|end| end > fence.frame) {
                        self.staged.push_front(observation); break;
                    }
                    let source = self.source(observation.episode.load);
                    self.remember_unresolved(observation.episode.load, observation.episode.deck, &id, &source);
                    let session = self.store.as_mut().unwrap().sessions.get_mut(&id).unwrap();
                    if let Err(error) = session.observe(observation, source) { session.incomplete = true; self.message = format!("History observation incomplete: {error}"); self.publish = true; }
                    self.dirty.insert(id.clone()); self.publish = true;
                }
                let session = self.store.as_mut().unwrap().sessions.get_mut(&id).unwrap();
                if fence.wall_ns >= session.started_ns && fence.frame >= session.confirmed_frame {
                    let changed = session.last_confirmed_ns < fence.wall_ns || session.confirmed_frame < fence.frame
                        || (!session.incomplete && fence.incomplete) || session.dropped_observation_frames < fence.dropped;
                    session.last_confirmed_ns = session.last_confirmed_ns.max(fence.wall_ns);
                    session.confirmed_frame = fence.frame;
                    session.incomplete |= fence.incomplete;
                    session.dropped_observation_frames = session.dropped_observation_frames.max(fence.dropped);
                    if changed { self.dirty.insert(id.clone()); self.publish = true; }
                }
            }
        }
        if ending.is_none() && self.renderer.as_ref().is_some_and(|h| !h.status().3) {
            if let Some(id) = self.active.take() {
                if let Some(session) = self.store.as_mut().and_then(|store| store.sessions.get_mut(&id)) {
                    session.recover_unclean(); self.dirty.insert(id);
                    self.message = "Audio output disconnected; history ends at its last confirmed prefix".into(); self.publish = true;
                }
            }
        }
        if let Some((pending, ack)) = ending {
            if let Boundary::End { id, closing } = pending.boundary {
                let result = self.store.as_mut().and_then(|s| s.sessions.get_mut(&id)).ok_or("history session unavailable".into())
                    .and_then(|session| session.end(ack.wall_ns, ack.frame, ack.incomplete, ack.dropped));
                if result.is_ok() { self.active = None; self.dirty.insert(id); self.closing |= closing; self.save_due(true); }
                self.finish(pending.request, result.map(|_| if self.dirty.is_empty() {
                    "Performance session ended and saved".into()
                } else { format!("Performance session ended; save remains pending: {}", self.message) }));
            }
        }
    }
    fn commit_result(&mut self, id: &str, commit: Commit) {
        match commit {
            Commit::Durable => { self.dirty.remove(id); self.durable_at_ns = Some(unix_ns()); self.message = "History saved".into(); },
            Commit::CommittedUnconfirmed(warning) => { self.dirty.insert(id.to_owned()); self.message = format!("History committed, durability unconfirmed: {warning}"); },
        }
        self.publish = true;
    }
    fn save_due(&mut self, force: bool) {
        if !force && self.save_at.elapsed() < Duration::from_secs(1) { return; }
        self.save_at = Instant::now();
        let ids: Vec<_> = self.dirty.iter().cloned().collect();
        for id in ids {
            let Some(store) = &mut self.store else { break; };
            let Some(session) = store.sessions.get(&id).cloned() else { continue; };
            match store.save(&session) {
                Ok(commit) => self.commit_result(&id, commit),
                Err(error) => { self.message = format!("History save failed; buffered data remains pending: {error}"); self.publish = true; },
            }
        }
    }
    fn view(&self) -> View {
        let mut sessions: Vec<_> = self.store.as_ref().into_iter().flat_map(|s| s.sessions.values()).map(|s| Summary {
            id: s.id.clone(), started_ns: s.started_ns, state: s.state, entries: s.entries.len() }).collect();
        sessions.sort_by_key(|s| std::cmp::Reverse(s.started_ns));
        View { sessions, selected: self.selected.as_ref().and_then(|id| self.store.as_ref()?.sessions.get(id).cloned()),
            active: self.active.clone(), ready: self.store.is_some(), durable: self.dirty.is_empty() && self.store.is_some(),
            durable_at_ns: self.durable_at_ns, message: self.message.clone(), receipt: self.receipt.clone() }
    }
}
fn unix_ns() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()
    .and_then(|d| u64::try_from(d.as_nanos()).ok()).unwrap_or(0) }

#[cfg(test)]
mod tests;
