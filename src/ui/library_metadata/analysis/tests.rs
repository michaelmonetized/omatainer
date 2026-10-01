use super::*;
use crate::{
    engine::{media_analysis::tests::{Files, wav}, media_load::{Loader, AnalysisRequest}, performance::Handle},
    library::{Metadata as Stored, read}, sampler_bank::SourceRef, track_analysis::Fields,
    ui::{bpm::{Bpm, Origin}, LibItem, library_metadata::{Metadata, Patch as OldPatch}, library_store::Capture},
};
use std::time::{Duration, Instant, SystemTime};

struct Fixture {
    disk: Disk,
    loader: Loader,
    handle: Handle,
    store: Store,
    reference: SourceRef,
    files: Files,
}
impl Fixture {
    fn new() -> Self {
        let files = Files::new();
        let mut reference = files.source("analysis.wav", &wav(8000, 8000, 1, true));
        let path = files.0.join("catalog.json");
        let mut store = Store::open(path.clone()).unwrap();
        store.catalog.upsert(reference.source.clone(), Some(reference.fingerprint), Stored {
            title: "Retained title".into(), artist: "Retained artist".into(), bpm: Bpm::hint(145.0),
            key: "user key".into(), duration: Some(999.0), last_play: None,
        }).unwrap().preparation.cue = 0.5;
        reference.track = store.catalog.track(&reference.source).unwrap().id.clone();
        store.save().unwrap();
        let handle = Handle::default();
        let loader = Loader::start_with_performance(handle.clone()).unwrap();
        Self { disk: Disk::new(&path), loader, handle, store, reference, files }
    }
    fn path(&self) -> PathBuf { self.files.0.join("catalog.json") }
    fn prepared(&self, fields: Fields) -> AnalysisCompletion {
        self.loader.request_analysis(AnalysisRequest { reference: self.reference.clone(), fields }).unwrap();
        let mut result = None;
        wait(|| { result = self.loader.take_analysis_ready(); result.is_some() });
        let result = result.unwrap();
        assert!(result.result.is_ok());
        result
    }
    fn inspection(&self, id: u64, fields: Fields, force: bool) -> Inspect {
        Inspect { id, reference: self.reference.clone(), fields, force,
            work: Arc::new(self.handle.optional_work().unwrap()), cancel: Arc::new(AtomicBool::new(false)) }
    }
}
fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "analysis publication did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn actual_decode_persists_reopens_and_inspects_exact_cached_results() {
    let mut f = Fixture::new();
    let done = f.prepared(Fields::ALL);
    let receipt = save(&mut f.store, &mut f.disk, done);
    assert!(receipt.committed && receipt.outcome.is_ok());
    let version = f.store.catalog.version(&f.reference.source, Some(f.reference.fingerprint)).unwrap();
    let record = version.analysis.clone().unwrap();
    assert_eq!(record.bpm.as_ref().unwrap().value, None, "silent source must not invent 120 BPM");
    assert_eq!(record.duration.as_ref().unwrap().value, 1.0);
    assert_eq!(version.metadata.bpm, Bpm::UNKNOWN);
    assert_eq!(version.preparation.cue, 0.5);
    assert_eq!(version.metadata.title, "Retained title");
    let path = f.path();
    let request = f.inspection(1, Fields::ALL, false);
    let result = inspect(&f.store, &mut f.disk, request);
    assert!(!result.outcome.as_ref().unwrap().needed.valid());
    assert_eq!(result.outcome.as_ref().unwrap().record, Some(record.clone()));
    assert!(result.outcome.as_ref().unwrap().waveform.is_some());
    assert!(!result.work.cancelled());
    let request = f.inspection(2, Fields::ALL, false);
    let Fixture { store, disk, files: _files, .. } = f;
    drop(store);
    drop(disk);
    let reopened = Store::open(path.clone()).unwrap();
    let mut reopened_disk = Disk::new(&path);
    let result = inspect(&reopened, &mut reopened_disk, request);
    let cached = result.outcome.unwrap();
    assert_eq!(cached.record, Some(record));
    assert!(!cached.needed.valid() && cached.waveform.is_some());
}

#[test]
fn selective_inspection_preserves_completed_fields_and_detects_corrupt_waveform() {
    let mut f = Fixture::new();
    let duration = Fields { bpm: false, duration: true, waveform: false };
    let done = f.prepared(duration);
    assert!(save(&mut f.store, &mut f.disk, done).outcome.is_ok());
    let request = f.inspection(1, Fields::ALL, false);
    let cached = inspect(&f.store, &mut f.disk, request).outcome.unwrap();
    assert_eq!(cached.needed, Fields { bpm: true, duration: false, waveform: true });
    let old_duration = cached.record.unwrap().duration;
    let done = f.prepared(cached.needed);
    assert!(save(&mut f.store, &mut f.disk, done).outcome.is_ok());
    let request = f.inspection(2, Fields::ALL, false);
    let cached = inspect(&f.store, &mut f.disk, request).outcome.unwrap();
    assert!(!cached.needed.valid());
    assert_eq!(cached.record.as_ref().unwrap().duration, old_duration);
    let wave = cached.record.unwrap().waveform.unwrap().value;
    let name = format!("{}.wave.json", wave.sha256.iter().map(|v| format!("{v:02x}")).collect::<String>());
    std::fs::write(f.path().with_extension("analysis").join(name), b"corrupt").unwrap();
    let request = f.inspection(3, Fields::ALL, false);
    let cached = inspect(&f.store, &mut f.disk, request).outcome.unwrap();
    assert_eq!(cached.needed, Fields { bpm: false, duration: false, waveform: true });
    assert!(cached.waveform.is_none() && cached.notice.unwrap().contains("unavailable"));
    assert!(read(&f.path()).unwrap().version(&f.reference.source, Some(f.reference.fingerprint)).unwrap().analysis.is_some());
}

#[test]
fn user_cancellation_and_new_request_share_the_actual_publication_claim() {
    for phase in [0, 1, 2, 3] {
        let mut f = Fixture::new();
        let done = f.prepared(Fields::ALL);
        let token = done.token.clone();
        let old = std::fs::read(f.path()).unwrap();
        let receipt = save_with(&mut f.store, &mut f.disk, done, |at| {
            if at == phase { assert_eq!(token.cancel(), phase < 2); }
        });
        assert_eq!(receipt.committed, phase >= 2);
        if phase < 2 {
            assert_eq!(receipt.outcome, Err(AnalysisFailure::Cancelled));
            assert_eq!(std::fs::read(f.path()).unwrap(), old);
        } else { assert!(receipt.outcome.is_ok()); }
    }
    let mut f = Fixture::new();
    let done = f.prepared(Fields::ALL);
    let token = done.token.clone();
    let reference = f.reference.clone();
    let receipt = save_with(&mut f.store, &mut f.disk, done, |phase| {
        if phase == 2 {
            f.loader.request_analysis(AnalysisRequest { reference: reference.clone(), fields: Fields::ALL }).unwrap();
            assert!(!token.is_current());
        }
    });
    assert!(receipt.committed && receipt.outcome.is_ok());
}

#[test]
fn protection_cycle_and_another_commit_have_distinct_persistence_outcomes() {
    let mut f = Fixture::new();
    let done = f.prepared(Fields::ALL);
    f.handle.set_enabled(true).unwrap();
    f.handle.set_enabled(false).unwrap();
    let receipt = save(&mut f.store, &mut f.disk, done);
    assert_eq!(receipt.outcome, Err(AnalysisFailure::Protected));
    assert!(!receipt.committed);
    let done = f.prepared(Fields::ALL);
    let other = f.handle.optional_work().unwrap();
    let guard = other.commit().unwrap();
    let receipt = save(&mut f.store, &mut f.disk, done);
    assert!(matches!(&receipt.outcome, Err(AnalysisFailure::Failed(error)) if error.contains("already pending")));
    assert!(!receipt.committed);
    drop(guard);
}

#[test]
fn catalog_failure_before_and_after_rename_retains_truthful_memory_and_disk() {
    for phase in [0, 3] {
        let mut f = Fixture::new();
        let done = f.prepared(Fields::ALL);
        let old = std::fs::read(f.path()).unwrap();
        let receipt = save_using(&mut f.store, &mut f.disk, done, |_| {}, |store| {
            store.save_for_test(|at| if at == phase { Err("injected catalog I/O failure".into()) } else { Ok(()) })
        });
        assert!(receipt.outcome.is_err());
        assert_eq!(receipt.committed, phase == 3);
        let memory = f.store.catalog.version(&f.reference.source, Some(f.reference.fingerprint)).unwrap();
        let disk = read(&f.path()).unwrap();
        assert_eq!(memory, disk.version(&f.reference.source, Some(f.reference.fingerprint)).unwrap());
        assert_eq!(memory.analysis.is_some(), phase == 3);
        if phase == 0 { assert_eq!(std::fs::read(f.path()).unwrap(), old); }
        else { assert!(receipt.outcome.unwrap_err().to_string().contains("replacement committed")); }
    }
}

#[test]
fn metadata_rebase_cannot_restore_stale_automatic_values_after_analysis_commit() {
    for protected in [false, true] {
        let f = Fixture::new();
        let path = f.path();
        let mut rows = Arc::new(f.store.catalog.tracks.iter().map(|track|
            LibItem::from_stored(track.source.clone(), &track.versions[track.current])).collect::<Vec<_>>());
        let done = f.prepared(Fields::ALL);
        let id = done.token.id;
        let reference = f.reference.clone();
        drop(f.store);
        let mut owner = Metadata::new(Some(path.clone()));
        owner.set_performance(f.handle.clone());
        wait(|| { owner.poll(&mut rows).unwrap(); !owner.active() });
        owner.update(OldPatch { source: reference.source.clone(), fingerprint: reference.fingerprint,
            bpm: Bpm::new(99.0, Origin::Heuristic), duration: Some(999.0) });
        wait(|| { owner.poll(&mut rows).unwrap(); !owner.active() });
        assert!(owner.save_analysis(done).is_ok());
        owner.poll(&mut rows).unwrap();
        // Stop GUI polling until the actual immutable result is durable.
        wait(|| read(&path).ok().is_some_and(|catalog|
            catalog.version(&reference.source, Some(reference.fingerprint)).unwrap().analysis.is_some()));
        if protected { f.handle.set_enabled(true).unwrap(); }
        let mut captured = rows[0].stored_metadata();
        captured.last_play = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100));
        owner.capture(Capture { source: reference.source.clone(), fingerprint: Some(reference.fingerprint),
            metadata: captured, preparation: None, played: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100)) });
        wait(|| { owner.poll(&mut rows).unwrap(); !owner.active() });
        let receipt = owner.take_analysis_result().unwrap();
        assert_eq!(receipt.id, id);
        assert!(receipt.committed && receipt.outcome.is_ok());
        if protected { f.handle.set_enabled(false).unwrap(); }
        owner.retry_save();
        wait(|| { owner.poll(&mut rows).unwrap(); !owner.active() });
        assert_eq!(rows[0].bpm, Bpm::UNKNOWN);
        assert_eq!(rows[0].length, Some(1.0));
        let catalog = read(&path).unwrap();
        let version = catalog.version(&reference.source, Some(reference.fingerprint)).unwrap();
        assert_eq!(version.metadata.bpm, Bpm::UNKNOWN);
        assert_eq!(version.metadata.duration, Some(1.0));
        assert_eq!(version.metadata.last_play, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(100)));
        assert_eq!(version.preparation.cue, 0.5);
    }
}

#[test]
fn metadata_refuses_replacement_until_terminal_receipt_is_consumed() {
    let f = Fixture::new();
    let path = f.path();
    let first = f.prepared(Fields::ALL);
    let id = first.token.id;
    let request = f.inspection(22, Fields::ALL, false);
    drop(f.store);
    let mut rows = Arc::new(Vec::new());
    let mut owner = Metadata::new(Some(path));
    owner.set_performance(f.handle.clone());
    assert!(owner.save_analysis(first).is_ok());
    let request = owner.inspect_analysis(request).err().expect("active save must exclude inspection");
    wait(|| { owner.poll(&mut rows).unwrap(); !owner.active() });
    let request = owner.inspect_analysis(request).err().expect("unconsumed receipt must exclude inspection");
    assert_eq!(owner.take_analysis_result().unwrap().id, id);
    assert!(owner.inspect_analysis(request).is_ok());
    wait(|| { owner.poll(&mut rows).unwrap(); !owner.active() });
    let inspected = owner.take_analysis_inspection().unwrap();
    assert_eq!(inspected.id, 22);
    assert!(!inspected.outcome.as_ref().unwrap().needed.valid());
    f.handle.set_enabled(true).unwrap();
    f.handle.set_enabled(false).unwrap();
    assert!(inspected.work.cancelled(), "completed read retains its old permit through GUI publication");
    assert!(owner.retire_analysis_inspection(inspected).is_ok());
    wait(|| { owner.poll(&mut rows).unwrap(); !owner.active() });
}

#[test]
fn disconnected_metadata_owner_returns_uncertain_terminal_outcome_without_hanging() {
    for inspecting in [false, true] {
        let f = Fixture::new();
        let path = f.path();
        let done = (!inspecting).then(|| f.prepared(Fields::ALL));
        let request = inspecting.then(|| f.inspection(77, Fields::ALL, false));
        drop(f.store);
        let mut owner = Metadata::with_hook(path, || panic!("controlled metadata owner failure"));
        let mut rows = Arc::new(Vec::new());
        if let Some(done) = done { assert!(owner.save_analysis(done).is_ok()); }
        if let Some(request) = request { assert!(owner.inspect_analysis(request).is_ok()); }
        wait(|| { let _ = owner.poll(&mut rows); !owner.analysis_worker_available() });
        assert!(!owner.active() && !owner.durable);
        if inspecting {
            assert!(owner.take_analysis_inspection().unwrap().outcome.err().unwrap().contains("unconfirmed"));
        } else {
            let receipt = owner.take_analysis_result().unwrap();
            assert!(!receipt.committed);
            assert!(receipt.outcome.unwrap_err().to_string().contains("unconfirmed"));
        }
    }
}

#[test]
fn captured_rows_and_indices_retire_on_the_existing_metadata_owner() {
    let f = Fixture::new();
    let path = f.path();
    let captured = Arc::new(vec![LibItem::from_stored(f.reference.source.clone(),
        f.store.catalog.version(&f.reference.source, Some(f.reference.fingerprint)).unwrap())]);
    let indices = Arc::new(vec![0]);
    let weak_rows = Arc::downgrade(&captured);
    let weak_indices = Arc::downgrade(&indices);
    drop(f.store);
    let mut owner = Metadata::new(Some(path));
    let mut rows = Arc::new(Vec::new());
    assert!(owner.retire_analysis_rows(captured, indices).is_ok());
    owner.poll(&mut rows).unwrap();
    assert_eq!(weak_rows.strong_count(), 1, "the worker job has the only remaining row owner");
    assert_eq!(weak_indices.strong_count(), 1);
    wait(|| { owner.poll(&mut rows).unwrap(); !owner.active() });
    wait(|| weak_rows.strong_count() == 0 && weak_indices.strong_count() == 0);
}
