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
}

impl Patch {
    fn apply(&self, item: &mut LibItem) {
        if item.source == self.source && item.fingerprint == Some(self.fingerprint) {
            item.bpm = item.bpm.reconcile(self.bpm);
        }
    }
}

struct Job {
    base: Arc<Vec<LibItem>>,
    candidate: Arc<Vec<LibItem>>,
    _retired_candidates: Vec<Arc<Vec<LibItem>>>,
    revision: u64,
    updates: Vec<Patch>,
}
struct Result {
    base: Weak<Vec<LibItem>>,
    revision: u64,
    items: Arc<Vec<LibItem>>,
    retire: mpsc::SyncSender<(Arc<Vec<LibItem>>, Option<Arc<Vec<LibItem>>>)>,
}

pub(super) struct Metadata {
    jobs: mpsc::SyncSender<Job>,
    results: mpsc::Receiver<Result>,
    pending: Vec<Patch>,
    staged: Option<Arc<Vec<LibItem>>>,
    retired_candidates: Vec<Arc<Vec<LibItem>>>,
    revision: u64,
    dirty: bool,
    in_flight: bool,
}

impl Default for Metadata {
    fn default() -> Self {
        let (jobs, work) = mpsc::sync_channel::<Job>(1);
        let (done, results) = mpsc::sync_channel(1);
        // A spawn failure is visible at the next submission. No synchronous
        // clone/sort fallback is allowed on the GUI thread.
        let _ = std::thread::Builder::new()
            .name("omatainer-metadata".into())
            .spawn(move || {
                let mut cache = HashMap::<LibSource, Patch>::new();
                while let Ok(job) = work.recv() {
                    for patch in job.updates {
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
                    sort_crate(&mut items);
                    let items = Arc::new(items);
                    let (retire, retired) = mpsc::sync_channel(1);
                    if done
                        .send(Result {
                            base: Arc::downgrade(&job.base),
                            revision: job.revision,
                            items: items.clone(),
                            retire,
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
            staged: None,
            retired_candidates: Vec::new(),
            revision: 0,
            dirty: false,
            in_flight: false,
        }
    }
}

impl Metadata {
    pub fn update(&mut self, patch: Patch) {
        if let Some(old) = self
            .pending
            .iter_mut()
            .find(|old| old.source == patch.source)
        {
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
            if result.revision == self.revision && result.base.as_ptr() == Arc::as_ptr(library) {
                let previous = std::mem::replace(library, result.items);
                let _ = result.retire.try_send((previous, self.staged.take()));
                published = true;
                self.dirty = false;
            } else {
                let _ = result.retire.try_send((result.items, None));
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
            };
            match self.jobs.try_send(job) {
                Ok(()) => {
                    self.dirty = false;
                    self.in_flight = true;
                }
                Err(mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job)) => {
                    self.pending = job.updates;
                    self.retired_candidates = job._retired_candidates;
                    return Err(
                        "crate metadata worker unavailable; BPM metadata was not refreshed",
                    );
                }
            }
        }
        Ok(published)
    }
    #[cfg(test)]
    pub(super) fn active(&self) -> bool {
        self.dirty || self.in_flight
    }
}

impl App {
    pub(super) fn poll_library_metadata(&mut self) {
        self.refresh_library_view(); // capture selection before an Arc swap
        match self.library_metadata.poll(&mut self.library) {
            Ok(true) => self.refresh_library_view(),
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
