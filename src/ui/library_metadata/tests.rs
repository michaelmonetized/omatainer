use super::*;
use crate::engine::{decode::decode_audio, load_receipt::Media};
use crate::ui::test_support::{crate_frame, Fixture};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-bpm-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn wave(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        // Short constant signal deliberately returns the existing heuristic's
        // fallback 120. This fixture tests provenance, not detection accuracy.
        let frames = 1024u32;
        let mut bytes = b"RIFF".to_vec();
        bytes.extend((36 + frames * 2).to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(48000u32.to_le_bytes());
        bytes.extend(96000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend((frames * 2).to_le_bytes());
        for _ in 0..frames {
            bytes.extend(1000i16.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn wait(mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while !check() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn scan(f: &mut Fixture, directory: &Files) {
    assert!(f
        .app
        .library_scan
        .start(vec![directory.0.clone()], f.app.library.clone()));
    wait(|| {
        f.app.poll_library_scan();
        !f.app.library_scan.active()
    });
    finish(f);
}
fn finish(f: &mut Fixture) {
    wait(|| {
        f.app.poll_load_receipts();
        !f.app.library_metadata.active()
    });
}
fn find<'a>(f: &'a Fixture, path: &PathBuf) -> &'a LibItem {
    f.app
        .library
        .iter()
        .find(|item| item.source == LibSource::File(path.clone()))
        .unwrap()
}
fn select(f: &mut Fixture, path: &PathBuf) {
    f.app.lib_filter.clear();
    f.app.refresh_library_view();
    f.app.lib_sel = f
        .app
        .library_view
        .indices
        .iter()
        .position(|&i| f.app.library[i].source == LibSource::File(path.clone()))
        .unwrap();
    f.app.refresh_library_view();
}
fn apply_load(f: &mut Fixture) {
    let command = f.rt.cmd_rx.try_recv().unwrap();
    assert!(matches!(command, Command::DeckLoadRequested { .. }));
    f.rt.apply(command);
    finish(f);
}
fn visible(output: &egui::FullOutput, text: &str) -> bool {
    output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(label) if label.galley.text().contains(text)))
}

#[test]
fn real_decode_reconciles_hint_to_labeled_current_deck_and_sorted_crate_without_retargeting_selection(
) {
    let files = Files::new();
    let path = files.wave("Artist - Track 155.wav");
    let other = files.wave("Artist - Track 140.wav");
    let mut f = Fixture::new(48);
    f.app.loader = Some(Loader::start().unwrap());
    scan(&mut f, &files);
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    select(&mut f, &path);
    f.app.load_sel(0);
    f.poll_loads();
    // Decoder completion alone never commits crate analysis.
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    select(&mut f, &other);
    let before = f.app.library.clone();
    apply_load(&mut f);
    assert!(!Arc::ptr_eq(&before, &f.app.library));
    assert_eq!(f.rt.decks[0].bpm, 120.0);
    assert_eq!(find(&f, &path).bpm, Bpm::new(120.0, Origin::Heuristic));
    assert_eq!(find(&f, &other).bpm, Bpm::hint(140.0));
    assert_eq!(
        f.app.selected_library_item().unwrap().source,
        LibSource::File(other)
    );
    assert!(f
        .app
        .library
        .windows(2)
        .all(|pair| pair[0].bpm.value().unwrap_or(f32::INFINITY)
            <= pair[1].bpm.value().unwrap_or(f32::INFINITY)));
    let ctx = egui::Context::default();
    let output = crate_frame(&ctx, &mut f.app, 0.0, vec![]);
    assert!(visible(&output, "120.0 est"));
    assert!(visible(&output, "140.0 hint"));
    let output = ctx.run(Default::default(), |ctx| f.app.load_status(ctx));
    assert!(visible(
        &output,
        "BPM 120.0 (heuristic estimate · unverified)"
    ));
    scan(&mut f, &files);
    assert_eq!(
        find(&f, &path).bpm.origin,
        Origin::Heuristic,
        "rescan lost cached provenance"
    );
}

#[test]
fn explicit_user_value_wins_real_decode_submission_and_same_identity_rescans() {
    let files = Files::new();
    let path = files.wave("Correction 155.wav");
    let mut f = Fixture::new(48);
    f.app.loader = Some(Loader::start().unwrap());
    scan(&mut f, &files);
    let items = Arc::make_mut(&mut f.app.library);
    items
        .iter_mut()
        .find(|item| item.source == LibSource::File(path.clone()))
        .unwrap()
        .bpm = Bpm::new(133.25, Origin::User);
    select(&mut f, &path);
    f.app.load_sel(1);
    f.poll_loads();
    let command = f.rt.cmd_rx.try_recv().unwrap();
    assert!(
        matches!(&command, Command::DeckLoadRequested { media: Media::Decoded { audio, .. }, .. } if audio.bpm == 133.25)
    );
    f.rt.apply(command);
    finish(&mut f);
    assert_eq!(f.rt.decks[1].bpm, 133.25);
    assert_eq!(find(&f, &path).bpm, Bpm::new(133.25, Origin::User));
    scan(&mut f, &files);
    assert_eq!(find(&f, &path).bpm.origin, Origin::User);
    // Replacing the pathname retires corrections attached to the old bytes.
    let replacement = files.wave("replacement.wav");
    std::fs::rename(replacement, &path).unwrap();
    scan(&mut f, &files);
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
}

#[test]
fn rejected_cancelled_and_superseded_loads_do_not_publish_analysis() {
    let files = Files::new();
    let path = files.wave("Track 155.wav");
    for mode in ["rejected", "cancelled", "superseded"] {
        let mut f = Fixture::new(32);
        scan(&mut f, &files);
        select(&mut f, &path);
        f.app.load_sel(0);
        f.decoder_jobs.recv_timeout(Duration::from_secs(3)).unwrap();
        if mode == "rejected" {
            while f.app.engine.send(Command::Tap(Instant::now())).is_ok() {}
        }
        f.decoder_results
            .send((0, Ok(decode_audio(&path).unwrap())))
            .unwrap();
        f.poll_loads();
        if mode == "cancelled" {
            f.app.load_sel(0);
        }
        while let Ok(command) = f.rt.cmd_rx.try_recv() {
            f.rt.apply(command);
        }
        if mode == "superseded" {
            f.rt.apply(Command::DeckUnload { deck: 0 });
        }
        finish(&mut f);
        assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0), "{mode}");
        assert_eq!(find(&f, &path).length, None, "{mode} acquired a duration");
    }
}

#[test]
fn changed_bytes_during_decode_or_after_scan_cannot_acquire_old_analysis() {
    let files = Files::new();
    let path = files.wave("Track 155.wav");
    let mut f = Fixture::new(32);
    scan(&mut f, &files);
    select(&mut f, &path);
    let outgoing = f.rt.decks[0].audio.as_ref().unwrap().clone();
    f.app.load_sel(0);
    f.decoder_jobs.recv_timeout(Duration::from_secs(3)).unwrap();
    let old_report = decode_audio(&path).unwrap();
    let replacement = files.wave("replacement.wav");
    std::fs::rename(replacement, &path).unwrap();
    f.decoder_results.send((0, Ok(old_report))).unwrap();
    f.poll_loads();
    finish(&mut f);
    assert!(matches!(f.app.loads[0].as_ref().unwrap().phase, Phase::Failed(_)));
    assert!(f.rt.cmd_rx.try_recv().is_err());
    assert!(Arc::ptr_eq(f.rt.decks[0].audio.as_ref().unwrap(), &outgoing));
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    scan(&mut f, &files);
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    // A version replaced after scan is refused before it can install analysis.
    let other = files.wave("replacement2.wav");
    std::fs::rename(other, &path).unwrap();
    f.app.loader = Some(Loader::start().unwrap());
    f.app.load_sel(0);
    f.poll_loads();
    finish(&mut f);
    assert!(matches!(f.app.loads[0].as_ref().unwrap().phase, Phase::Failed(_)));
    assert!(f.rt.cmd_rx.try_recv().is_err());
    assert!(Arc::ptr_eq(f.rt.decks[0].audio.as_ref().unwrap(), &outgoing));
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    scan(&mut f, &files);
    assert_eq!(find(&f, &path).bpm, Bpm::hint(155.0));
    select(&mut f, &path);
    f.app.load_sel(0);
    f.poll_loads();
    apply_load(&mut f);
    assert_eq!(find(&f, &path).bpm.origin, Origin::Heuristic);
}

#[test]
fn pending_metadata_rebases_on_new_arc_and_preserves_intervening_user_correction() {
    let files = Files::new();
    let path = files.wave("Track 155.wav");
    let mut f = Fixture::new(32);
    scan(&mut f, &files);
    let patch = Patch {
        tags: None,
        source: LibSource::File(path.clone()),
        fingerprint: find(&f, &path).fingerprint.unwrap(),
        bpm: Bpm::new(120.0, Origin::Heuristic),
        duration: None,
    };
    f.app.library_metadata.update(patch);
    f.app.poll_library_metadata();
    assert!(f.app.library_metadata.in_flight);
    // The job has captured its old base. Replace it before any result can publish.
    let mut next = f.app.library.as_ref().clone();
    next.iter_mut()
        .find(|item| item.source == LibSource::File(path.clone()))
        .unwrap()
        .bpm = Bpm::new(131.0, Origin::User);
    f.app.library = Arc::new(next);
    f.app.library_metadata.rebase();
    finish(&mut f);
    assert_eq!(find(&f, &path).bpm, Bpm::new(131.0, Origin::User));
}

#[test]
fn unknown_and_invalid_bpm_have_explicit_state_and_cannot_sort_as_zero() {
    for value in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(Bpm::hint(value), Bpm::UNKNOWN);
        assert_eq!(Bpm::new(value, Origin::Heuristic).cell(), "— unknown");
    }
    let mut items = builtin_crate_items();
    items[0].bpm = Bpm::UNKNOWN;
    sort_crate(&mut items);
    assert_eq!(items.last().unwrap().bpm, Bpm::UNKNOWN);
    assert_eq!(
        Bpm::new(125.0, Origin::User).reconcile(Bpm::UNKNOWN).origin,
        Origin::User
    );
}

#[test]
fn scan_started_before_analysis_never_republishes_its_stale_filename_hint() {
    use std::sync::{atomic::AtomicBool, Mutex};
    let files = Files::new();
    let path = files.wave("Track 155.wav");
    let mut f = Fixture::new(48);
    f.app.loader = Some(Loader::start().unwrap());
    scan(&mut f, &files);
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let once = AtomicBool::new(false);
    let held = Mutex::new(held);
    let options = library_scan::Options {
        before_entry: Some(Arc::new(move |_| {
            if !once.swap(true, Ordering::AcqRel) {
                entered.send(()).unwrap();
                held.lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(4))
                    .unwrap();
            }
        })),
    };
    assert!(f
        .app
        .library_scan
        .start_with(vec![files.0.clone()], f.app.library.clone(), options));
    ready.recv_timeout(Duration::from_secs(3)).unwrap();
    select(&mut f, &path);
    f.app.load_sel(0);
    f.poll_loads();
    apply_load(&mut f);
    assert_eq!(find(&f, &path).bpm.origin, Origin::Heuristic);
    release.send(()).unwrap();
    wait(|| {
        f.app.poll_library_scan();
        assert_eq!(
            find(&f, &path).bpm.origin,
            Origin::Heuristic,
            "stale scan metadata became visible"
        );
        !f.app.library_scan.active() && !f.app.library_metadata.active()
    });
    assert_eq!(
        f.app.selected_library_item().unwrap().source,
        LibSource::File(path)
    );
}

#[test]
fn newer_patch_revision_wins_over_an_already_inflight_candidate() {
    let files = Files::new();
    let path = files.wave("Track 155.wav");
    let mut f = Fixture::new(32);
    scan(&mut f, &files);
    let mut patch = Patch {
        tags: None,
        source: LibSource::File(path.clone()),
        fingerprint: find(&f, &path).fingerprint.unwrap(),
        bpm: Bpm::new(120.0, Origin::Heuristic),
        duration: None,
    };
    f.app.library_metadata.update(patch.clone());
    f.app.poll_library_metadata();
    assert!(f.app.library_metadata.in_flight);
    patch.bpm = Bpm::new(118.0, Origin::Heuristic);
    f.app.library_metadata.update(patch);
    finish(&mut f);
    assert_eq!(find(&f, &path).bpm, Bpm::new(118.0, Origin::Heuristic));
}

fn settle_metadata(metadata: &mut Metadata, library: &mut Arc<Vec<LibItem>>) {
    wait(|| {
        metadata.poll(library).unwrap();
        !metadata.active()
    });
}

#[test]
fn sampler_proofs_survive_cancelled_optional_rebase_and_enable_move_after_original_disappears() {
    use crate::sampler_bank::SourceRef;
    use sha2::{Digest, Sha256};
    let files = Files::new();
    let original = files.wave("Sampler original.wav");
    let source = LibSource::File(original.clone());
    let fingerprint = FileFingerprint::read(&original).unwrap();
    let hash: [u8; 32] = Sha256::digest(std::fs::read(&original).unwrap()).into();
    let path = files.0.join("catalog/library.json");
    let mut store = crate::library::Store::open(path.clone()).unwrap();
    let saved_metadata = crate::library::Metadata {
        title: "Sampler identity".into(), artist: "Local".into(),
        bpm: Bpm::new(127.0, Origin::User), key: "Am".into(),
        duration: Some(1024.0 / 48000.0), last_play: None,
    };
    store.catalog.upsert(source.clone(), Some(fingerprint), saved_metadata.clone()).unwrap();
    let id = store.catalog.track(&source).unwrap().id.clone();
    store.save().unwrap();
    drop(store);
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let mut calls = 0;
    let mut metadata = Metadata::with_hook(path.clone(), move || {
        calls += 1;
        if calls == 2 {
            entered.send(()).unwrap();
            held.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    });
    let performance = Handle::default();
    metadata.set_performance(performance.clone());
    let mut library = Arc::new(builtin_crate_items());
    settle_metadata(&mut metadata, &mut library);
    let proof = SourceRef { track: id.clone(), source: source.clone(), fingerprint, content_hash: Some(hash) };
    metadata.qualify_sampler(proof.clone()).unwrap();
    let added = files.wave("Optional extra.wav");
    let mut scanner = library_scan::LibraryScan::default();
    scanner.set_performance(performance.clone());
    assert!(scanner.start(vec![files.0.clone()], library.clone()));
    let mut publication = None;
    wait(|| { publication = scanner.poll(); publication.is_some() });
    metadata.stage_scan(publication.unwrap(), &library);
    metadata.poll(&mut library).unwrap();
    ready.recv_timeout(Duration::from_secs(3)).unwrap();
    let mut latest_metadata = saved_metadata;
    latest_metadata.last_play = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1234));
    let mut incoming_metadata = latest_metadata.clone();
    // A late decoded estimate cannot erase the saved user tempo.
    incoming_metadata.bpm = Bpm::new(139.0, Origin::Heuristic);
    let preparation = crate::engine::preparation::Preparation {
        cue: 0.01, ..Default::default()
    };
    metadata.capture(super::super::library_store::Capture {
        source: source.clone(), fingerprint: Some(fingerprint), metadata: incoming_metadata,
        preparation: Some(preparation), played: None,
    });
    // No original path remains by the time the worker receives the proof.
    let destination = files.0.join("Moved sampler.wav");
    std::fs::rename(&original, &destination).unwrap();
    performance.set_enabled(true).unwrap();
    release.send(()).unwrap();
    settle_metadata(&mut metadata, &mut library);
    assert!(metadata.durable);
    let saved = crate::library::read(&path).unwrap();
    let version = saved.version(&source, Some(fingerprint)).unwrap();
    assert_eq!(version.content_hash, Some(hash));
    assert_eq!(version.metadata, latest_metadata);
    assert_eq!(version.preparation, preparation);
    assert!(saved.track(&LibSource::File(added)).is_none());
    assert_eq!(saved.track(&source).unwrap().id, id);
    performance.set_enabled(false).unwrap();
    let request = crate::library::Relocate { id: id.clone(), source: source.clone(), fingerprint, destination: destination.clone() };
    assert!(metadata.relocate(request.clone()));
    settle_metadata(&mut metadata, &mut library);
    assert!(metadata.relocation_result(&request).unwrap().is_ok());
    let reopened = crate::library::read(&path).unwrap();
    let resolved = proof.resolve(&reopened).unwrap();
    assert_eq!(resolved.source, LibSource::File(destination));
    assert_eq!(resolved.track, id);
    assert_eq!(resolved.content_hash, Some(hash));
    // A stale/conflicting producer cannot silently overwrite the durable hash.
    let mut conflict = proof;
    conflict.content_hash.as_mut().unwrap()[0] ^= 1;
    metadata.qualify_sampler(conflict).unwrap();
    settle_metadata(&mut metadata, &mut library);
    assert!(metadata.label().contains("content proof rejected"));
    assert_eq!(crate::library::read(&path).unwrap().tracks, reopened.tracks);
}

#[test]
fn sampler_proof_queue_is_bounded_deduplicated_and_rejects_unmeasured_claims() {
    use crate::sampler_bank::SourceRef;
    let files = Files::new();
    let file = files.wave("Queue.wav");
    let proof = SourceRef { track: crate::library::TrackId(format!("{:032x}", 1)),
        source: LibSource::File(file.clone()), fingerprint: FileFingerprint::read(&file).unwrap(),
        content_hash: Some([7; 32]) };
    let mut unavailable = Metadata::default();
    assert!(unavailable.qualify_sampler(proof.clone()).is_err());
    let mut metadata = Metadata::new(Some(files.0.join("catalog/library.json")));
    let mut unmeasured = proof.clone();
    unmeasured.content_hash = None;
    assert!(metadata.qualify_sampler(unmeasured).is_err());
    for i in 0..SAMPLER_PROOF_LIMIT {
        let mut next = proof.clone();
        next.track = crate::library::TrackId(format!("{:032x}", i + 1));
        metadata.qualify_sampler(next).unwrap();
    }
    metadata.qualify_sampler(proof.clone()).unwrap();
    assert_eq!(metadata.sampler_sources.len(), SAMPLER_PROOF_LIMIT);
    let mut conflict = proof.clone();
    conflict.content_hash = Some([9; 32]);
    assert!(metadata.qualify_sampler(conflict).is_err());
    let mut overflow = proof;
    overflow.track = crate::library::TrackId("f".repeat(32));
    assert!(metadata.qualify_sampler(overflow).unwrap_err().contains("queue is full"));
    assert_eq!(metadata.sampler_sources.len(), SAMPLER_PROOF_LIMIT);
}

#[test]
fn performance_cancelled_staged_scan_keeps_essential_metadata_durable() {
    let files = Files::new();
    let added = files.wave("Optional 143.wav");
    let path = files.0.join("catalog/library.json");
    let performance = Handle::default();
    let (entered, ready) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let mut calls = 0;
    let mut metadata = Metadata::with_hook(path.clone(), move || {
        calls += 1;
        if calls == 2 {
            entered.send(()).unwrap();
            held.recv_timeout(Duration::from_secs(3)).unwrap();
        }
    });
    metadata.set_performance(performance.clone());
    let mut library = Arc::new(builtin_crate_items());
    settle_metadata(&mut metadata, &mut library);
    let source = library[0].source.clone();
    let mut capture = library[0].stored_metadata();
    capture.bpm = Bpm::new(133.0, Origin::User);
    metadata.capture(super::super::library_store::Capture {
        source: source.clone(),
        fingerprint: None,
        metadata: capture,
        preparation: None,
        played: None,
    });
    let mut scanner = library_scan::LibraryScan::default();
    scanner.set_performance(performance.clone());
    assert!(scanner.start(vec![files.0.clone()], library.clone()));
    let mut publication = None;
    wait(|| {
        publication = scanner.poll();
        publication.is_some()
    });
    metadata.stage_scan(publication.unwrap(), &library);
    metadata.poll(&mut library).unwrap();
    ready.recv_timeout(Duration::from_secs(2)).unwrap();
    performance.set_enabled(true).unwrap();
    release.send(()).unwrap();
    settle_metadata(&mut metadata, &mut library);
    assert!(metadata.durable);
    assert!(library
        .iter()
        .all(|item| item.source != LibSource::File(added.clone())));
    let saved = crate::library::read(&path).unwrap();
    assert!(saved.track(&LibSource::File(added)).is_none());
    assert_eq!(
        saved.version(&source, None).unwrap().metadata.bpm,
        Bpm::new(133.0, Origin::User)
    );
    assert_eq!(
        library
            .iter()
            .find(|item| item.source == source)
            .unwrap()
            .bpm,
        Bpm::new(133.0, Origin::User)
    );
    wait(|| performance.status().optional_active == 0);
}

#[test]
fn performance_preserves_committed_import_but_defers_new_rows_and_cancels_uncommitted_import() {
    let files = Files::new();
    let added = files.wave("Imported 142.wav");
    let source = LibSource::File(added.clone());
    let import_path = files.0.join("import/library.json");
    let mut imported = crate::library::Store::open(import_path.clone()).unwrap();
    let mut details = builtin_crate_items()[0].stored_metadata();
    details.title = "Imported".into();
    imported
        .catalog
        .upsert(source.clone(), FileFingerprint::read(&added), details)
        .unwrap();
    imported.save().unwrap();
    drop(imported);
    let performance = Handle::default();
    let path = files.0.join("destination/library.json");
    let mut metadata = Metadata::new(Some(path.clone()));
    metadata.set_performance(performance.clone());
    let mut library = Arc::new(builtin_crate_items());
    settle_metadata(&mut metadata, &mut library);
    assert!(metadata.import(import_path));
    metadata.poll(&mut library).unwrap();
    // Observe the real durable transaction without consuming its queued GUI result.
    wait(|| {
        crate::library::read(&path)
            .ok()
            .is_some_and(|catalog| catalog.track(&source).is_some())
    });
    wait(|| performance.set_enabled(true).is_ok());
    settle_metadata(&mut metadata, &mut library);
    assert!(metadata.durable);
    assert!(metadata.catalog.track(&source).is_some());
    assert!(library.iter().all(|item| item.source != source));
    assert!(metadata.deferred);
    assert!(!metadata.import(files.0.join("missing-while-protected")));
    performance.set_enabled(false).unwrap();
    settle_metadata(&mut metadata, &mut library);
    assert!(library.iter().any(|item| item.source == source));

    // Cancellation precedes any import read; essential capture still persists.
    assert!(metadata.import(files.0.join("missing-cancelled-before-read")));
    let stable = library
        .iter()
        .find(|item| matches!(item.source, LibSource::Builtin(_)))
        .unwrap();
    let stable_source = stable.source.clone();
    let mut details = stable.stored_metadata();
    details.bpm = Bpm::new(137.0, Origin::User);
    metadata.capture(super::super::library_store::Capture {
        source: stable_source.clone(),
        fingerprint: None,
        metadata: details,
        preparation: None,
        played: None,
    });
    performance.set_enabled(true).unwrap();
    settle_metadata(&mut metadata, &mut library);
    assert!(metadata.durable);
    assert!(
        metadata
            .label()
            .contains("cancelled the optional catalog import"),
        "{}",
        metadata.label()
    );
    assert_eq!(
        crate::library::read(&path)
            .unwrap()
            .version(&stable_source, None)
            .unwrap()
            .metadata
            .bpm,
        Bpm::new(137.0, Origin::User)
    );
}

#[test]
fn performance_committed_scan_defers_added_and_replaced_identities_until_studio() {
    let files = Files::new();
    let stable = files.wave("Original 140.wav");
    let performance = Handle::default();
    let path = files.0.join("catalog/library.json");
    let mut metadata = Metadata::new(Some(path.clone()));
    metadata.set_performance(performance.clone());
    let mut library = Arc::new(builtin_crate_items());
    let mut scanner = library_scan::LibraryScan::default();
    scanner.set_performance(performance.clone());
    settle_metadata(&mut metadata, &mut library);
    assert!(scanner.start(vec![files.0.clone()], library.clone()));
    let mut publication = None;
    wait(|| {
        publication = scanner.poll();
        publication.is_some()
    });
    metadata.stage_scan(publication.take().unwrap(), &library);
    settle_metadata(&mut metadata, &mut library);
    let source = LibSource::File(stable.clone());
    let original = library
        .iter()
        .find(|item| item.source == source)
        .unwrap()
        .clone();
    let added = LibSource::File(files.wave("Added 141.wav"));
    // Change the identity without changing path: a protected publication cannot
    // replace the visible old version merely because its source already exists.
    std::fs::write(&stable, b"different bytes and length").unwrap();
    let new_fingerprint = FileFingerprint::read(&stable);
    assert_ne!(new_fingerprint, original.fingerprint);
    assert!(scanner.start(vec![files.0.clone()], library.clone()));
    wait(|| {
        publication = scanner.poll();
        publication.is_some()
    });
    metadata.stage_scan(publication.take().unwrap(), &library);
    metadata.poll(&mut library).unwrap();
    wait(|| {
        crate::library::read(&path)
            .ok()
            .is_some_and(|catalog| catalog.track(&added).is_some())
    });
    wait(|| performance.set_enabled(true).is_ok());
    settle_metadata(&mut metadata, &mut library);
    assert!(metadata.durable);
    assert!(metadata.catalog.track(&added).is_some());
    assert!(library.iter().all(|item| item.source != added));
    assert_eq!(
        library
            .iter()
            .find(|item| item.source == source)
            .unwrap()
            .fingerprint,
        original.fingerprint
    );
    performance.set_enabled(false).unwrap();
    settle_metadata(&mut metadata, &mut library);
    assert!(library.iter().any(|item| item.source == added));
    assert_eq!(
        library
            .iter()
            .find(|item| item.source == source)
            .unwrap()
            .fingerprint,
        new_fingerprint
    );
}

#[test]
fn performance_committed_import_never_leaks_existing_row_metadata_but_keeps_essential_capture() {
    let files = Files::new();
    let performance = Handle::default();
    let path = files.0.join("destination/library.json");
    let mut metadata = Metadata::new(Some(path.clone()));
    metadata.set_performance(performance.clone());
    let mut library = Arc::new(builtin_crate_items());
    settle_metadata(&mut metadata, &mut library);
    let imported_source = library[0].source.clone();
    let essential_source = library[1].source.clone();
    let original = library[0].clone();
    let import_path = files.0.join("import/library.json");
    let mut imported = crate::library::Store::open(import_path.clone()).unwrap();
    let mut details = original.stored_metadata();
    details.title = "Imported title".into();
    details.bpm = Bpm::new(133.0, Origin::User);
    details.duration = Some(92.0);
    imported
        .catalog
        .upsert(imported_source.clone(), None, details)
        .unwrap();
    imported.save().unwrap();
    drop(imported);
    assert!(metadata.import(import_path));
    let mut essential = library[1].stored_metadata();
    essential.bpm = Bpm::new(137.0, Origin::User);
    metadata.capture(super::super::library_store::Capture {
        source: essential_source.clone(),
        fingerprint: None,
        metadata: essential,
        preparation: None,
        played: Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(300)),
    });
    metadata.poll(&mut library).unwrap();
    wait(|| {
        crate::library::read(&path).ok().is_some_and(|catalog| {
            catalog
                .version(&imported_source, None)
                .is_some_and(|v| v.metadata.bpm == Bpm::new(133.0, Origin::User))
        })
    });
    wait(|| performance.set_enabled(true).is_ok());
    // Force the first durable worker result to become stale before GUI polling.
    // Its essential capture must still reach the protected view on rebase.
    metadata.retry_save();
    settle_metadata(&mut metadata, &mut library);
    assert!(metadata.durable);
    assert_eq!(
        metadata
            .catalog
            .version(&imported_source, None)
            .unwrap()
            .metadata
            .title,
        "Imported title"
    );
    let visible = library
        .iter()
        .find(|item| item.source == imported_source)
        .unwrap();
    assert_eq!(visible.bpm, original.bpm);
    assert_eq!(visible.title, original.title);
    assert_eq!(visible.length, original.length);
    let essential = library
        .iter()
        .find(|item| item.source == essential_source)
        .unwrap();
    assert_eq!(essential.bpm, Bpm::new(137.0, Origin::User));
    assert_eq!(
        essential.last_play,
        Some(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(300))
    );
    // Further essential saves during protection also must not republish the
    // committed imported version through a normal catalog rebase.
    let mut subsequent = essential.stored_metadata();
    subsequent.duration = Some(81.0);
    metadata.capture(super::super::library_store::Capture {
        source: essential_source.clone(),
        fingerprint: None,
        metadata: subsequent,
        preparation: None,
        played: None,
    });
    settle_metadata(&mut metadata, &mut library);
    assert_eq!(
        library
            .iter()
            .find(|item| item.source == imported_source)
            .unwrap()
            .bpm,
        original.bpm
    );
    assert_eq!(
        library
            .iter()
            .find(|item| item.source == essential_source)
            .unwrap()
            .length,
        Some(81.0)
    );
    performance.set_enabled(false).unwrap();
    settle_metadata(&mut metadata, &mut library);
    let visible = library
        .iter()
        .find(|item| item.source == imported_source)
        .unwrap();
    assert_eq!(visible.bpm, Bpm::new(133.0, Origin::User));
    assert_eq!(visible.title, "Imported title");
}

#[test]
fn watched_root_bookmarks_commit_with_catalog_and_cancelled_scan_preserves_essential_cues() {
    let files=Files::new();let wave=files.wave("observed.wav");let path=files.0.join("saved/library.json");
    let mut catalog=crate::library::Catalog::default();let source=LibSource::File(wave.clone());let fp=FileFingerprint::read(&wave).unwrap();
    catalog.upsert(source.clone(),Some(fp),builtin_crate_items()[0].stored_metadata()).unwrap();
    let mut store=crate::library::Store::open(path.clone()).unwrap();store.catalog=catalog;store.save().unwrap();drop(store);
    let (entered,seen)=mpsc::sync_channel(1);let (release,held)=mpsc::sync_channel(1);let mut calls=0;
    let mut metadata=Metadata::with_hook(path.clone(),move ||{calls+=1;if calls==2 {entered.send(()).unwrap();held.recv_timeout(Duration::from_secs(5)).unwrap();}});
    let mut library=Arc::new(builtin_crate_items());settle_metadata(&mut metadata,&mut library);
    let performance=Handle::default();metadata.set_performance(performance.clone());let mut scan=LibraryScan::default();scan.set_performance(performance.clone());
    assert!(scan.start_watched(vec![files.0.clone()],library.clone(),"Live".into(),metadata.catalog.clone()));
    let mut publication=None;wait(||{publication=scan.poll();publication.is_some()});metadata.stage_scan(publication.unwrap(),&library);metadata.poll(&mut library).unwrap();seen.recv_timeout(Duration::from_secs(3)).unwrap();
    let preparation=crate::engine::preparation::Preparation {cue:0.005,..Default::default()};
    metadata.capture(super::super::library_store::Capture {source:source.clone(),fingerprint:Some(fp),metadata:library.iter().find(|i|i.source==source).unwrap().stored_metadata(),preparation:Some(preparation),played:None});
    performance.set_enabled(true).unwrap();release.send(()).unwrap();settle_metadata(&mut metadata,&mut library);
    assert!(metadata.durable);let reopened=crate::library::read(&path).unwrap();assert!(reopened.watched_roots.binding("Live",&files.0).is_none());assert_eq!(reopened.version(&source,Some(fp)).unwrap().preparation,preparation);
    performance.set_enabled(false).unwrap();assert!(scan.start_watched(vec![files.0.clone()],library.clone(),"Live".into(),metadata.catalog.clone()));
    let mut publication=None;wait(||{publication=scan.poll();publication.is_some()});metadata.stage_scan(publication.unwrap(),&library);settle_metadata(&mut metadata,&mut library);
    let reopened=crate::library::read(&path).unwrap();assert_eq!(reopened.watched_roots.binding("Live",&files.0).unwrap().source,LibSource::File(files.0.clone()));
    assert!(scan.start_watched(Vec::new(),library.clone(),"Live".into(),metadata.catalog.clone()));let mut publication=None;wait(||{publication=scan.poll();publication.is_some()});metadata.stage_scan(publication.unwrap(),&library);settle_metadata(&mut metadata,&mut library);
    let reopened=crate::library::read(&path).unwrap();assert!(reopened.watched_roots.binding("Live",&files.0).is_none());assert_eq!(reopened.version(&source,Some(fp)).unwrap().preparation,preparation);assert!(wave.is_file());
}
