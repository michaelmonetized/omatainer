//! A single worker clones/sorts immutable crate versions and retires old vectors.
//! GUI polling only swaps a candidate built from the current base and revision.
use super::*;
use crate::engine::media_source::FileFingerprint;
use std::sync::{mpsc, Weak};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(super) struct Patch {
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
    pub bpm: Bpm,
    pub duration: Option<f64>,
}

impl Patch {
    fn preserve_duration(&mut self, previous: &Self) {
        if self.duration.is_none() && self.fingerprint == previous.fingerprint {
            self.duration = previous.duration;
        }
    }

    fn apply(&self, item: &mut LibItem) {
        if item.source == self.source && item.fingerprint == Some(self.fingerprint) {
            item.bpm = item.bpm.reconcile(self.bpm);
            if let Some(duration) = self
                .duration
                .filter(|value| value.is_finite() && *value >= 0.0)
            {
                item.length = Some(duration);
            }
        }
    }
}

struct Job {
    base: Arc<Vec<LibItem>>,
    candidate: Arc<Vec<LibItem>>,
    _retired_candidates: Vec<Arc<Vec<LibItem>>>,
    revision: u64,
    updates: Vec<Patch>,
    captures: Vec<super::library_store::Capture>,
    import: Option<PathBuf>,
}
struct Result {
    base: Weak<Vec<LibItem>>,
    revision: u64,
    items: Arc<Vec<LibItem>>,
    retire: mpsc::SyncSender<(
        Arc<Vec<LibItem>>,
        Option<Arc<Vec<LibItem>>>,
        Arc<crate::library::Catalog>,
    )>,
    catalog: Arc<crate::library::Catalog>,
    storage: Option<String>,
    durable: bool,
}

pub(super) struct Metadata {
    jobs: mpsc::SyncSender<Job>,
    results: mpsc::Receiver<Result>,
    pending: Vec<Patch>,
    captures: Vec<super::library_store::Capture>,
    import: Option<PathBuf>,
    pub catalog: Arc<crate::library::Catalog>,
    pub storage: Option<String>,
    pub storage_error: Option<String>,
    pub durable: bool,
    clear_error: bool,
    staged: Option<Arc<Vec<LibItem>>>,
    retired_candidates: Vec<Arc<Vec<LibItem>>>,
    revision: u64,
    dirty: bool,
    in_flight: bool,
}

impl Default for Metadata {
    fn default() -> Self {
        Self::new(None)
    }
}
impl Metadata {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self::spawn(path, || {})
    }
    #[cfg(test)]
    pub fn with_hook(path: PathBuf, hook: impl FnMut() + Send + 'static) -> Self {
        Self::spawn(Some(path), hook)
    }
    fn spawn(path: Option<PathBuf>, mut before_job: impl FnMut() + Send + 'static) -> Self {
        let persistent = path.is_some();
        let (jobs, work) = mpsc::sync_channel::<Job>(1);
        let (done, results) = mpsc::sync_channel(1);
        // A spawn failure is visible at the next submission. No synchronous
        // clone/sort fallback is allowed on the GUI thread.
        let _ = std::thread::Builder::new()
            .name("omatainer-metadata".into())
            .spawn(move || {
                let mut store = path.map(crate::library::Store::open);
                let mut cache = HashMap::<LibSource, Patch>::new();
                while let Ok(job) = work.recv() {
                    before_job();
                    for mut patch in job.updates {
                        // A BPM-only update cannot erase a known duration for
                        // these same bytes; a replacement identity starts fresh.
                        if let Some(previous) = cache.get(&patch.source) {
                            patch.preserve_duration(previous);
                        }
                        cache.insert(patch.source.clone(), patch);
                    }
                    let corrections: HashMap<_, _> = job
                        .base
                        .iter()
                        .filter(|item| item.bpm.origin == Origin::User)
                        .map(|item| (&item.source, (item.fingerprint, item.bpm)))
                        .collect();
                    let mut items = job.candidate.as_ref().clone();
                    for item in &mut items {
                        if let Some((fingerprint, bpm)) = corrections.get(&item.source) {
                            if item.fingerprint == *fingerprint {
                                item.bpm = *bpm;
                            }
                        }
                        if let Some(patch) = cache.get(&item.source) {
                            patch.apply(item);
                        }
                    }
                    let mut storage = None;
                    let mut durable = false;
                    let catalog = if let Some(store) = &mut store {
                        match store {
                            Ok(store) => {
                                storage = Some(
                                    match super::library_store::reconcile(
                                        store,
                                        &mut items,
                                        job.captures,
                                        job.import,
                                    ) {
                                        Ok(import_error) => {
                                            durable = true;
                                            import_error.map_or_else(|| "DJ library saved".into(), |error|
                                                format!("DJ library saved; import rejected: {error}"))
                                        },
                                        Err(error) => format!("DJ library NOT saved: {error}"),
                                    },
                                );
                                Arc::new(store.catalog.clone())
                            }
                            Err(error) => {
                                storage = Some(format!(
                                    "DJ library unavailable; original store preserved (repair it and restart): {error}"
                                ));
                                Arc::new(crate::library::Catalog::default())
                            }
                        }
                    } else {
                        Arc::new(crate::library::Catalog::default())
                    };
                    sort_crate(&mut items);
                    let items = Arc::new(items);
                    let (retire, retired) = mpsc::sync_channel(1);
                    if done
                        .send(Result {
                            base: Arc::downgrade(&job.base),
                            revision: job.revision,
                            items: items.clone(),
                            retire,
                            catalog: catalog.clone(),
                            storage,
                            durable,
                        })
                        .is_err()
                    {
                        break;
                    }
                    // Hold both candidate and base until GUI publication/discard.
                    // Their final large deallocation therefore stays here.
                    drop(retired.recv());
                    drop(items);
                    drop(job.base);
                    drop(job.candidate);
                }
            });
        Self {
            jobs,
            results,
            pending: Vec::new(),
            captures: Vec::new(),
            import: None,
            catalog: Arc::new(crate::library::Catalog::default()),
            storage: persistent.then(|| "Opening DJ library…".into()),
            storage_error: None,
            durable: !persistent,
            clear_error: false,
            staged: None,
            retired_candidates: Vec::new(),
            revision: 0,
            dirty: persistent,
            in_flight: false,
        }
    }
}

impl Metadata {
    pub fn capture(&mut self, capture: super::library_store::Capture) {
        if self.storage.is_none() {
            return;
        }
        if let Some(old) = self
            .captures
            .iter_mut()
            .find(|old| old.source == capture.source && old.fingerprint == capture.fingerprint)
        {
            old.preparation = capture.preparation.or(old.preparation);
            old.played = old.played.max(capture.played);
        } else {
            self.captures.push(capture);
        }
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
    }
    pub fn import(&mut self, path: PathBuf) -> bool {
        if self.storage.is_none() || self.import.is_some() {
            return false;
        }
        self.import = Some(path);
        self.clear_error = true;
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        true
    }
    pub fn retry_save(&mut self) {
        self.dirty = true;
        self.clear_error = true;
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn label(&self) -> &str {
        if let Some(error) = &self.storage_error {
            return error;
        }
        if self.dirty || self.in_flight {
            if self
                .storage
                .as_ref()
                .is_some_and(|s| s == "DJ library saved")
            {
                return "Saving DJ library…";
            }
        }
        self.storage.as_deref().unwrap_or("Session crate")
    }
    pub fn update(&mut self, mut patch: Patch) {
        if let Some(old) = self
            .pending
            .iter_mut()
            .find(|old| old.source == patch.source)
        {
            patch.preserve_duration(old);
            *old = patch;
        } else {
            self.pending.push(patch);
        }
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
    }
    /// A scan candidate becomes visible only after cached metadata is merged.
    /// Acknowledge its handoff to the scanner, while retaining candidate ownership
    /// until our worker can retire it after a valid publication or replacement.
    pub fn stage_scan(
        &mut self,
        publication: library_scan::Publication,
        library: &Arc<Vec<LibItem>>,
    ) {
        let mut candidate = library.clone();
        publication.publish(&mut candidate);
        if let Some(previous) = self.staged.replace(candidate) {
            self.retired_candidates.push(previous);
        }
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
    }
    /// A preference change invalidates pending scans from the old root set.
    /// Existing visible rows remain until the new scan succeeds; retirement and
    /// metadata overlays still belong to this worker.
    pub fn cancel_scan(&mut self) {
        if let Some(candidate) = self.staged.take() { self.retired_candidates.push(candidate); }
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
    }
    #[cfg(test)]
    pub fn rebase(&mut self) {
        self.dirty = true;
    }

    /// Returns true only when a new sorted immutable crate was published.
    pub fn poll(
        &mut self,
        library: &mut Arc<Vec<LibItem>>,
    ) -> std::result::Result<bool, &'static str> {
        let mut published = false;
        if let Ok(result) = self.results.try_recv() {
            self.in_flight = false;
            if result
                .storage
                .as_ref()
                .is_some_and(|s| s != "DJ library saved")
            {
                self.storage_error = result.storage.clone();
            } else if self.clear_error && result.revision == self.revision {
                self.storage_error = None;
            }
            if result.revision == self.revision {
                self.clear_error = false;
            }
            self.storage = result.storage;
            self.durable = result.durable;
            let retired_catalog = std::mem::replace(&mut self.catalog, result.catalog);
            if result.revision == self.revision && result.base.as_ptr() == Arc::as_ptr(library) {
                let previous = std::mem::replace(library, result.items);
                let _ = result
                    .retire
                    .try_send((previous, self.staged.take(), retired_catalog));
                published = true;
                self.dirty = false;
            } else {
                let _ = result
                    .retire
                    .try_send((result.items, None, retired_catalog));
                self.dirty = true;
            }
        }
        if self.dirty && !self.in_flight {
            let job = Job {
                base: library.clone(),
                candidate: self.staged.as_ref().unwrap_or(library).clone(),
                _retired_candidates: std::mem::take(&mut self.retired_candidates),
                revision: self.revision,
                updates: std::mem::take(&mut self.pending),
                captures: std::mem::take(&mut self.captures),
                import: self.import.take(),
            };
            match self.jobs.try_send(job) {
                Ok(()) => {
                    self.dirty = false;
                    self.in_flight = true;
                }
                Err(mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job)) => {
                    self.pending = job.updates;
                    self.captures = job.captures;
                    self.import = job.import;
                    self.retired_candidates = job._retired_candidates;
                    if self.storage.is_some() {
                        self.durable = false;
                        self.storage_error =
                            Some("DJ library worker unavailable; changes are NOT saved".into());
                    }
                    return Err(
                        "crate metadata worker unavailable; library metadata was not refreshed",
                    );
                }
            }
        }
        Ok(published)
    }
    pub(super) fn active(&self) -> bool {
        self.dirty || self.in_flight
    }
}

impl App {
    pub(super) fn poll_library_metadata(&mut self) {
        self.refresh_library_view(); // capture selection before an Arc swap
        match self.library_metadata.poll(&mut self.library) {
            Ok(true) => {
                self.refresh_library_view();
                self.restore_initial_library_preparation();
            }
            Err(error) => self.status = error.into(),
            _ => {}
        }
    }

    /// Analysis is committed once, only after this request has actually become
    /// current. Browsing, stale decoder replies and rejected commands cannot
    /// retarget it. File identity is checked again on the worker against each row.
    pub(super) fn reconcile_loaded_metadata(&mut self) {
        for load in self.loads.iter_mut().flatten() {
            if load
                .receipt
                .as_ref()
                .is_some_and(|r| r.state() == crate::engine::load_receipt::State::Current)
                && load.token.as_ref().is_some_and(|t| t.is_current())
            {
                if let Some(patch) = load.metadata.take() {
                    self.library_metadata.update(patch);
                }
            }
        }
        self.poll_library_metadata();
    }
}
