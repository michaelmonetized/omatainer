//! A single bounded owner for reusable reference definitions. Neither decoding
//! nor project state belongs to this store. UI publication returns old metadata
//! to the worker for retirement; a committed save is never reported cancelled.
use super::{BankId, Collection, Commit, Store};
use crate::engine::{performance, sampler};
use std::{path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, mpsc, Arc}};

pub(crate) struct Saved {
    pub definition: BankId,
    pub commit: Commit,
}
#[derive(Debug)]
enum Action {
    Open,
    Save { bank: sampler::Bank, name: String, replace: Option<BankId>, revision: u64 },
}
#[derive(Debug)]
struct Job { action: Action, cancel: Arc<AtomicBool> }
struct Update {
    collection: Option<Arc<Collection>>,
    result: Result<Option<Saved>, String>,
    retired: mpsc::SyncSender<Option<Arc<Collection>>>,
}
pub(crate) struct Manager {
    pub collection: Option<Arc<Collection>>,
    pub error: Option<String>,
    pub saved: Option<Saved>,
    pub busy: bool,
    jobs: mpsc::SyncSender<Job>,
    results: mpsc::Receiver<Update>,
    cancel: Arc<AtomicBool>,
    performance: performance::Handle,
}
impl Manager {
    pub fn start(path: PathBuf, performance: performance::Handle) -> std::io::Result<Self> {
        let (jobs, incoming) = mpsc::sync_channel::<Job>(1);
        let (results, updates) = mpsc::sync_channel::<Update>(1);
        let policy = performance.clone();
        std::thread::Builder::new().name("omat-sampler-store".into()).spawn(move || {
            let mut store: Option<Store> = None;
            while let Ok(job) = incoming.recv() {
                let result = (|| {
                    if job.cancel.load(Ordering::Acquire) { return Err("sampler store operation cancelled".into()); }
                    let work = policy.optional_work().map_err(|e| e.to_string())?;
                    match job.action {
                        Action::Open => {
                            if store.is_none() { store = Some(Store::open(path.clone())?); }
                            if job.cancel.load(Ordering::Acquire) || work.cancelled() {
                                return Err("sampler store read cancelled; last visible definitions retained".into());
                            }
                            Ok(None)
                        }
                        Action::Save { bank, name, replace, revision } => {
                            let store = store.as_mut().ok_or("reusable sampler store is unavailable; retry opening it first")?;
                            if store.collection.revision != revision {
                                return Err("reusable definitions changed; review the current target before saving".into());
                            }
                            let id = match replace { Some(id) => id, None => BankId::new()? };
                            let mut definition = bank.data.definition(id)?;
                            definition.name = name;
                            definition.validate()?;
                            let mut collection = store.collection.clone();
                            collection.put(definition, replace)?;
                            if job.cancel.load(Ordering::Acquire) { return Err("sampler bank save cancelled before commit".into()); }
                            // Mode entry refuses Changing once this short
                            // irreversible commit begins. Save's own token
                            // distinguishes precommit cancel from late cancel.
                            let _commit = work.commit().map_err(|e| e.to_string())?;
                            let commit = store.save(collection, &job.cancel)?;
                            Ok(Some(Saved { definition: id, commit }))
                        }
                    }
                })();
                let collection = if result.is_ok() { store.as_ref().map(|store| Arc::new(store.collection.clone())) } else { None };
                let (retired, ack) = mpsc::sync_channel(1);
                if results.send(Update { collection, result, retired }).is_err() { return; }
                // At most one publication waits for UI acknowledgment. Closing
                // the UI drops the reply sender; there is no UI join or busy loop.
                match ack.recv() { Ok(old) => drop(old), Err(_) => return }
            }
        })?;
        let cancel = Arc::new(AtomicBool::new(false));
        jobs.try_send(Job { action: Action::Open, cancel: cancel.clone() }).expect("fresh sampler store queue");
        Ok(Self { collection: None, error: None, saved: None, busy: true,
            jobs, results: updates, cancel, performance })
    }
    fn request(&mut self, action: Action) -> Result<(), String> {
        if self.busy { return Err("a sampler store operation is still pending".into()); }
        self.performance.optional_work().map_err(|e| e.to_string())?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.try_send(Job { action, cancel: cancel.clone() }).map_err(|_| "sampler store worker is unavailable or busy".to_string())?;
        self.cancel = cancel;
        self.busy = true;
        self.error = None;
        self.saved = None;
        Ok(())
    }
    pub fn retry(&mut self) -> Result<(), String> { self.request(Action::Open) }
    pub fn save(&mut self, bank: sampler::Bank, name: String, replace: Option<BankId>) -> Result<(), String> {
        let revision = self.collection.as_ref().ok_or("reusable sampler store is unavailable")?.revision;
        self.request(Action::Save { bank, name, replace, revision })
    }
    pub fn cancel(&self) { self.cancel.store(true, Ordering::Release); }
    pub fn poll(&mut self) -> bool {
        let update = match self.results.try_recv() {
            Ok(update) => update,
            Err(mpsc::TryRecvError::Empty) => return false,
            Err(mpsc::TryRecvError::Disconnected) => {
                if self.busy { self.busy = false; self.error = Some("sampler store worker disconnected".into()); return true; }
                return false;
            }
        };
        self.busy = false;
        let old = if let Some(collection) = update.collection {
            self.collection.replace(collection)
        } else { None };
        match update.result {
            Ok(saved) => { self.saved = saved; self.error = None; }
            Err(error) => { self.error = Some(error); self.saved = None; }
        }
        // The one-element reply is empty until this exact publication is read.
        let _ = update.retired.try_send(old);
        true
    }
}
impl Drop for Manager { fn drop(&mut self) { self.cancel(); } }
