//! A single worker clones/sorts immutable crate versions and retires old vectors.
//! GUI polling only swaps a candidate built from the current base and revision.
use super::*;
use crate::engine::media_source::FileFingerprint;
use crate::engine::performance::{Handle, WorkPermit};
use std::sync::{mpsc, Weak};

const SAMPLER_PROOF_LIMIT: usize = 256;
mod analysis;
pub(super) mod tags;
mod collections;
mod collection_rows;
pub(super) use collection_rows::CollectionRows;
pub(super) use collections::{Action as CollectionAction, Admission as CollectionAdmission, Token as CollectionToken, Receipt as CollectionReceipt, Outcome as CollectionOutcome};
pub(super) use collections::files;
pub(super) use analysis::{Receipt as AnalysisReceipt, Inspect as AnalysisInspect, Inspected as AnalysisInspected, Cached as AnalysisCached};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub(super) struct Patch {
    pub tags: Option<std::result::Result<crate::media_tags::Observation, String>>,
    pub source: LibSource,
    pub fingerprint: FileFingerprint,
    pub bpm: Bpm,
    pub duration: Option<f64>,
}

impl Patch {
    fn filename_item(&self) -> Option<LibItem> {
        let path = match &self.source {
            LibSource::File(path) => path,
            LibSource::Removable { relative_path, .. } => relative_path,
            _ => return None,
        };
        let stem = path.file_stem().and_then(|stem| stem.to_str()).unwrap_or("track");
        let (artist, title) = split_artist_title(stem);
        let (bpm, key) = parse_tags(stem);
        Some(LibItem { title, artist, bpm: Bpm::hint(bpm), key,
            source: self.source.clone(), fingerprint: Some(self.fingerprint),
            length: self.duration, last_play: None })
    }
    fn preserve_duration(&mut self, previous: &Self) {
        if self.tags.is_none() && self.fingerprint == previous.fingerprint { self.tags = previous.tags.clone(); }
        if self.duration.is_none() && self.fingerprint == previous.fingerprint {
            self.duration = previous.duration;
        }
    }

    /// Update an exact row without replacing protected preparation.
    /// Takes the row and saved track locks; applies permitted matching-version fields only.
    fn apply(&self, item: &mut LibItem, locks:crate::library::protection::Locks) {
        if item.source == self.source && item.fingerprint == Some(self.fingerprint) {
            if !locks.bpm {item.bpm = item.bpm.reconcile(self.bpm);}
            if let Some(Ok(observation)) = self.tags.as_ref().filter(|_|!locks.metadata) {
                if let Some(field) = &observation.fields.title { item.title.clone_from(&field.value); }
                if let Some(field) = &observation.fields.artist { item.artist.clone_from(&field.value); }
                if let Some(field) = &observation.fields.key { item.key.clone_from(&field.value); }
            }
            if let Some(duration) = self
                .duration
                .filter(|value| !locks.metadata && value.is_finite() && *value >= 0.0)
            {
                item.length = Some(duration);
            }
        }
    }
}

struct Import {
    path: PathBuf,
    work: Arc<WorkPermit>,
}
struct Relocation {
    reviewed: Option<crate::library::relocation_search::Candidate>,
    request: crate::library::Relocate,
    work: Arc<WorkPermit>,
}
struct Staged {
    roots: Option<Arc<library_scan::ScanRoots>>,
    items: Arc<Vec<LibItem>>,
    work: Arc<WorkPermit>,
}
struct Job {
    tags: Option<tags::Save>,
    collection: Option<collections::Request>,
    base: Arc<Vec<LibItem>>,
    candidate: Arc<Vec<LibItem>>,
    scan_work: Option<Arc<WorkPermit>>,
    roots: Option<Arc<library_scan::ScanRoots>>,
    restricted: bool,
    _retired_candidates: Vec<Arc<Vec<LibItem>>>,
    _retired_roots:Vec<Arc<library_scan::ScanRoots>>,
    _retired_file_reviews: Vec<Arc<crate::library::Catalog>>,
    revision: u64,
    updates: Vec<Patch>,
    captures: Vec<super::library_store::Capture>,
    sampler_sources: Vec<crate::sampler_bank::SourceRef>,
    import: Option<Import>,
    relocation: Option<Relocation>,
    qualification_work: Option<WorkPermit>,
    analysis: Option<crate::engine::media_load::AnalysisCompletion>,
    inspection: Option<AnalysisInspect>,
    retired_analysis_indices: Vec<Arc<Vec<usize>>>,
    retired_analysis_inspections: Vec<AnalysisInspected>,
}
struct Result {
    tags: Option<tags::Receipt>,
    collection: Option<CollectionReceipt>,
    analysis: Option<AnalysisReceipt>,
    inspection: Option<AnalysisInspected>,
    relocation: Option<RelocationResult>,
    qualification_pending: bool,
    base: Weak<Vec<LibItem>>,
    revision: u64,
    items: Arc<Vec<LibItem>>,
    restricted: Arc<Vec<LibItem>>,
    row_indices: [Arc<CollectionRows>; 3],
    retire: mpsc::SyncSender<(
        Arc<Vec<LibItem>>,
        Option<Arc<Vec<LibItem>>>,
        Arc<crate::library::Catalog>,
        Arc<Vec<LibItem>>,
        [Arc<CollectionRows>; 3],
    )>,
    catalog: Arc<crate::library::Catalog>,
    storage: Option<String>,
    durable: bool,
}
struct RelocationResult {
    request: crate::library::Relocate,
    outcome: std::result::Result<(), String>,
}

pub(super) struct Metadata {
    pub tag_recovery_root: Option<PathBuf>,
    tag_reservation: bool,
    tag_save: Option<tags::Save>,
    tag_save_id: Option<u64>,
    tag_result: Option<tags::Receipt>,
    initialized:bool,
    collection: Option<collections::Request>,
    collection_result: Option<CollectionReceipt>,
    collection_active: Option<CollectionToken>,
    next_collection: u64,
    collections_closing: bool,
    analysis: Option<crate::engine::media_load::AnalysisCompletion>,
    inspection: Option<AnalysisInspect>,
    retired_analysis_indices: Vec<Arc<Vec<usize>>>,
    retired_analysis_inspections: Vec<AnalysisInspected>,
    analysis_result: Option<AnalysisReceipt>,
    analysis_active: Option<u64>,
    inspection_result: Option<AnalysisInspected>,
    inspection_active: Option<(u64, crate::sampler_bank::SourceRef, Arc<WorkPermit>, Arc<std::sync::atomic::AtomicBool>)>,
    worker_closed: bool,
    relocation_result: Option<RelocationResult>,
    qualification_pending: bool,
    performance: Handle,
    deferred: bool,
    jobs: mpsc::SyncSender<Job>,
    results: mpsc::Receiver<Result>,
    pending: Vec<Patch>,
    captures: Vec<super::library_store::Capture>,
    sampler_sources: Vec<crate::sampler_bank::SourceRef>,
    import: Option<Import>,
    relocation: Option<Relocation>,
    pub catalog: Arc<crate::library::Catalog>,
    collection_rows: Arc<CollectionRows>,
    pub storage: Option<String>,
    pub storage_error: Option<String>,
    pub durable: bool,
    clear_error: bool,
    staged: Option<Staged>,
    retired_candidates: Vec<Arc<Vec<LibItem>>>,
    retired_roots:Vec<Arc<library_scan::ScanRoots>>,
    retired_file_reviews: Vec<Arc<crate::library::Catalog>>,
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
        let tag_recovery_root = path.as_ref().map(|path| path.with_extension("tag-recovery"));
        let (jobs, work) = mpsc::sync_channel::<Job>(1);
        let (done, results) = mpsc::sync_channel(1);
        // A spawn failure is visible at the next submission. No synchronous
        // clone/sort fallback is allowed on the GUI thread.
        let _ = std::thread::Builder::new()
            .name("omatainer-metadata".into())
            .spawn(move || {
                let mut analysis_disk = path.as_deref().map(analysis::Disk::new);
                let mut store = path.map(crate::library::Store::open);
                let mut cache = HashMap::<LibSource, Patch>::new();
                // Latest captured file identity per source only. Protection can
                // defer this optional read while the essential cue save proceeds.
                let mut qualifications = HashMap::<LibSource, Option<FileFingerprint>>::new();
                // Retain only accepted overlays for identities still visible.
                // A stale optional result must not hide an essential capture on
                // the next protected rebase. This cache is bounded by base rows.
                let mut essential = HashMap::<LibSource, super::library_store::Capture>::new();
                // Keep the latest immutable maps pinned until replaced or the
                // worker exits. GUI teardown cannot free their large tables.
                let mut row_index_pins: Option<[Arc<CollectionRows>; 3]> = None;
                while let Ok(mut job) = work.recv() {
                    before_job();
                    let refresh_smart = job.collection.as_ref().is_some_and(|request|matches!(request.action, collections::Action::Read));
                    for capture in &job.captures {
                        // Excess tracks retain their essential cues and can be
                        // verified by an explicit relocation while the original
                        // is available. Never grow optional backlog without bound.
                        if (qualifications.len() < 256 || qualifications.contains_key(&capture.source))
                            && matches!(capture.source, LibSource::File(_) | LibSource::Removable {..})
                            && capture.preparation.is_some_and(|p| p.grid.is_some() || p.hotcues.iter().any(Option::is_some) || p.saved_loops.slots.iter().any(Option::is_some)) {
                            qualifications.insert(capture.source.clone(), capture.fingerprint);
                        }
                    }
                    let qualification_sources: Vec<_> = qualifications.iter().map(|(source, fingerprint)| (source.clone(), *fingerprint)).collect();
                    let visible_identity: HashMap<_, _> = job.base.iter()
                        .map(|item| (&item.source, item.fingerprint)).collect();
                    essential.retain(|source, capture| visible_identity.get(source)
                        .is_some_and(|fingerprint| *fingerprint == capture.fingerprint));
                    for capture in &job.captures {
                        if visible_identity.get(&capture.source) != Some(&capture.fingerprint) { continue; }
                        let mut next = capture.clone();
                        if let Some(previous) = essential.get(&capture.source) {
                            next.metadata.bpm = previous.metadata.bpm.reconcile(next.metadata.bpm);
                            next.metadata.duration = next.metadata.duration.or(previous.metadata.duration);
                            next.metadata.last_play = next.metadata.last_play.max(previous.metadata.last_play);
                            next.played = next.played.max(previous.played);
                            for (value, old) in [(&mut next.metadata.title, &previous.metadata.title),
                                (&mut next.metadata.artist, &previous.metadata.artist),
                                (&mut next.metadata.key, &previous.metadata.key)] {
                                if !old.is_empty() { *value = old.clone(); }
                            }
                        }
                        essential.insert(capture.source.clone(), next);
                    }
                    let loaded: Vec<_> = job.updates.iter().filter(|patch| patch.tags.is_some())
                        .filter_map(Patch::filename_item).collect();
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
                    let mut fallback = job.base.as_ref().clone();
                    for item in &mut fallback {
                        if let Some(patch) = cache.get(&item.source) { let locks=store.as_ref().and_then(|store|store.as_ref().ok()).and_then(|store|store.catalog.track_for_version(&item.source,item.fingerprint)).map_or(Default::default(),|track|track.locks); patch.apply(item,locks); }
                    }
                    let mut items = if job.scan_work.as_ref().is_some_and(|work| work.cancel().load(std::sync::atomic::Ordering::Acquire)) {
                        fallback.clone()
                    } else { job.candidate.as_ref().clone() };
                    for item in loaded {
                        if !items.iter().any(|existing| existing.source == item.source) { items.push(item); }
                    }
                    for item in &mut items {
                        if let Some((fingerprint, bpm)) = corrections.get(&item.source) {
                            if item.fingerprint == *fingerprint {
                                item.bpm = *bpm;
                            }
                        }
                        if let Some(patch) = cache.get(&item.source) {
                            let locks=store.as_ref().and_then(|store|store.as_ref().ok()).and_then(|store|store.catalog.track_for_version(&item.source,item.fingerprint)).map_or(Default::default(),|track|track.locks); patch.apply(item,locks);
                        }
                    }
                    let mut storage = None;
                    let mut durable = false;
                    let mut qualification_pending = false;
                    let mut relocation_outcome = None;
                    let mut analysis_result = None;
                    let mut inspection_result = None;
                    let mut collection_result = None;
                    let mut tag_result = None;
                    let mut analysis_committed = false;
                    let catalog = if let Some(store) = &mut store {
                        match store {
                            Ok(store) => {
                                storage = Some(
                                    match super::library_store::reconcile_optional(
                                        store, &mut items, &job.captures, &job.sampler_sources,
                                        job.import.as_ref().map(|import| import.path.as_path()),
                                        job.import.as_ref().map(|import| import.work.as_ref()),
                                        job.scan_work.as_deref(), job.roots.as_deref(), &fallback,
                                        job.relocation.as_ref().map(|r| (&r.request, r.work.as_ref(), r.reviewed.as_ref())), job.qualification_work.as_ref(), &qualification_sources,
                                    ) {
                                        Ok(result) => {
                                            qualification_pending = result.qualification_pending;
                                            relocation_outcome = result.relocation;
                                            durable = true;
                                            result.notice.map_or_else(|| "DJ library saved".into(), |error|
                                                format!("DJ library saved; {error}"))
                                        },
                                        Err(error) if store.last_save_replaced() => format!("DJ library replacement committed; durability unconfirmed: {error}"),
                                        Err(error) => format!("DJ library NOT saved: {error}"),
                                    },
                                );
                                if durable {
                                    if let Err(error) = tags::observe_loaded(store, cache.values()) {
                                        durable = false;
                                        storage = Some(format!("Loaded tags require a library save retry: {error}"));
                                    }
                                    items = store.catalog.tracks.iter().map(|track|
                                        LibItem::from_stored(track.source.clone(), &track.versions[track.current])).collect();
                                }
                                if let Some(request) = job.tags.take() {
                                    let media_installed = matches!(request.result.as_ref(), library_scan::tag_jobs::Reply::Applied(_) | library_scan::tag_jobs::Reply::Recover(_));
                                    let receipt = if durable { tags::save(store, request) } else {
                                        tags::Receipt { id: request.id, committed: false, durable: false, cleanup: None,
                                            outcome: Err("Tag catalog save waits for confirmed essential library persistence; any installed media remains journaled".into()) }
                                    };
                                    if !receipt.durable && (receipt.committed || media_installed) {
                                        durable = false;
                                        storage = Some(format!("Tag catalog save unconfirmed: {}", receipt.outcome.as_ref().err().map(String::as_str).unwrap_or("durability unavailable")));
                                    }
                                    if receipt.committed {
                                        items = store.catalog.tracks.iter().map(|track|
                                            LibItem::from_stored(track.source.clone(), &track.versions[track.current])).collect();
                                    }
                                    tag_result = Some(receipt);
                                }
                                if let Some(completion) = job.analysis.take() {
                                    let receipt = if durable {
                                        analysis::save(store, analysis_disk.as_mut().unwrap(), completion)
                                    } else {
                                        AnalysisReceipt::refused(completion, "Analysis was not saved: essential catalog persistence failed".into())
                                    };
                                    analysis_committed = receipt.committed;
                                    if receipt.committed && receipt.outcome.is_err() {
                                        durable = false;
                                        storage = Some(format!("Analysis replacement committed; durability unconfirmed: {}", receipt.outcome.as_ref().unwrap_err()));
                                    }
                                    if analysis_committed {
                                        items = store.catalog.tracks.iter().map(|track|
                                            LibItem::from_stored(track.source.clone(), &track.versions[track.current])).collect();
                                    }
                                    analysis_result = Some(receipt);
                                }
                                if let Some(request) = job.collection.take() {
                                    let changed_rows=matches!(&request.action,collections::Action::ImportPlaylist{..}|collections::Action::Files(_));
                                    let receipt = if durable { collections::apply(store, request) }
                                        else { CollectionReceipt::refused(request, store.catalog.crates.revision(),
                                            collections::Failure::Storage("Crate edit requires confirmed essential catalog persistence".into())) };
                                    match &receipt.outcome {
                                        CollectionOutcome::CommittedUnconfirmed(error) => {
                                            durable = false;
                                            storage = Some(format!("Crate replacement committed; durability unconfirmed: {error}"));
                                        }
                                        CollectionOutcome::Rejected(error) => {
                                            storage = Some(format!("Crate request was not applied: {error}"));
                                        }
                                        _ => {}
                                    }
                                    collection_result = Some(receipt);
                                    if changed_rows && matches!(&collection_result.as_ref().unwrap().outcome,CollectionOutcome::Durable{changed:true}|CollectionOutcome::CommittedUnconfirmed(_)|CollectionOutcome::Unknown(_)) {
                                        items=store.catalog.tracks.iter().map(|track|LibItem::from_stored(track.source.clone(),&track.versions[track.current])).collect();
                                    }
                                }
                                if let Some(request) = job.inspection.take() {
                                    inspection_result = Some(if durable {
                                        analysis::inspect(store, analysis_disk.as_mut().unwrap(), request)
                                    } else {
                                        AnalysisInspected::refused(request, "Analysis inspection requires a confirmed catalog save".into())
                                    });
                                }
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
                    if let Some(request) = job.tags.take() {
                        tag_result = Some(tags::Receipt { id: request.id, committed: false, durable: false, cleanup: None,
                            outcome: Err("Tag edits require a persistent writable library; media recovery records are retained".into()) });
                    }
                    if let Some(request) = job.collection.take() {
                        collection_result = Some(CollectionReceipt::refused(request, catalog.crates.revision(), collections::Failure::Unavailable));
                    }
                    if let Some(completion) = job.analysis.take() {
                        analysis_result = Some(AnalysisReceipt::refused(completion,
                            "Analysis was not saved: persistent DJ library is unavailable".into()));
                    }
                    if let Some(request) = job.inspection.take() {
                        inspection_result = Some(AnalysisInspected::refused(request,
                            "Analysis inspection requires a persistent DJ library".into()));
                    }
                    if durable && !qualification_pending { qualifications.clear(); }
                    sort_crate(&mut items);
                    // Prepare a second immutable view on the worker. It keeps
                    // prior visible identities and their essential saved metadata,
                    // never optional scan/import changes, even for the same path.
                    let visible: std::collections::HashSet<_> = job.base.iter().map(|item| &item.source).collect();
                    let optional = tag_result.as_ref().is_some_and(|receipt| receipt.committed) || analysis_committed || job.restricted || job.scan_work.is_some() || job.import.is_some() || job.relocation.is_some()
                        || items.iter().any(|item| !visible.contains(&item.source));
                    let restricted = optional.then(|| {
                        let mut rows = if persistent { super::library_store::restricted_rows(&fallback, &essential.values().cloned().collect::<Vec<_>>()) }
                            else { fallback };
                        sort_crate(&mut rows);
                        Arc::new(rows)
                    });
                    let items = Arc::new(items);
                    let restricted = restricted.unwrap_or_else(|| items.clone());
                    let relocation = job.relocation.as_ref().map(|relocation| {
                        let saved = durable && catalog.tracks.iter().any(|track| track.id==relocation.request.id && track.source!=relocation.request.source);
                        RelocationResult {
                            request: relocation.request.clone(),
                            outcome: match relocation_outcome {
                                Some(Ok(())) if saved => Ok(()),
                                Some(Err(error)) => Err(error),
                                _ => Err(storage.clone().unwrap_or_else(|| "Relocation was not saved: library storage is unavailable".into())),
                            },
                        }
                    });
                    let previous = row_index_pins.as_ref().filter(|_|!refresh_smart);
                    let full_index = Arc::new(CollectionRows::build_incremental(&items, &catalog, previous.map(|pins|pins[0].as_ref())));
                    let restricted_index = if Arc::ptr_eq(&items, &restricted) { full_index.clone() }
                        else { Arc::new(CollectionRows::build_incremental(&restricted, &catalog, previous.map(|pins|pins[1].as_ref()))) };
                    let row_indices = [full_index, restricted_index,
                        Arc::new(CollectionRows::build_incremental(&job.base, &catalog, previous.map(|pins|pins[2].as_ref())))];
                    let (retire, retired) = mpsc::sync_channel(1);
                    if done
                        .send(Result {
                            tags: tag_result,
                            collection: collection_result,
                            analysis: analysis_result,
                            inspection: inspection_result,
                            relocation,
                            qualification_pending,
                            base: Arc::downgrade(&job.base),
                            revision: job.revision,
                            items: items.clone(),
                            restricted: restricted.clone(),
                            row_indices: row_indices.clone(),
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
                    // The GUI has returned every replaced/unused map before
                    // we release the previous generation's pins here.
                    drop(retired.recv());
                    row_index_pins = Some(row_indices);
                    drop(items);
                    drop(restricted);
                    drop(job.base);
                    drop(job.candidate);
                }
                drop(row_index_pins);
            });
        Self {
            tag_recovery_root, tag_reservation: false, tag_save: None, tag_save_id: None, tag_result: None,
            collection: None,
            collection_result: None,
            collection_active: None,
            next_collection: 0,
            collections_closing: false,
            analysis: None,
            inspection: None,
            retired_analysis_indices: Vec::new(),
            retired_analysis_inspections: Vec::new(),
            analysis_result: None,
            analysis_active: None,
            inspection_result: None,
            inspection_active: None,
            worker_closed: false,
            relocation_result: None,
            qualification_pending: false,
            performance: Handle::default(),
            deferred: false,
            jobs,
            results,
            pending: Vec::new(),
            captures: Vec::new(),
            sampler_sources: Vec::new(),
            import: None,
            relocation: None,
            catalog: Arc::new(crate::library::Catalog::default()),
            collection_rows: Arc::new(CollectionRows::default()),
            initialized:!persistent,
            storage: persistent.then(|| "Opening DJ library…".into()),
            storage_error: None,
            durable: !persistent,
            clear_error: false,
            staged: None,
            retired_candidates: Vec::new(),
            retired_roots:Vec::new(),
            retired_file_reviews: Vec::new(),
            revision: 0,
            dirty: persistent,
            in_flight: false,
        }
    }
}

impl Metadata {
    /// Return a captured file review to its catalog worker.
    /// Takes one immutable catalog view; queues bounded worker retirement or returns ownership for a later retry.
    pub fn retire_file_review(&mut self, review: Arc<crate::library::Catalog>) -> std::result::Result<(), Arc<crate::library::Catalog>> {
        if self.worker_closed || self.retired_file_reviews.len() >= 2 { return Err(review); }
        self.retired_file_reviews.push(review);
        self.revision=self.revision.wrapping_add(1);self.dirty=true;Ok(())
    }
    /// The table is prepared and pinned by the owner. Borrow it instead of
    /// retaining an Arc whose eventual destruction could move to the GUI.
    pub fn collection_rows(&self) -> &CollectionRows { &self.collection_rows }
    #[cfg(test)]
    pub(in crate::ui) fn bind_test_rows(&mut self, rows: &Arc<Vec<LibItem>>) {
        self.collection_rows = Arc::new(CollectionRows::build(rows, &self.catalog));
    }
    #[cfg(test)]
    pub(in crate::ui) fn bind_incremental_test_rows(&mut self, rows: &Arc<Vec<LibItem>>) {
        self.collection_rows = Arc::new(CollectionRows::build_incremental(rows, &self.catalog, Some(&self.collection_rows)));
    }

    /// Bounded handoff: one pending result, plus the worker's one in-flight job.
    pub fn save_analysis(&mut self, completion: crate::engine::media_load::AnalysisCompletion)
        -> std::result::Result<(), crate::engine::media_load::AnalysisCompletion> {
        if self.worker_closed || self.analysis_active.is_some() || self.inspection_active.is_some() { return Err(completion); }
        self.analysis_active = Some(completion.token.id);
        self.analysis = Some(completion);
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        Ok(())
    }
    pub fn analysis_worker_available(&self) -> bool { !self.worker_closed }
    pub fn take_analysis_result(&mut self) -> Option<AnalysisReceipt> {
        let result = self.analysis_result.take();
        if result.is_some() { self.analysis_active = None; }
        result
    }
    pub fn inspect_analysis(&mut self, request: AnalysisInspect) -> std::result::Result<(), AnalysisInspect> {
        if self.worker_closed || self.analysis_active.is_some() || self.inspection_active.is_some() { return Err(request); }
        self.inspection_active = Some((request.id, request.reference.clone(), request.work.clone(), request.cancel.clone()));
        self.inspection = Some(request);
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        Ok(())
    }
    pub fn take_analysis_inspection(&mut self) -> Option<AnalysisInspected> {
        let result = self.inspection_result.take();
        if result.is_some() { self.inspection_active = None; }
        result
    }
    pub fn retire_analysis_inspection(&mut self, result: AnalysisInspected) -> std::result::Result<(), AnalysisInspected> {
        if self.worker_closed || self.retired_analysis_inspections.len() >= 2 { return Err(result); }
        self.retired_analysis_inspections.push(result);
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        Ok(())
    }
    /// Return a completed queue's captured large views to the metadata worker.
    /// The caller retains ownership and retries if the two retirement slots fill.
    pub fn retire_analysis_rows(&mut self, rows: Arc<Vec<LibItem>>, indices: Arc<Vec<usize>>)
        -> std::result::Result<(), (Arc<Vec<LibItem>>, Arc<Vec<usize>>)> {
        if self.worker_closed || self.retired_analysis_indices.len() >= 2 { return Err((rows, indices)); }
        self.retired_candidates.push(rows);
        self.retired_analysis_indices.push(indices);
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        Ok(())
    }
    /// Enqueue only freshly measured same-descriptor hash/decode proofs from
    /// media workers. A project/definition SourceRef is not a measured proof.
    /// At most 256 proofs wait here and one 256-proof job can be in flight.
    pub fn qualify_sampler(&mut self, proof: crate::sampler_bank::SourceRef) -> std::result::Result<(), String> {
        self.qualify_measured_content(proof)
    }
    pub fn qualify_measured_content(&mut self, proof: crate::sampler_bank::SourceRef) -> std::result::Result<(), String> {
        if self.storage.is_none() {
            return Err("sampler source identity is not saved: persistent DJ library unavailable".into());
        }
        proof.validate()?;
        if proof.content_hash.is_none() {
            return Err("sampler source identity requires a measured content digest".into());
        }
        if let Some(old) = self.sampler_sources.iter().find(|old| old.track == proof.track
            && old.source == proof.source && old.fingerprint == proof.fingerprint) {
            return if old.content_hash == proof.content_hash { Ok(()) }
                else { Err("sampler source identity conflicts with an already queued digest".into()) };
        }
        if self.sampler_sources.len() == SAMPLER_PROOF_LIMIT {
            return Err("sampler source identity queue is full (256); retry after the library save".into());
        }
        self.sampler_sources.push(proof);
        self.clear_error = true;
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        Ok(())
    }

    pub fn reserve_tags(&mut self) -> bool {
        if self.tag_reservation || self.active() || !self.ready() || self.storage.is_none() || self.collections_closing { return false; }
        self.tag_reservation = true;
        true
    }
    pub fn release_tags(&mut self) { self.tag_reservation = false; }
    pub fn save_tags(&mut self, request: tags::Save) -> std::result::Result<(), tags::Save> {
        if !self.tag_reservation || self.worker_closed || self.tag_save_id.is_some() || self.tag_save.is_some() || self.tag_result.is_some() { return Err(request); }
        self.tag_save_id = Some(request.id);
        self.tag_save = Some(request);
        self.clear_error = true;
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        Ok(())
    }
    pub fn take_tag_result(&mut self) -> Option<tags::Receipt> {
        let receipt = self.tag_result.take();
        if receipt.is_some() { self.tag_save_id = None; }
        receipt
    }

    pub fn ready(&self)->bool {self.initialized && !self.worker_closed && (self.storage.is_none() || self.durable)}
    pub fn set_performance(&mut self, performance: Handle) {
        self.performance = performance;
    }
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
        if self.storage.is_none() || self.import.is_some() || self.tag_reservation {
            return false;
        }
        let work = match self.performance.optional_work() {
            Ok(work) => Arc::new(work),
            Err(error) => {
                self.storage_error = Some(error.to_string());
                return false;
            }
        };
        self.import = Some(Import { path, work });
        self.clear_error = true;
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        true
    }
    pub fn relocate(&mut self, request: crate::library::Relocate) -> bool {
        self.relocate_choice(request, None)
    }
    pub fn relocate_reviewed(&mut self, request: crate::library::Relocate, candidate: crate::library::relocation_search::Candidate) -> bool {
        self.relocate_choice(request, Some(candidate))
    }
    fn relocate_choice(&mut self, request: crate::library::Relocate, reviewed: Option<crate::library::relocation_search::Candidate>) -> bool {
        if self.storage.is_none() || self.relocation.is_some() || self.in_flight || self.tag_reservation { return false; }
        let work = match self.performance.optional_work() {
            Ok(work) => Arc::new(work),
            Err(error) => {
                self.storage_error = Some(format!("Relocation was not accepted: {error}"));
                return false;
            }
        };
        self.relocation = Some(Relocation { request, work, reviewed });
        self.relocation_result = None;
        self.clear_error = true;
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
        true
    }
    pub fn relocation_result(&self, request: &crate::library::Relocate) -> Option<&std::result::Result<(), String>> {
        self.relocation_result.as_ref().filter(|result| result.request.id == request.id
            && result.request.destination == request.destination && result.request.source == request.source
            && result.request.fingerprint == request.fingerprint).map(|result| &result.outcome)
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
        let Some((items, work, roots)) = publication.stage(library) else {
            return;
        };
        if let Some(previous) = self.staged.replace(Staged { items, work, roots }) {
            previous
                .work
                .cancel()
                .store(true, std::sync::atomic::Ordering::Release);
            self.retired_candidates.push(previous.items);
            if let Some(roots)=previous.roots {self.retired_roots.push(roots);}
        }
        self.revision = self.revision.wrapping_add(1);
        self.dirty = true;
    }
    /// A preference change invalidates pending scans from the old root set.
    /// Existing visible rows remain until the new scan succeeds; retirement and
    /// metadata overlays still belong to this worker.
    pub fn cancel_scan(&mut self) {
        if let Some(candidate) = self.staged.take() {
            candidate
                .work
                .cancel()
                .store(true, std::sync::atomic::Ordering::Release);
            self.retired_candidates.push(candidate.items);
            if let Some(roots)=candidate.roots {self.retired_roots.push(roots);}
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
        if self.staged.as_ref().is_some_and(|staged| {
            staged
                .work
                .cancel()
                .load(std::sync::atomic::Ordering::Acquire)
        }) {
            self.cancel_scan();
        }
        if (self.deferred || self.qualification_pending) && !self.performance.protected() && !self.in_flight && !self.dirty {
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
        let received = self.results.try_recv();
        if matches!(received, Err(mpsc::TryRecvError::Disconnected)) {
            self.worker_closed = true;
            self.in_flight = false;
            self.dirty = false;
            let error = "DJ library worker unavailable; pending save outcome is unconfirmed";
            self.storage_error = Some(error.into());
            self.durable = false;
            if self.tag_result.is_none() {
                if let Some(id) = self.tag_save_id { self.tag_result = Some(tags::Receipt::refused(id, error.into())); }
            }
            if self.collection_result.is_none() {
                if let Some(token) = &self.collection_active {
                    self.collection_result = Some(CollectionReceipt::unavailable(token, self.catalog.crates.revision()));
                }
            }
            if self.analysis_result.is_none() {
                if let Some(id) = self.analysis_active {
                    self.analysis_result = Some(AnalysisReceipt { id, committed: false,
                        outcome: Err(crate::engine::media_load::AnalysisFailure::Failed(error.into())) });
                }
            }
            if self.inspection_result.is_none() {
                if let Some((id, reference, work, cancel)) = &self.inspection_active {
                    self.inspection_result = Some(AnalysisInspected { id: *id, reference: reference.clone(),
                        work: work.clone(), cancel: cancel.clone(), outcome: Err(error.into()) });
                }
            }
            return Err(error);
        }
        if let Ok(result) = received {
            self.initialized=true;
            if let Some(receipt) = result.tags { self.tag_result = Some(receipt); }
            if let Some(collection) = result.collection { self.collection_result = Some(collection); }
            if let Some(analysis) = result.analysis { self.analysis_result = Some(analysis); }
            if let Some(inspection) = result.inspection { self.inspection_result = Some(inspection); }
            if let Some(relocation) = result.relocation { self.relocation_result = Some(relocation); }
            self.qualification_pending = result.qualification_pending;
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
            let [full_index, restricted_index, base_index] = result.row_indices;
            if result.revision == self.revision && result.base.as_ptr() == Arc::as_ptr(library) {
                // New optional rows only become visible under a fresh commit
                // guard. Already durable results remain truthful while mode
                // protection selects the worker-built essential-only view.
                let additional_rows = !Arc::ptr_eq(&result.items, &result.restricted);
                let permit = (additional_rows && !self.performance.protected())
                    .then(|| self.performance.optional_work())
                    .transpose()
                    .ok()
                    .flatten();
                let guard = permit.as_ref().and_then(|permit| permit.commit().ok());
                self.deferred = additional_rows && guard.is_none();
                let (next, unused, next_index, unused_index) = if self.deferred {
                    (result.restricted, result.items, restricted_index, full_index)
                } else {
                    (result.items, result.restricted, full_index, restricted_index)
                };
                let previous_index = std::mem::replace(&mut self.collection_rows, next_index);
                let previous = std::mem::replace(library, next);
                let _ = result.retire.try_send((
                    previous,
                    self.staged.take().map(|staged| staged.items),
                    retired_catalog,
                    unused,
                    [previous_index, unused_index, base_index],
                ));
                drop(guard);
                published = true;
                self.dirty = false;
            } else {
                // The catalog receipt remains current even when row publication
                // is stale. Its base table still gives saved member labels O(1);
                // row lookup refuses any different actual row allocation.
                let previous_index = std::mem::replace(&mut self.collection_rows, base_index);
                self.deferred |= !Arc::ptr_eq(&result.items, &result.restricted);
                let _ = result.retire.try_send((
                    result.items,
                    None,
                    retired_catalog,
                    result.restricted,
                    [previous_index, full_index, restricted_index],
                ));
                self.dirty = true;
            }
        }
        if self.dirty && !self.in_flight {
            let job = Job {
                tags: self.tag_save.take(),
                collection: self.collection.take(),
                base: library.clone(),
                candidate: self
                    .staged
                    .as_ref()
                    .map_or(&*library, |staged| &staged.items)
                    .clone(),
                scan_work: self.staged.as_ref().map(|staged| staged.work.clone()),
                roots: self.staged.as_ref().and_then(|staged|staged.roots.clone()),
                restricted: self.deferred || self.performance.protected(),
                _retired_candidates: std::mem::take(&mut self.retired_candidates),
                _retired_roots:std::mem::take(&mut self.retired_roots),
                _retired_file_reviews:std::mem::take(&mut self.retired_file_reviews),
                revision: self.revision,
                updates: std::mem::take(&mut self.pending),
                captures: std::mem::take(&mut self.captures),
                sampler_sources: std::mem::take(&mut self.sampler_sources),
                import: self.import.take(),
                relocation: self.relocation.take(),
                qualification_work: self.performance.optional_work().ok(),
                analysis: self.analysis.take(),
                inspection: self.inspection.take(),
                retired_analysis_indices: std::mem::take(&mut self.retired_analysis_indices),
                retired_analysis_inspections: std::mem::take(&mut self.retired_analysis_inspections),
            };
            match self.jobs.try_send(job) {
                Ok(()) => {
                    self.dirty = false;
                    self.in_flight = true;
                }
                Err(mpsc::TrySendError::Full(job) | mpsc::TrySendError::Disconnected(job)) => {
                    self.tag_save = job.tags;
                    self.collection = job.collection;
                    self.analysis = job.analysis;
                    self.inspection = job.inspection;
                    self.retired_analysis_indices = job.retired_analysis_indices;
                    self.retired_analysis_inspections = job.retired_analysis_inspections;
                    self.pending = job.updates;
                    self.captures = job.captures;
                    self.sampler_sources = job.sampler_sources;
                    self.import = job.import;
                    self.relocation = job.relocation;
                    self.retired_candidates = job._retired_candidates;
                    self.retired_roots=job._retired_roots;
                    self.retired_file_reviews=job._retired_file_reviews;
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
        self.dirty || self.in_flight || self.tag_reservation
    }
    #[cfg(test)]
    pub(super) fn pending_description(&self) -> String {
        format!(
            "revision={} dirty={} in_flight={} staged={} closed={} deferred={} pending={} captures={} protection={:?} storage_error={:?}",
            self.revision, self.dirty, self.in_flight, self.staged.is_some(), self.worker_closed,
            self.deferred, self.pending.len(), self.captures.len(), self.performance.status(),
            self.storage_error,
        )
    }
}

impl App {
    pub(super) fn poll_library_metadata(&mut self) {
        self.refresh_library_view(); // capture selection before an Arc swap
        let selected = self.library_view.indices.get(self.lib_sel).and_then(|&index| {
            let item = &self.library[index];
            self.library_metadata.catalog.track_for_version(&item.source, item.fingerprint)
                .map(|track| (item.source.clone(), track.id.clone()))
        });
        match self.library_metadata.poll(&mut self.library) {
            Ok(true) => {
                if let Some((source, id)) = selected {
                    if let Some(destination) = self.library_metadata.catalog.tracks.iter()
                        .find(|track| track.id == id && track.source != source).map(|track| track.source.clone()) {
                        self.follow_library_relocation(&source, &destination);
                    }
                }
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
