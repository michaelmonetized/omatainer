use super::*;
use crate::{
    engine::{media_analysis::tests::Files, performance::Handle},
    library::{self, tags::Patch, Metadata},
    ui::bpm::Bpm,
};
use std::{fs, sync::atomic::Ordering};

struct Fixture {
    store: Store,
    review: Review,
    files: Files,
    handle: Handle,
    work: Arc<WorkPermit>,
}
impl Fixture {
    fn new() -> Self {
        let files = Files::new();
        let reference = files.source(
            "tag-save.wav",
            include_bytes!("../../../../tests/fixtures/audio/tone-tags.wav"),
        );
        let mut store = Store::open(files.0.join("catalog.json")).unwrap();
        store
            .catalog
            .upsert(
                reference.source.clone(),
                Some(reference.fingerprint),
                Metadata {
                    title: "Filename title".into(),
                    artist: "Filename artist".into(),
                    bpm: Bpm::hint(96.0),
                    key: "Am".into(),
                    duration: Some(0.25),
                    last_play: None,
                },
            )
            .unwrap()
            .preparation
            .cue = 0.05;
        let review = Review {
            id: store.catalog.track(&reference.source).unwrap().id.clone(),
            source: reference.source,
            fingerprint: reference.fingerprint,
        };
        store.save().unwrap();
        let handle = Handle::default();
        let work = Arc::new(handle.optional_work().unwrap());
        Self {
            store,
            review,
            files,
            handle,
            work,
        }
    }
    fn media(&self) -> std::path::PathBuf {
        Location::resolve(&self.review.source).unwrap().path
    }
    fn patch() -> Patch {
        Patch {
            title: Some("夜の音".into()),
            artist: Some("".into()),
            bpm: Some("".into()),
            key: None,
        }
    }
    fn sidecar(&self, id: u64) -> Save {
        let location = Location::resolve(&self.review.source).unwrap();
        let observation =
            crate::media_tags::inspect(&location, self.review.fingerprint, &self.work.cancel());
        Save {
            id,
            result: Arc::new(Reply::Sidecar {
                target: self.review.clone(),
                patch: Self::patch(),
                observation,
                notice: "Media bytes unchanged".into(),
            }),
            recovery_index: None,
            work: self.work.clone(),
        }
    }
    fn applied(&self, id: u64) -> Save {
        let location = Location::resolve(&self.review.source).unwrap();
        let applied = write::apply(
            &location,
            &self.review,
            &Self::patch(),
            &self.files.0.join("recovery"),
            &self.work,
        )
        .unwrap();
        Save {
            id,
            result: Arc::new(Reply::Applied(applied)),
            recovery_index: None,
            work: self.work.clone(),
        }
    }
    fn current(&self) -> &crate::library::Version {
        let track = self.store.catalog.track(&self.review.source).unwrap();
        &track.versions[track.current]
    }
    fn disk_current(&self) -> crate::library::Version {
        let catalog = library::read(&self.files.0.join("catalog.json")).unwrap();
        let track = catalog.track(&self.review.source).unwrap();
        track.versions[track.current].clone()
    }
}

#[test]
fn sidecar_save_is_durable_preserves_media_and_cancelled_or_stale_reviews_cannot_publish() {
    let mut fixture = Fixture::new();
    let original = fs::read(fixture.media()).unwrap();
    let request = fixture.sidecar(42);
    let receipt = save(&mut fixture.store, request);
    assert_eq!(receipt.id, 42);
    assert!(receipt.committed && receipt.durable && receipt.outcome.is_ok());
    assert!(receipt.cleanup.is_none());
    assert_eq!(fs::read(fixture.media()).unwrap(), original);
    assert_eq!(fixture.current().metadata.title, "夜の音");
    assert_eq!(fixture.current().metadata.artist, "");
    assert_eq!(fixture.current().metadata.bpm, Bpm::USER_CLEARED);
    assert_eq!(fixture.disk_current(), *fixture.current());
    let baseline = fixture.store.catalog.tracks.clone();
    let request = fixture.sidecar(43);
    fixture.handle.set_enabled(true).unwrap();
    let cancelled = save(&mut fixture.store, request);
    assert!(!cancelled.committed && !cancelled.durable && cancelled.outcome.is_err());
    assert_eq!(fixture.store.catalog.tracks, baseline);
    fixture.handle.set_enabled(false).unwrap();
    fixture.work = Arc::new(fixture.handle.optional_work().unwrap());
    let request = fixture.sidecar(44);
    fs::write(fixture.media(), b"external changed media").unwrap();
    let stale = save(&mut fixture.store, request);
    assert!(!stale.committed && stale.outcome.is_err());
    assert_eq!(fixture.store.catalog.tracks, baseline);
}

#[test]
fn sidecar_precommit_failure_restores_essential_state_and_late_cancel_does_not_undo_commit() {
    for failure in [0, 2] {
        let mut fixture = Fixture::new();
        fixture.store.catalog.update_preparation(
            &fixture.review.source,
            Some(fixture.review.fingerprint),
            Some(crate::engine::preparation::Preparation {
                cue: 0.125,
                ..Default::default()
            }),
            None,
        );
        fixture.store.save().unwrap();
        let essential = fixture.store.catalog.tracks.clone();
        let request = fixture.sidecar(1);
        let receipt = save_using(
            &mut fixture.store,
            request,
            |_| {},
            |store| {
                store.save_for_test(|point| {
                    if point == failure {
                        Err("injected catalog failure".into())
                    } else {
                        Ok(())
                    }
                })
            },
        );
        assert!(!receipt.committed && !receipt.durable && receipt.outcome.is_err());
        assert!(receipt.cleanup.is_none());
        assert_eq!(fixture.store.catalog.tracks, essential);
        assert_eq!(fixture.disk_current().preparation.cue, 0.125);
        // An unrelated essential autosave must not accidentally publish the rejected sidecar.
        fixture.store.save().unwrap();
        assert_eq!(fixture.disk_current().metadata.title, "Filename title");
    }
    let mut fixture = Fixture::new();
    let request = fixture.sidecar(2);
    let work = fixture.work.clone();
    let receipt = save_using(
        &mut fixture.store,
        request,
        |point| {
            if point == 1 {
                work.cancel().store(true, Ordering::Release);
            }
        },
        Store::save,
    );
    assert!(receipt.committed && receipt.durable && receipt.outcome.is_ok());
    assert_eq!(fixture.disk_current().metadata.title, "夜の音");
}

#[test]
fn sidecar_postrename_failure_retains_committed_catalog_and_reports_unconfirmed_durability() {
    let mut fixture = Fixture::new();
    let request = fixture.sidecar(1);
    let receipt = save_using(
        &mut fixture.store,
        request,
        |_| {},
        |store| {
            store.save_for_test(|point| {
                if point == 3 {
                    Err("injected directory sync failure".into())
                } else {
                    Ok(())
                }
            })
        },
    );
    assert!(receipt.committed && !receipt.durable && receipt.outcome.is_err());
    assert!(receipt.cleanup.is_none());
    assert_eq!(fixture.disk_current().metadata.title, "夜の音");
    assert_eq!(fixture.current().metadata.title, "夜の音");
}

#[test]
fn installed_media_is_essential_after_protection_and_idempotent_recovery_preserves_later_sidecars()
{
    let mut fixture = Fixture::new();
    let request = fixture.applied(1);
    let Reply::Applied(applied) = request.result.as_ref() else {
        unreachable!()
    };
    let record = applied.record.clone();
    let fingerprint = applied.proof.new_fingerprint;
    fixture.handle.set_enabled(true).unwrap();
    assert!(fixture.work.cancelled());
    let receipt = save(&mut fixture.store, request);
    assert!(receipt.committed && receipt.durable && receipt.outcome.is_ok());
    assert_eq!(
        receipt.cleanup.unwrap().journal_path(),
        record.journal_path()
    );
    assert_eq!(fixture.current().fingerprint, Some(fingerprint));
    assert_eq!(fixture.current().preparation.cue, 0.05);
    let current = Review {
        fingerprint,
        ..fixture.review.clone()
    };
    fixture
        .store
        .catalog
        .apply_tag_sidecar(
            &current,
            &Patch {
                title: Some("Later user edit".into()),
                ..Default::default()
            },
        )
        .unwrap();
    fixture.store.save().unwrap();
    let recovered = write::recover(&fixture.files.0.join("recovery"));
    assert!(matches!(
        recovered.as_slice(),
        [write::Recovery::Applied(_)]
    ));
    let request = Save {
        id: 2,
        result: Arc::new(Reply::Recover(recovered)),
        recovery_index: Some(0),
        work: fixture.work.clone(),
    };
    let receipt = save(&mut fixture.store, request);
    assert!(receipt.committed && receipt.durable && receipt.outcome.is_ok());
    assert_eq!(fixture.current().metadata.title, "Later user edit");
    write::finalize(&receipt.cleanup.unwrap()).unwrap();
    assert!(!record.journal_path().exists());
}

#[test]
fn installed_media_catalog_failures_keep_recovery_and_retry_exact_installed_version() {
    for failure in [0, 2, 3] {
        let mut fixture = Fixture::new();
        let request = fixture.applied(1);
        let Reply::Applied(applied) = request.result.as_ref() else {
            unreachable!()
        };
        let journal = applied.record.journal_path();
        let new_fingerprint = applied.proof.new_fingerprint;
        let result = request.result.clone();
        let receipt = save_using(
            &mut fixture.store,
            request,
            |_| {},
            |store| {
                store.save_for_test(|point| {
                    if point == failure {
                        Err("injected tag catalog failure".into())
                    } else {
                        Ok(())
                    }
                })
            },
        );
        assert_eq!(receipt.committed, failure == 3);
        assert!(!receipt.durable && receipt.cleanup.is_none());
        assert!(receipt.outcome.unwrap_err().contains(if failure == 3 {
            "durability unconfirmed"
        } else {
            "library update pending"
        }));
        assert!(journal.exists());
        assert_eq!(
            FileFingerprint::read(&fixture.media()),
            Some(new_fingerprint)
        );
        assert_eq!(
            fixture.current().fingerprint,
            Some(if failure == 3 {
                new_fingerprint
            } else {
                fixture.review.fingerprint
            })
        );
        let retry = Save {
            id: 2,
            result,
            recovery_index: None,
            work: fixture.work.clone(),
        };
        let receipt = save(&mut fixture.store, retry);
        assert!(receipt.committed && receipt.durable && receipt.outcome.is_ok());
        assert_eq!(fixture.current().fingerprint, Some(new_fingerprint));
        write::finalize(&receipt.cleanup.unwrap()).unwrap();
        assert!(!journal.exists());
    }
}

#[test]
fn invalid_recovery_selection_and_changed_installed_path_cannot_publish_or_retire_journal() {
    let mut fixture = Fixture::new();
    let invalid = Save {
        id: 9,
        result: Arc::new(Reply::Recover(Vec::new())),
        recovery_index: Some(7),
        work: fixture.work.clone(),
    };
    let receipt = save(&mut fixture.store, invalid);
    assert!(!receipt.committed && receipt.cleanup.is_none());
    let request = fixture.applied(10);
    let Reply::Applied(applied) = request.result.as_ref() else {
        unreachable!()
    };
    let journal = applied.record.journal_path();
    let original = fixture.store.catalog.tracks.clone();
    fs::write(fixture.media(), b"externally replaced installed bytes").unwrap();
    let receipt = save(&mut fixture.store, request);
    assert!(!receipt.committed && !receipt.durable && receipt.cleanup.is_none());
    assert!(receipt
        .outcome
        .unwrap_err()
        .contains("library update pending"));
    assert!(journal.exists());
    assert_eq!(fixture.store.catalog.tracks, original);
}
