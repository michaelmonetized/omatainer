use super::*;
use crate::{
    engine::{
        media_analysis::tests::{wav, Files},
        media_source::LibSource,
        performance::Handle,
    },
    library::{self, Metadata as Stored},
    ui::{
        bpm::{Bpm, Origin},
        library_store::Capture,
        LibItem,
    },
};
use std::{
    sync::{atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "collection owner did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn create(name: &str) -> Action {
    Action::Create {
        name: name.into(),
        parent: None,
        before: None,
    }
}
fn request(handle: &Handle, id: u64, expected: u64, action: Action) -> Request {
    let work = if matches!(action, Action::Read) {
        None
    } else {
        Some(handle.optional_work().unwrap())
    };
    Request {
        token: Token {
            id,
            state: Arc::new(AtomicU8::new(PENDING)),
            cancel: work.as_ref().map(WorkPermit::cancel),
        },
        expected,
        action,
        work,
    }
}
fn store(files: &Files) -> Store {
    let proof = files.source("source.wav", &wav(8000, 8000, 1, false));
    let mut store = Store::open(files.0.join("catalog.json")).unwrap();
    store
        .catalog
        .upsert(
            proof.source,
            Some(proof.fingerprint),
            Stored {
                title: "Stored".into(),
                artist: "Artist".into(),
                bpm: Bpm::new(128.0, Origin::User),
                key: "Am".into(),
                duration: Some(1.0),
                last_play: None,
            },
        )
        .unwrap()
        .preparation
        .cue = 0.5;
    store.save().unwrap();
    store
}
fn settle(metadata: &mut Metadata, rows: &mut Arc<Vec<LibItem>>) {
    wait(|| {
        metadata.poll(rows).unwrap();
        !metadata.active()
    });
}
fn submit(metadata: &mut Metadata, rows: &mut Arc<Vec<LibItem>>, action: Action) -> Receipt {
    let token = metadata
        .edit_crates(metadata.catalog.crates.revision(), action)
        .unwrap();
    settle(metadata, rows);
    let receipt = metadata.take_collection_result().unwrap();
    assert_eq!(receipt.id, token.id);
    receipt
}
fn created(receipt: Receipt) -> CrateId {
    assert!(
        matches!(receipt.outcome, Outcome::Durable { changed: true }),
        "{:?}",
        receipt.outcome
    );
    receipt.created.unwrap()
}

#[test]
fn playlist_review_single_writer_save_reopen_preserves_order_versions_and_source_bytes() {
    let files=Files::new();let original=store(&files);let path=files.0.join("catalog.json");let existing=original.catalog.tracks[0].clone();drop(original);
    let new=files.0.join("new.flac");std::fs::write(&new,include_bytes!("../../../../tests/fixtures/audio/tone.flac")).unwrap();
    let playlist=files.0.join("Tour.m3u8");std::fs::write(&playlist,"source.wav\nnew.flac\nsource.wav\nmissing.mp3\n").unwrap();let bytes=std::fs::read(&playlist).unwrap();
    let mut metadata=Metadata::new(Some(path.clone()));let mut rows=Arc::new(Vec::new());settle(&mut metadata,&mut rows);
    let before=std::fs::read(&path).unwrap();let receipt=submit(&mut metadata,&mut rows,Action::ReviewPlaylist(crate::playlist_import::Input{path:playlist.clone(),mapping:None}));assert_eq!(receipt.outcome,Outcome::Read);
    let review=receipt.review.unwrap();assert_eq!(review.rows.iter().filter(|r|r.ready()).count(),3);assert_eq!(std::fs::read(&path).unwrap(),before);
    let id=created(submit(&mut metadata,&mut rows,Action::ImportPlaylist{review,selected:vec![0],new_snapshot:false}));
    drop(metadata);let mut reopened=None;wait(||{reopened=Store::open(path.clone()).ok();reopened.is_some()});let reopened=reopened.unwrap();let members=&reopened.catalog.crates.node(&id).unwrap().members;assert_eq!(members.len(),2);assert_eq!(members[0],existing.id);assert_eq!(reopened.catalog.track(&existing.source).unwrap().versions,existing.versions);
    assert!(rows.iter().any(|row|row.source==LibSource::File(new.clone())));assert_eq!(std::fs::read(playlist).unwrap(),bytes);assert_eq!(std::fs::read(new).unwrap(),include_bytes!("../../../../tests/fixtures/audio/tone.flac"));
}

#[test]
fn playlist_cancel_and_failed_save_do_not_publish_partial_tracks_or_crates() {
    for mode in [0,1,2] {
        let files=Files::new();let mut store=store(&files);let handle=Handle::default();
        std::fs::write(files.0.join("new.flac"),include_bytes!("../../../../tests/fixtures/audio/tone.flac")).unwrap();let path=files.0.join("Atomic.m3u");std::fs::write(&path,"source.wav\nnew.flac\n").unwrap();
        let reviewed=apply(&mut store,request(&handle,1,0,Action::ReviewPlaylist(crate::playlist_import::Input{path,mapping:None}))).review.unwrap();
        let pending=request(&handle,2,0,Action::ImportPlaylist{review:reviewed,selected:vec![0],new_snapshot:false});let token=pending.token.clone();let before=std::fs::read(files.0.join("catalog.json")).unwrap();let tracks=store.catalog.tracks.clone();
        let result=apply_using(&mut store,pending,|phase|{if phase==0 && mode==0 {assert!(token.cancel());}},|store|if mode==1 {store.save_for_test(|phase|if phase==1 {Err("injected pre-replacement failure".into())}else{Ok(())})}else{store.save()});
        if mode<2 {assert!(matches!(result.outcome,Outcome::Rejected(_)));assert_eq!(store.catalog.tracks,tracks);assert!(store.catalog.crates.nodes().is_empty());assert_eq!(std::fs::read(files.0.join("catalog.json")).unwrap(),before);}
        else {assert!(matches!(result.outcome,Outcome::Durable{changed:true}));drop(store);assert_eq!(Store::open(files.0.join("catalog.json")).unwrap().catalog.crates.nodes().len(),1);}
    }
}

#[test]
fn actual_single_writer_nested_edits_receipts_and_restart_preserve_audio() {
    let files = Files::new();
    let original = store(&files);
    let versions = original.catalog.tracks.clone();
    let path = files.0.join("catalog.json");
    let LibSource::File(audio) = &versions[0].source else {
        unreachable!()
    };
    let bytes = std::fs::read(audio).unwrap();
    let fp = crate::engine::media_source::FileFingerprint::read(audio).unwrap();
    drop(original);
    let mut metadata = Metadata::new(Some(path.clone()));
    let mut rows = Arc::new(Vec::new());
    settle(&mut metadata, &mut rows);
    let root = created(submit(&mut metadata, &mut rows, create("Warmup")));
    let child = created(submit(
        &mut metadata,
        &mut rows,
        Action::Create {
            name: "Peak".into(),
            parent: Some(root.clone()),
            before: None,
        },
    ));
    let track = versions[0].id.clone();
    for target in [&root, &child] {
        assert!(matches!(
            submit(
                &mut metadata,
                &mut rows,
                Action::Edit(Edit::AddMembers {
                    id: target.clone(),
                    members: vec![track.clone()],
                    before: None
                })
            )
            .outcome,
            Outcome::Durable { changed: true }
        ));
    }
    assert!(matches!(
        submit(
            &mut metadata,
            &mut rows,
            Action::Edit(Edit::Rename {
                id: child.clone(),
                name: "Final".into()
            })
        )
        .outcome,
        Outcome::Durable { changed: true }
    ));
    let forest = metadata.catalog.crates.clone();
    assert_eq!(forest.node(&root).unwrap().children, vec![child.clone()]);
    assert_eq!(forest.node(&root).unwrap().members, vec![track.clone()]);
    assert_eq!(forest.node(&child).unwrap().members, vec![track]);
    let receipt = submit(&mut metadata, &mut rows, Action::Read);
    assert!(matches!(receipt.outcome, Outcome::Read));
    assert_eq!(receipt.revision, forest.revision());
    // The worker owns the store until it observes shutdown. Reopen via read's
    // independent descriptor while active, then wait for writer lock release.
    assert_eq!(library::read(&path).unwrap().crates, forest);
    drop(metadata);
    let mut reopened = None;
    wait(|| {
        reopened = Store::open(path.clone()).ok();
        reopened.is_some()
    });
    assert_eq!(reopened.as_ref().unwrap().catalog.crates, forest);
    assert_eq!(reopened.unwrap().catalog.tracks, versions);
    assert_eq!(std::fs::read(audio).unwrap(), bytes);
    assert_eq!(
        crate::engine::media_source::FileFingerprint::read(audio),
        Some(fp)
    );
}

#[test]
fn cancellation_claim_and_protection_have_one_irreversible_boundary() {
    for phase in [0, 1, 2] {
        let files = Files::new();
        let mut store = store(&files);
        let handle = Handle::default();
        let request = request(&handle, 1, 0, create("New"));
        let token = request.token.clone();
        let old = std::fs::read(files.0.join("catalog.json")).unwrap();
        let result = apply_using(
            &mut store,
            request,
            |at| {
                if at == phase {
                    assert_eq!(token.cancel(), phase == 0);
                }
            },
            Store::save,
        );
        if phase == 0 {
            assert_eq!(result.outcome, Outcome::Rejected(Failure::Cancelled));
            assert_eq!(std::fs::read(files.0.join("catalog.json")).unwrap(), old);
            assert!(store.catalog.crates.nodes().is_empty());
        } else {
            assert!(matches!(result.outcome, Outcome::Durable { changed: true }));
            assert_eq!(store.catalog.crates.nodes().len(), 1);
        }
    }
    let files = Files::new();
    let mut store = store(&files);
    let handle = Handle::default();
    let cancelled = request(&handle, 2, 0, create("Cancelled by quick protection"));
    handle.set_enabled(true).unwrap();
    handle.set_enabled(false).unwrap();
    assert!(matches!(
        apply(&mut store, cancelled).outcome,
        Outcome::Rejected(Failure::Performance(performance::Error::Protected))
    ));
    let admitted = request(&handle, 3, 0, create("Commit owns protection boundary"));
    let result = apply_using(
        &mut store,
        admitted,
        |phase| {
            if phase == 1 {
                assert_eq!(handle.set_enabled(true), Err(performance::Error::Changing));
            }
        },
        Store::save,
    );
    assert!(matches!(result.outcome, Outcome::Durable { changed: true }));
    handle.set_enabled(true).unwrap();
    let read = request(&handle, 4, 1, Action::Read);
    assert!(matches!(apply(&mut store, read).outcome, Outcome::Read));
}

#[test]
fn real_precommit_and_postrename_failures_keep_catalog_and_retry_truthful() {
    for phase in [0, 1, 2, 3, 4] {
        let files = Files::new();
        let mut store = store(&files);
        let handle = Handle::default();
        let old = std::fs::read(files.0.join("catalog.json")).unwrap();
        let op = request(&handle, 1, 0, create("Failure fixture"));
        let result = apply_using(
            &mut store,
            op,
            |_| {},
            |store| {
                store.save_for_test(|at| {
                    if at == phase {
                        Err("injected ENOSPC/catalog sync failure".into())
                    } else {
                        Ok(())
                    }
                })
            },
        );
        let disk = library::read(&files.0.join("catalog.json")).unwrap();
        assert_eq!(store.catalog.crates, disk.crates);
        if phase == 3 {
            assert!(matches!(result.outcome, Outcome::CommittedUnconfirmed(_)));
            assert!(result.created.is_some());
            assert_eq!(result.revision, 1);
            // Retrying persistence is a save, not a duplicate Create.
            store.save().unwrap();
            assert_eq!(store.catalog.crates.nodes().len(), 1);
        } else {
            assert!(matches!(
                result.outcome,
                Outcome::Rejected(Failure::Storage(_))
            ));
            assert!(result.created.is_none());
            assert_eq!(result.revision, 0);
            assert_eq!(std::fs::read(files.0.join("catalog.json")).unwrap(), old);
            assert!(matches!(
                apply(&mut store, request(&handle, 2, 0, create("Retry"))).outcome,
                Outcome::Durable { changed: true }
            ));
        }
    }
}

#[test]
fn worker_slot_stale_revision_cancel_and_close_do_not_drop_essential_captures() {
    let files = Files::new();
    let original = store(&files);
    let path = files.0.join("catalog.json");
    let track = original.catalog.tracks[0].clone();
    drop(original);
    let (entered, waiting) = mpsc::sync_channel(1);
    let (resume, release) = mpsc::sync_channel(1);
    let pause = Arc::new(AtomicBool::new(false));
    let hook = pause.clone();
    let mut metadata = Metadata::with_hook(path.clone(), move || {
        if hook.swap(false, Ordering::AcqRel) {
            entered.send(()).unwrap();
            release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    });
    let handle = Handle::default();
    metadata.set_performance(handle.clone());
    let mut rows = Arc::new(Vec::new());
    settle(&mut metadata, &mut rows);
    let token = metadata.edit_crates(0, create("Cancelled")).unwrap();
    assert_eq!(
        metadata
            .edit_crates(0, create("Must not replace"))
            .unwrap_err(),
        Admission::Busy
    );
    pause.store(true, Ordering::Release);
    metadata.poll(&mut rows).unwrap();
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut prep = track.versions[0].preparation;
    prep.cue = 0.75;
    metadata.capture(Capture {
        source: track.source.clone(),
        fingerprint: track.versions[0].fingerprint,
        metadata: track.versions[0].metadata.clone(),
        preparation: Some(prep),
        played: None,
    });
    metadata.set_collections_closing(true);
    assert_eq!(
        metadata.edit_crates(0, create("Closing")).unwrap_err(),
        Admission::Closing
    );
    assert!(token.cancel());
    resume.send(()).unwrap();
    settle(&mut metadata, &mut rows);
    assert!(matches!(
        metadata.take_collection_result().unwrap().outcome,
        Outcome::Rejected(Failure::Cancelled)
    ));
    assert!(metadata.catalog.crates.nodes().is_empty());
    assert_eq!(
        library::read(&path).unwrap().tracks[0].versions[0]
            .preparation
            .cue,
        0.75
    );
    metadata.set_collections_closing(false);
    let saved = metadata.edit_crates(0, create("Retained receipt")).unwrap();
    settle(&mut metadata, &mut rows);
    assert_eq!(
        metadata
            .edit_crates(1, create("Terminal not consumed"))
            .unwrap_err(),
        Admission::Busy
    );
    assert_eq!(metadata.take_collection_result().unwrap().id, saved.id);
    metadata
        .edit_crates(0, create("Stale expected revision"))
        .unwrap();
    settle(&mut metadata, &mut rows);
    assert!(matches!(
        metadata.take_collection_result().unwrap().outcome,
        Outcome::Rejected(Failure::Invalid(_))
    ));
    handle.set_enabled(true).unwrap();
    assert!(matches!(
        metadata.edit_crates(1, create("Protected")),
        Err(Admission::Performance(performance::Error::Protected))
    ));
    assert!(matches!(
        submit(&mut metadata, &mut rows, Action::Read).outcome,
        Outcome::Read
    ));
    assert!(metadata.durable);
    assert_eq!(
        metadata.label(),
        "DJ library saved",
        "a successful explicit read must retire the prior request error"
    );
}

#[test]
fn unavailable_saturated_invalid_and_lost_writer_results_are_explicit() {
    let mut session = Metadata::default();
    assert_eq!(
        session
            .edit_crates(0, create("No persistence"))
            .unwrap_err(),
        Admission::Unavailable
    );
    let files = Files::new();
    let original = store(&files);
    let path = files.0.join("catalog.json");
    drop(original);
    let mut metadata = Metadata::new(Some(path.clone()));
    let mut rows = Arc::new(Vec::new());
    settle(&mut metadata, &mut rows);
    assert!(matches!(
        metadata.edit_crates(0, create(" ")),
        Err(Admission::Invalid(_))
    ));
    let many = vec![TrackId("f".repeat(32)); MAX_SELECTION + 1];
    assert!(matches!(
        metadata.edit_crates(
            0,
            Action::Edit(Edit::AddMembers {
                id: CrateId("1".repeat(32)),
                members: many,
                before: None
            })
        ),
        Err(Admission::Invalid(_))
    ));
    let handle = Handle::default();
    metadata.set_performance(handle.clone());
    let permits: Vec<_> = (0..32).map(|_| handle.optional_work().unwrap()).collect();
    assert!(matches!(
        metadata.edit_crates(0, create("No work slots")),
        Err(Admission::Performance(performance::Error::BackgroundBusy))
    ));
    drop(permits);
    drop(metadata);
    let mut lost = Metadata::with_hook(files.0.join("lost.json"), || {
        panic!("controlled metadata worker loss")
    });
    lost.edit_crates(0, create("Lost")).unwrap();
    wait(|| {
        let _ = lost.poll(&mut rows);
        lost.worker_closed
    });
    assert_eq!(
        lost.take_collection_result().unwrap().outcome,
        Outcome::Rejected(Failure::Unavailable)
    );
    assert!(!lost.active());
    // After a publication claim, a vanished worker must not manufacture a
    // rejection. The caller receives an unknown result requiring disk re-open.
    let req = request(&Handle::default(), 77, 0, create("Claimed"));
    req.token.claim().unwrap();
    assert!(matches!(
        Receipt::unavailable(&req.token, 0).outcome,
        Outcome::Unknown(_)
    ));
}

#[test]
fn lost_terminal_after_actual_commit_never_manufactures_rejection() {
    let files = Files::new();
    let mut store = store(&files);
    let handle = Handle::default();
    let request = request(&handle, 77, 0, create("Committed before owner loss"));
    let token = request.token.clone();
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        apply_using(
            &mut store,
            request,
            |phase| {
                if phase == 2 {
                    panic!("controlled owner loss after confirmed disk save");
                }
            },
            Store::save,
        )
    }));
    assert!(failure.is_err());
    assert!(
        !token.cancel(),
        "a late Cancel cannot undo the actual saved crate"
    );
    assert!(matches!(
        Receipt::unavailable(&token, 0).outcome,
        Outcome::Unknown(_)
    ));
    let disk = library::read(&files.0.join("catalog.json")).unwrap();
    assert_eq!(disk.crates.revision(), 1);
    assert_eq!(disk.crates.nodes()[0].name, "Committed before owner loss");
    assert_eq!(disk.crates, store.catalog.crates);
    handle.set_enabled(true).unwrap(); // unwinding released the commit guard
}

#[test]
fn actual_catalog_import_keeps_collection_transaction_and_essential_edits_on_rejection() {
    let files = Files::new();
    let original = store(&files);
    let path = files.0.join("catalog.json");
    drop(original);
    let (entered, waiting) = mpsc::sync_channel(1);
    let (resume, release) = mpsc::sync_channel(1);
    let pause = Arc::new(AtomicBool::new(false));
    let hook = pause.clone();
    let mut metadata = Metadata::with_hook(path.clone(), move || {
        if hook.swap(false, Ordering::AcqRel) {
            entered.send(()).unwrap();
            release.recv_timeout(Duration::from_secs(5)).unwrap();
        }
    });
    let handle = Handle::default();
    metadata.set_performance(handle.clone());
    let mut rows = Arc::new(Vec::new());
    settle(&mut metadata, &mut rows);
    let root = created(submit(&mut metadata, &mut rows, create("Local root")));
    let mut incoming = metadata.catalog.as_ref().clone();
    let child = CrateId("e".repeat(32));
    let track = incoming.tracks[0].id.clone();
    incoming
        .edit_crates(
            incoming.crates.revision(),
            &Edit::Create {
                id: child.clone(),
                name: "Imported child".into(),
                parent: Some(root.clone()),
                before: None,
            },
        )
        .unwrap();
    // Strict import does not alter an existing root's child list. A new
    // disjoint tree is supported; overlap/reparent conflicts remain explicit.
    let rejected = files.0.join("parent-conflict.json");
    std::fs::write(&rejected, serde_json::to_vec(&incoming).unwrap()).unwrap();
    assert!(metadata.import(rejected));
    settle(&mut metadata, &mut rows);
    assert!(metadata.label().contains("import rejected"));
    assert!(metadata.catalog.crates.node(&child).is_none());
    let mut incoming = library::Catalog::default();
    incoming.tracks = metadata.catalog.tracks.clone();
    // Decode roundtrip rebuilds indices before this independent tree import.
    let import = files.0.join("import.json");
    std::fs::write(&import, serde_json::to_vec(&incoming).unwrap()).unwrap();
    incoming = library::read(&import).unwrap();
    incoming
        .edit_crates(
            0,
            &Edit::Create {
                id: child.clone(),
                name: "Imported root".into(),
                parent: None,
                before: None,
            },
        )
        .unwrap();
    incoming
        .edit_crates(
            1,
            &Edit::AddMembers {
                id: child.clone(),
                members: vec![track],
                before: None,
            },
        )
        .unwrap();
    std::fs::write(&import, serde_json::to_vec(&incoming).unwrap()).unwrap();
    assert!(metadata.import(import.clone()));
    settle(&mut metadata, &mut rows);
    let forest = metadata.catalog.crates.clone();
    assert_eq!(forest.roots(), &[root, child.clone()]);
    assert_eq!(forest.node(&child).unwrap().members.len(), 1);
    assert!(metadata.import(import.clone()));
    settle(&mut metadata, &mut rows);
    assert_eq!(metadata.catalog.crates, forest);
    let current = metadata.catalog.tracks[0].clone();
    let mut prep = current.versions[0].preparation;
    prep.cue = 0.875;
    incoming
        .edit_crates(
            incoming.crates.revision(),
            &Edit::Rename {
                id: child.clone(),
                name: "Conflicting same identity".into(),
            },
        )
        .unwrap();
    std::fs::write(&import, serde_json::to_vec(&incoming).unwrap()).unwrap();
    metadata.capture(Capture {
        source: current.source.clone(),
        fingerprint: current.versions[0].fingerprint,
        metadata: current.versions[0].metadata.clone(),
        preparation: Some(prep),
        played: None,
    });
    assert!(metadata.import(import.clone()));
    settle(&mut metadata, &mut rows);
    assert_eq!(metadata.catalog.crates, forest);
    assert!(metadata.label().contains("import rejected"));
    assert_eq!(
        library::read(&path).unwrap().tracks[0].versions[0]
            .preparation
            .cue,
        0.875
    );
    // A queued import and its new forest are cancelled by protection while
    // existing accepted preparation still reaches disk on the same worker.
    let mut fresh = library::Catalog::default();
    let fresh_id = CrateId("d".repeat(32));
    fresh
        .edit_crates(
            0,
            &Edit::Create {
                id: fresh_id.clone(),
                name: "Protected import".into(),
                parent: None,
                before: None,
            },
        )
        .unwrap();
    std::fs::write(&import, serde_json::to_vec(&fresh).unwrap()).unwrap();
    assert!(metadata.import(import));
    pause.store(true, Ordering::Release);
    metadata.poll(&mut rows).unwrap();
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    handle.set_enabled(true).unwrap();
    resume.send(()).unwrap();
    settle(&mut metadata, &mut rows);
    assert_eq!(metadata.catalog.crates, forest);
    assert!(metadata.catalog.crates.node(&fresh_id).is_none());
    assert_eq!(library::read(&path).unwrap().crates, forest);
}

#[test]
fn prepared_queue_creation_is_one_atomic_save_with_ordered_stable_members() {
    let files = Files::new();let mut store = store(&files);let handle = Handle::default();
    let member = store.catalog.tracks[0].id.clone();
    let create = Action::CreatePrepared {name:"Requests".into(),members:vec![member.clone()]};
    create.validate().unwrap();
    let mut calls = 0;
    let result = apply_using(&mut store,request(&handle,1,0,create.clone()),|_|{},|candidate| {
        calls += 1;
        assert_eq!(candidate.catalog.crates.nodes().len(),1);
        assert_eq!(candidate.catalog.crates.nodes()[0].members,vec![member.clone()]);
        candidate.save()
    });
    assert_eq!(calls,1);
    let id = created(result);
    assert_eq!(library::read(&files.0.join("catalog.json")).unwrap().crates.node(&id).unwrap().members,vec![member.clone()]);
    let baseline = store.catalog.crates.clone();
    let current = store.catalog.crates.revision();
    let missing=Action::CreatePrepared {name:"Invalid".into(),members:vec![TrackId("f".repeat(32))]};
    let result=apply_using(&mut store,request(&handle,2,current,missing),|_|{},|_|panic!("Invalid membership must reject before persistence"));
    assert!(matches!(result.outcome,Outcome::Rejected(Failure::Invalid(_))));assert!(result.created.is_none());assert_eq!(store.catalog.crates,baseline);
    let result=apply_using(&mut store,request(&handle,3,current,Action::CreatePrepared {name:"Storage failure".into(),members:vec![member]}),|_|{},|candidate|candidate.save_for_test(|phase| if phase==0 {Err("Storage refused before replacement".into())} else {Ok(())}));
    assert!(matches!(result.outcome,Outcome::Rejected(Failure::Storage(_))),"{:?}",result.outcome);assert!(result.created.is_none());assert_eq!(store.catalog.crates,baseline);
}
