use super::super::{apply, apply_using as apply_collection};
use super::*;
use crate::{
    engine::{
        media_analysis::tests::{wav, Files},
        media_source::{FileFingerprint, LibSource},
        performance::Handle,
    },
    library::{Metadata as Stored, Track},
    ui::{
        bpm::{Bpm, Origin},
        library_metadata::Metadata,
        LibItem,
    },
};
use std::{
    fs,
    sync::atomic::AtomicU8,
    time::{Duration, Instant},
};

fn fixture(files: &Files) -> Store {
    let mut store = Store::open(files.0.join("catalog.json")).unwrap();
    for index in 0..3 {
        let proof = files.source(&format!("track{index}.wav"), &wav(8000, 8000, 1, false));
        let LibSource::File(path) = &proof.source else {
            unreachable!()
        };
        let hash = crate::library::hash_project_source(path, proof.fingerprint, || true).unwrap();
        let version = store
            .catalog
            .upsert(
                proof.source,
                Some(proof.fingerprint),
                Stored {
                    title: format!("Track {index}"),
                    artist: "Saved artist".into(),
                    bpm: Bpm::new(128.0, Origin::User),
                    key: "Am".into(),
                    duration: Some(1.0),
                    last_play: None,
                },
            )
            .unwrap();
        version.preparation.cue = index as f64 / 10.0;
        version.preparation.hotcues[7] = Some(0.7);
        version.content_hash = Some(hash);
    }
    store.save().unwrap();
    store
}
fn selected(catalog: &Catalog, indices: &[usize]) -> Selection {
    Selection {
        catalog: Arc::new(catalog.clone()),
        ids: indices
            .iter()
            .map(|&i| catalog.tracks[i].id.clone())
            .collect(),
    }
}
fn request(store: &Store, handle: &Handle, action: Action) -> Request {
    let work = handle.optional_work().unwrap();
    Request {
        token: Token {
            id: 1,
            state: Arc::new(AtomicU8::new(PENDING)),
            cancel: Some(work.cancel()),
        },
        expected: store.catalog.crates.revision(),
        action: super::super::Action::Files(action),
        work: Some(work),
    }
}
fn path(track: &Track) -> PathBuf {
    let LibSource::File(path) = &track.source else {
        unreachable!()
    };
    path.clone()
}
fn wait(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "File catalog owner did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn settle(owner: &mut Metadata, rows: &mut Arc<Vec<LibItem>>) {
    wait(|| {
        owner.poll(rows).unwrap();
        !owner.active()
    });
}
fn submit(owner: &mut Metadata, rows: &mut Arc<Vec<LibItem>>, action: Action) -> Receipt {
    let token = owner
        .edit_crates(
            owner.catalog.crates.revision(),
            super::super::Action::Files(action),
        )
        .unwrap();
    settle(owner, rows);
    let result = owner.take_collection_result().unwrap();
    assert_eq!(result.id, token.id);
    result
}

#[test]
fn real_single_catalog_owner_copies_merges_removes_and_reopens_without_recreating_old_references() {
    let files = Files::new();
    let original = fixture(&files);
    let baseline = original.catalog.tracks.clone();
    let bytes = baseline
        .iter()
        .map(|track| fs::read(path(track)).unwrap())
        .collect::<Vec<_>>();
    let store_path = original.path().to_path_buf();
    drop(original);
    let mut owner = Metadata::new(Some(store_path.clone()));
    let mut rows = Arc::new(Vec::new());
    settle(&mut owner, &mut rows);
    let selection = selected(&owner.catalog, &[0]);
    let duplicates = submit(&mut owner, &mut rows, Action::Duplicates(selection));
    assert_eq!(duplicates.outcome, Outcome::Read);
    assert_eq!(duplicates.files.unwrap().duplicates.len(), 2);
    let destination = files.0.join("copies");
    fs::create_dir(&destination).unwrap();
    let selection = selected(&owner.catalog, &[0]);
    let result = submit(
        &mut owner,
        &mut rows,
        Action::Transfer {
            selection,
            destination: destination.clone(),
            move_files: false,
        },
    );
    assert_eq!(result.outcome, Outcome::Durable { changed: true });
    let current = owner
        .catalog
        .tracks
        .iter()
        .find(|track| track.id == baseline[0].id)
        .unwrap();
    assert_eq!(path(current), destination.join("track0.wav"));
    assert_eq!(
        current.versions[current.current].preparation,
        baseline[0].versions[baseline[0].current].preparation
    );
    assert!(rows.iter().any(|row| row.source == current.source));
    assert!(!rows.iter().any(|row| row.source == baseline[0].source));
    let reviewed = selected(&owner.catalog, &[0, 1]);
    let retained = baseline[0].id.clone();
    let preparation = baseline[1].id.clone();
    let result = submit(
        &mut owner,
        &mut rows,
        Action::Merge {
            selection: reviewed,
            retain: retained.clone(),
            preparation,
        },
    );
    assert_eq!(result.outcome, Outcome::Durable { changed: true });
    assert_eq!(owner.catalog.tracks.len(), 2);
    assert_eq!(rows.len(), 2);
    let selection = selected(&owner.catalog, &[0]);
    assert_eq!(
        submit(&mut owner, &mut rows, Action::Remove(selection)).outcome,
        Outcome::Durable { changed: true }
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(owner.catalog.tracks[0].id, baseline[2].id);
    settle(&mut owner, &mut rows);
    drop(owner);
    let mut reopened = None;
    wait(|| {
        reopened = Store::open(store_path.clone()).ok();
        reopened.is_some()
    });
    let reopened = reopened.unwrap();
    assert_eq!(reopened.catalog.tracks, vec![baseline[2].clone()]);
    for (track, bytes) in baseline.iter().zip(bytes) {
        assert_eq!(fs::read(path(track)).unwrap(), bytes);
    }
    assert!(destination.join("track0.wav").exists());
    println!(
        "LIBRARY_FILES_OWNER {}",
        serde_json::json!({"single_catalog_writer":true,"rows_republished":true,"stable_ids":true,"prepared_versions":true,"save_reopen":true,"reference_removal_keeps_audio":true,"physical_devices_opened":false})
    );
}

#[test]
fn journaled_move_and_explicit_restore_preserve_preparation_and_unselected_files_across_restart() {
    let files = Files::new();
    let mut store = fixture(&files);
    let before = store.catalog.tracks.clone();
    let destination = files.0.join("move");
    fs::create_dir(&destination).unwrap();
    let sentinel = destination.join("unselected.wav");
    fs::write(&sentinel, b"foreign sentinel").unwrap();
    let handle = Handle::default();
    let selection = selected(&store.catalog, &[0, 1]);
    let pending = request(
        &store,
        &handle,
        Action::Transfer {
            selection,
            destination: destination.clone(),
            move_files: true,
        },
    );
    let result = apply(&mut store, pending);
    assert_eq!(result.outcome, Outcome::Durable { changed: true });
    let recovery = result.files.unwrap().recovery.unwrap();
    assert_eq!(recovery.files, 2);
    assert!(!path(&before[0]).exists());
    assert!(!path(&before[1]).exists());
    assert!(destination.join("track0.wav").exists());
    assert_eq!(store.catalog.tracks[2], before[2]);
    let store_path = store.path().to_path_buf();
    drop(store);
    let mut store = Store::open(store_path.clone()).unwrap();
    assert_eq!(journal::pending(&store).unwrap().unwrap().id, recovery.id);
    let pending = request(&store, &handle, Action::Restore { id: recovery.id });
    let result = apply(&mut store, pending);
    assert_eq!(result.outcome, Outcome::Durable { changed: true });
    assert!(result.files.unwrap().recovery.is_none());
    assert!(!destination.join("track0.wav").exists());
    assert!(!destination.join("track1.wav").exists());
    assert_eq!(fs::read(sentinel).unwrap(), b"foreign sentinel");
    for original in &before {
        let track = store
            .catalog
            .tracks
            .iter()
            .find(|track| track.id == original.id)
            .unwrap();
        assert_eq!(track.source, original.source);
        assert_eq!(
            track.versions[track.current].preparation,
            original.versions[original.current].preparation
        );
        assert_eq!(fs::read(path(original)).unwrap(), wav(8000, 8000, 1, false));
    }
    let saved = store.catalog.tracks.clone();
    drop(store);
    assert_eq!(Store::open(store_path).unwrap().catalog.tracks, saved);
    println!(
        "LIBRARY_FILES_MOVE {}",
        serde_json::json!({"actual_filesystem":true,"journal_before_publication":true,"originals_quarantined":true,"explicit_restore":true,"reopened_recovery":true,"prepared_versions":true,"unselected_sentinel_untouched":true,"physical_devices_opened":false})
    );
}

#[test]
fn cancellation_failed_save_and_unconfirmed_replacement_keep_correct_media_ownership() {
    for mode in 0..3 {
        let files = Files::new();
        let mut store = fixture(&files);
        let before = store.catalog.tracks.clone();
        let destination = files.0.join("move");
        fs::create_dir(&destination).unwrap();
        let handle = Handle::default();
        let selection = selected(&store.catalog, &[0]);
        let pending = request(
            &store,
            &handle,
            Action::Transfer {
                selection,
                destination: destination.clone(),
                move_files: true,
            },
        );
        let token = pending.token.clone();
        let result = apply_collection(
            &mut store,
            pending,
            |phase| {
                if mode == 0 && phase == 0 {
                    assert!(token.cancel());
                }
            },
            |store| {
                store.save_for_test(|phase| {
                    if (mode == 1 && phase == 1) || (mode == 2 && phase == 3) {
                        Err(format!("injected save phase {phase}"))
                    } else {
                        Ok(())
                    }
                })
            },
        );
        assert!(path(&before[0]).exists());
        assert_eq!(
            fs::read(path(&before[0])).unwrap(),
            wav(8000, 8000, 1, false)
        );
        if mode < 2 {
            assert!(matches!(result.outcome, Outcome::Rejected(_)));
            assert_eq!(store.catalog.tracks, before);
            assert!(!destination.join("track0.wav").exists());
            assert!(journal::pending(&store).unwrap().is_none());
        } else {
            assert!(matches!(result.outcome, Outcome::CommittedUnconfirmed(_)));
            assert!(destination.join("track0.wav").exists());
            let id = journal::pending(&store).unwrap().unwrap().id;
            let store_path = store.path().to_path_buf();
            drop(store);
            let mut store = Store::open(store_path).unwrap();
            assert_eq!(
                path(&store.catalog.tracks[0]),
                destination.join("track0.wav")
            );
            let pending = request(&store, &handle, Action::Restore { id });
            assert!(matches!(
                apply(&mut store, pending).outcome,
                Outcome::Durable { changed: true }
            ));
            assert!(path(&before[0]).exists());
            assert!(!destination.join("track0.wav").exists());
        }
    }
    println!(
        "LIBRARY_FILES_SAVE_OWNERSHIP {}",
        serde_json::json!({"cancel_before_claim":true,"pre_replacement_rollback":true,"post_replacement_copies_retained":true,"originals_not_retired_on_uncertain_save":true,"physical_devices_opened":false})
    );
}

#[test]
fn interrupted_staging_partial_install_and_committed_move_are_recoverable_after_reopen() {
    for phase in 0..4 {
        let files = Files::new();
        let mut store = fixture(&files);
        let before = store.catalog.tracks.clone();
        let destination = files.0.join("move");
        fs::create_dir(&destination).unwrap();
        let cancel = AtomicBool::new(false);
        let targets = before[..2]
            .iter()
            .map(model::Target::capture)
            .collect::<Vec<_>>();
        let mut staged = transfer::stage(&store.catalog, &targets, &destination, &cancel).unwrap();
        let mut record = Journal::prepare(&store, &staged).unwrap();
        if phase == 1 {
            assert!(staged
                .install_for_test(&cancel, |index| if index == 1 {
                    Err("simulated interruption after first install".into())
                } else {
                    Ok(())
                })
                .is_err());
        }
        if phase >= 2 {
            let installed = staged.install(&cancel).unwrap();
            let candidate = store
                .catalog
                .adopt_file_copies(&installed, store.catalog.crates.revision(), &cancel)
                .unwrap();
            record.bind_candidate(&candidate).unwrap();
            if phase == 3 {
                store.catalog = candidate;
                store.save().unwrap();
                record.retire_originals(&store.catalog, &cancel).unwrap();
            }
        }
        staged.preserve_for_crash_test();
        drop(staged);
        let id = journal::pending(&store).unwrap().unwrap().id;
        let store_path = store.path().to_path_buf();
        drop(record);
        drop(store);
        let mut store = Store::open(store_path).unwrap();
        let handle = Handle::default();
        let pending = request(&store, &handle, Action::Restore { id });
        let result = apply(&mut store, pending);
        assert!(
            matches!(result.outcome, Outcome::Durable { .. }),
            "phase {phase}: {:?}",
            result.outcome
        );
        assert!(journal::pending(&store).unwrap().is_none());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
        for original in &before {
            assert!(path(original).exists());
            let track = store
                .catalog
                .tracks
                .iter()
                .find(|track| track.id == original.id)
                .unwrap();
            assert_eq!(track.source, original.source);
            assert_eq!(
                track.versions[track.current].preparation,
                original.versions[original.current].preparation
            );
        }
    }
    println!(
        "LIBRARY_FILES_INTERRUPTION {}",
        serde_json::json!({"staged_before_install":true,"partial_install":true,"bound_before_catalog_save":true,"committed_and_retired":true,"restart_and_explicit_recovery":true,"physical_devices_opened":false})
    );
}

#[test]
fn restoration_failed_save_retries_without_losing_either_copy_or_recovery_record() {
    let files = Files::new();
    let mut store = fixture(&files);
    let original = store.catalog.tracks[0].clone();
    let destination = files.0.join("move");
    fs::create_dir(&destination).unwrap();
    let handle = Handle::default();
    let selection = selected(&store.catalog, &[0]);
    let pending = request(
        &store,
        &handle,
        Action::Transfer {
            selection,
            destination: destination.clone(),
            move_files: true,
        },
    );
    let result = apply(&mut store, pending);
    let id = result.files.unwrap().recovery.unwrap().id;
    let moved = store.catalog.tracks.clone();
    let pending = request(&store, &handle, Action::Restore { id: id.clone() });
    let failed = apply_collection(
        &mut store,
        pending,
        |_| {},
        |store| {
            store.save_for_test(|phase| {
                if phase == 1 {
                    Err("injected restore save failure".into())
                } else {
                    Ok(())
                }
            })
        },
    );
    assert!(matches!(failed.outcome, Outcome::Rejected(_)));
    assert_eq!(store.catalog.tracks, moved);
    assert!(path(&original).exists());
    assert!(destination.join("track0.wav").exists());
    assert!(journal::pending(&store).unwrap().is_some());
    let pending = request(&store, &handle, Action::Restore { id });
    assert_eq!(
        apply(&mut store, pending).outcome,
        Outcome::Durable { changed: true }
    );
    assert_eq!(store.catalog.tracks[0].source, original.source);
    assert_eq!(
        store.catalog.tracks[0].versions[store.catalog.tracks[0].current].preparation,
        original.versions[original.current].preparation
    );
    assert!(!destination.join("track0.wav").exists());
    assert!(journal::pending(&store).unwrap().is_none());
}

#[test]
fn recovery_refuses_foreign_paths_changed_preparation_and_missing_restored_originals() {
    for mode in 0..3 {
        let files = Files::new();
        let mut store = fixture(&files);
        let original = store.catalog.tracks[0].clone();
        let destination = files.0.join("move");
        fs::create_dir(&destination).unwrap();
        let handle = Handle::default();
        let selection = selected(&store.catalog, &[0]);
        let pending = request(
            &store,
            &handle,
            Action::Transfer {
                selection,
                destination: destination.clone(),
                move_files: true,
            },
        );
        let result = apply(&mut store, pending);
        let id = result.files.unwrap().recovery.unwrap().id;
        if mode == 0 {
            fs::write(path(&original), b"foreign original replacement").unwrap();
        }
        if mode == 1 {
            let track = &mut store.catalog.tracks[0];
            track.versions[track.current].preparation.cue = 0.23;
            store.save().unwrap();
        }
        if mode == 2 {
            let mut record = Journal::reviewed(&store, &id).unwrap();
            store.catalog = record
                .restore_prepare(&store, &AtomicBool::new(false))
                .unwrap()
                .unwrap();
            store.save().unwrap();
            fs::remove_file(path(&original)).unwrap();
        }
        let catalog = serde_json::to_vec(&store.catalog).unwrap();
        let copy = fs::read(destination.join("track0.wav")).unwrap();
        let pending = request(&store, &handle, Action::Restore { id });
        let result = apply(&mut store, pending);
        assert!(matches!(result.outcome, Outcome::Rejected(_)));
        assert_eq!(serde_json::to_vec(&store.catalog).unwrap(), catalog);
        assert_eq!(fs::read(destination.join("track0.wav")).unwrap(), copy);
        assert!(journal::pending(&store).unwrap().is_some());
        if mode == 0 {
            assert_eq!(
                fs::read(path(&original)).unwrap(),
                b"foreign original replacement"
            );
        }
    }
    println!(
        "LIBRARY_FILES_RECOVERY_REFUSAL {}",
        serde_json::json!({"foreign_original_preserved":true,"changed_preparation_refused":true,"missing_restored_original_retains_last_copy":true,"physical_devices_opened":false})
    );
}

#[test]
fn retained_moves_allow_new_batches_and_reopen_independent_explicit_recovery_without_overwriting_archives(
) {
    let files = Files::new();
    let mut store = fixture(&files);
    let before = store.catalog.tracks.clone();
    let handle = Handle::default();
    let destination = files.0.join("kept");
    fs::create_dir(&destination).unwrap();
    let mut ids = Vec::new();
    for index in [0, 1] {
        let selection = selected(&store.catalog, &[index]);
        let pending = request(
            &store,
            &handle,
            Action::Transfer {
                selection,
                destination: destination.clone(),
                move_files: true,
            },
        );
        let receipt = apply(&mut store, pending);
        assert_eq!(receipt.outcome, Outcome::Durable { changed: true });
        let id = receipt.files.unwrap().recovery.unwrap().id;
        let pending = request(&store, &handle, Action::Keep { id: id.clone() });
        assert_eq!(
            apply(&mut store, pending).outcome,
            Outcome::Durable { changed: false }
        );
        assert!(journal::pending(&store).unwrap().is_none());
        assert!(!path(&before[index]).exists());
        ids.push(id);
    }
    let archives = journal::archived(&store).unwrap();
    assert_eq!(archives.len(), 2);
    assert!(archives.iter().all(|record| !record.active));
    let catalog = store.catalog.tracks.clone();
    let store_path = store.path().to_path_buf();
    drop(store);
    let mut store = Store::open(store_path).unwrap();
    assert_eq!(store.catalog.tracks, catalog);
    let foreign_record = store
        .path()
        .with_extension(format!("files-{}.json", ids[0]));
    let bytes = fs::read(&foreign_record).unwrap();
    let pending = request(&store, &handle, Action::Keep { id: ids[0].clone() });
    assert!(matches!(
        apply(&mut store, pending).outcome,
        Outcome::Unknown(_)
    ));
    assert_eq!(fs::read(&foreign_record).unwrap(), bytes);
    for id in ids {
        let pending = request(&store, &handle, Action::Restore { id });
        assert_eq!(
            apply(&mut store, pending).outcome,
            Outcome::Durable { changed: true }
        );
    }
    assert!(journal::archived(&store).unwrap().is_empty());
    for track in before {
        assert!(path(&track).exists());
        let current = store
            .catalog
            .tracks
            .iter()
            .find(|current| current.id == track.id)
            .unwrap();
        assert_eq!(current.source, track.source);
        assert_eq!(
            current.versions[current.current].preparation,
            track.versions[track.current].preparation
        );
    }
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    println!(
        "LIBRARY_FILES_RETAINED_MOVES {}",
        serde_json::json!({"new_batches_after_keep":true,"private_no_overwrite_records":true,"restart_inspection":true,"independent_restoration":true,"original_bytes_retained":true,"physical_devices_opened":false})
    );
}
