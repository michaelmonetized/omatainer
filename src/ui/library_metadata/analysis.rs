//! Analysis publication belongs to the existing sole catalog writer. A decoded
//! result and even a complete waveform blob are not a durable catalog receipt.
use crate::{
    engine::media_load::{AnalysisCompletion, AnalysisFailure},
    library::Store,
    track_analysis::{cache::Cache, Patch},
};
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicBool, Ordering}}};

pub(in crate::ui) struct Inspect {
    pub id: u64,
    pub reference: crate::sampler_bank::SourceRef,
    pub fields: crate::track_analysis::Fields,
    pub force: bool,
    pub work: Arc<crate::engine::performance::WorkPermit>,
    pub cancel: Arc<AtomicBool>,
}
impl Inspect {
    fn cancelled(&self) -> bool { self.work.cancelled() || self.cancel.load(Ordering::Acquire) }
}
pub(in crate::ui) struct Cached {
    pub needed: crate::track_analysis::Fields,
    pub record: Option<crate::track_analysis::Record>,
    pub waveform: Option<crate::track_analysis::cache::Waveform>,
    pub notice: Option<String>,
}
pub(in crate::ui) struct Inspected {
    pub id: u64,
    pub reference: crate::sampler_bank::SourceRef,
    pub work: Arc<crate::engine::performance::WorkPermit>,
    pub cancel: Arc<AtomicBool>,
    pub outcome: Result<Cached, String>,
}
impl Inspected {
    pub fn refused(request: Inspect, error: String) -> Self {
        Self { id: request.id, reference: request.reference, work: request.work,
            cancel: request.cancel, outcome: Err(error) }
    }
}

pub(super) fn inspect(store: &Store, disk: &mut Disk, mut request: Inspect) -> Inspected {
    let outcome = (|| {
        if request.cancelled() { return Err("Analysis inspection cancelled".into()); }
        request.reference.validate()?;
        if !request.fields.valid() { return Err("Choose at least one analysis field".into()); }
        let reference = &request.reference;
        let track = store.catalog.track_for_version(&reference.source, Some(reference.fingerprint))
            .filter(|track| track.id == reference.track).ok_or("Analysis source version is no longer in the catalog")?;
        let version = track.versions.iter().find(|v| v.fingerprint == Some(reference.fingerprint)).unwrap();
        if reference.content_hash.is_some() && reference.content_hash != version.content_hash {
            return Err("Analysis source digest no longer matches this catalog version".into());
        }
        if crate::engine::media_source::FileFingerprint::read(reference.path()?) != Some(reference.fingerprint) {
            return Err("Analysis source is missing or changed; saved results were retained".into());
        }
        request.reference.content_hash = version.content_hash;
        let record = version.analysis.as_ref().filter(|record|
            version.content_hash.is_some() && record.valid()).cloned();
        let mut needed = request.fields;
        if !request.force {
            if let Some(record) = &record {
                needed.bpm &= record.bpm.is_none();
                needed.duration &= record.duration.is_none();
                needed.waveform &= record.waveform.is_none();
            }
        }
        let mut notice = None;
        let waveform = if request.fields.waveform {
            if let Some(waveform) = record.as_ref().and_then(|record| record.waveform.as_ref()) {
                match disk.cache().and_then(|cache| cache.read(&waveform.value, || request.cancelled())) {
                    Ok(waveform) => Some(waveform),
                    Err(error) if !request.cancelled() => {
                        needed.waveform = true;
                        notice = Some(format!("Saved waveform unavailable; reanalysis is required: {error}"));
                        None
                    },
                    Err(error) => return Err(error),
                }
            } else { None }
        } else { None };
        if request.cancelled() { return Err("Analysis inspection cancelled".into()); }
        Ok(Cached { needed, record, waveform, notice })
    })();
    Inspected { id: request.id, reference: request.reference, work: request.work,
        cancel: request.cancel, outcome }
}

pub(super) struct Disk {
    root: PathBuf,
    cache: Option<Cache>,
}
impl Disk {
    pub fn new(catalog: &std::path::Path) -> Self {
        Self { root: catalog.with_extension("analysis"), cache: None }
    }
    pub fn cache(&mut self) -> Result<&mut Cache, String> {
        if self.cache.is_none() { self.cache = Some(Cache::open(self.root.clone())?); }
        Ok(self.cache.as_mut().unwrap())
    }
}

pub(in crate::ui) struct Receipt {
    pub id: u64,
    pub committed: bool,
    pub outcome: Result<(), AnalysisFailure>,
}
impl Receipt {
    pub fn refused(completion: AnalysisCompletion, error: String) -> Self {
        Self { id: completion.token.id, committed: false, outcome: Err(AnalysisFailure::Failed(error)) }
    }
}

pub(super) fn save(store: &mut Store, disk: &mut Disk, completion: AnalysisCompletion) -> Receipt {
    save_with(store, disk, completion, |_| {})
}
fn save_with(store: &mut Store, disk: &mut Disk, completion: AnalysisCompletion,
    mut checkpoint: impl FnMut(u8)) -> Receipt {
    let id = completion.token.id;
    let mut committed = false;
    let outcome = (|| {
        let prepared = completion.result?;
        let token = completion.token;
        let work = completion.work;
        let check = || {
            if work.cancelled() { return Err(AnalysisFailure::Protected); }
            if !token.is_current() {
                return Err(token.failure().unwrap_or(AnalysisFailure::Cancelled));
            }
            Ok(())
        };
        check()?;
        checkpoint(0);
        let waveform = if let Some(waveform) = prepared.waveform {
            let reference = disk.cache().map_err(AnalysisFailure::Failed)?
                .write(waveform, || check().is_err());
            // A cancellation error retains its typed reason, allowing explicit
            // foreground preemption to be retried separately from file failure.
            check()?;
            Some(reference.map_err(AnalysisFailure::Failed)?)
        } else { None };
        let patch = Patch { reference: prepared.reference, fields: prepared.fields,
            at_unix_ms: prepared.at_unix_ms, bpm: prepared.bpm,
            duration: prepared.duration, waveform };
        let mut candidate = store.catalog.clone();
        candidate.apply_analysis(&patch).map_err(AnalysisFailure::Failed)?;
        checkpoint(1);
        check()?;
        let guard = work.commit().map_err(|error| {
            if work.cancelled() { AnalysisFailure::Protected }
            else { AnalysisFailure::Failed(error.to_string()) }
        })?;
        token.claim_publication()?;
        checkpoint(2);
        // No token/current check after the shared claim: newer request admission
        // or a late Cancel cannot relabel this transaction's actual outcome.
        let baseline = std::mem::replace(&mut store.catalog, candidate);
        let saved = store.save();
        committed = saved.is_ok() || store.last_save_replaced();
        if !committed { store.catalog = baseline; }
        checkpoint(3);
        drop(guard);
        saved.map_err(AnalysisFailure::Failed)
    })();
    Receipt { id, committed, outcome }
}
