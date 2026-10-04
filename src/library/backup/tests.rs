use super::*;
use crate::engine::{beatgrid::Grid, preparation::Loop};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omatainer-backup143-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn catalog(&self) -> Catalog {
        let mut catalog = Catalog::default();
        for name in ["a.wav", "b.wav"] {
            let path = self.0.join(name);
            fs::write(&path, vec![11u8; 200_000]).unwrap();
            let source = LibSource::File(path.clone());
            catalog
                .upsert(source.clone(), FileFingerprint::read(&path), fields())
                .unwrap();
            fs::write(&path, vec![12u8; 200_000]).unwrap();
            let version = catalog
                .upsert(source.clone(), FileFingerprint::read(&path), fields())
                .unwrap();
            version.preparation = Preparation {
                cue: 0.1,
                grid: Some(Grid::new(0.01, 120.0).unwrap()),
                hotcues: [Some(0.2); 8],
                loop_region: Some(Loop {
                    start: 0.2,
                    length: 0.2,
                    enabled: true,
                }),
                ..Default::default()
            };
            let index = catalog.index[&source];
            catalog.tracks[index].annotations.rating = 5;
            catalog.tracks[index].annotations.tags = vec!["prepared".into()];
            catalog.tracks[index].locks = protection::Locks {
                grid: true,
                bpm: true,
                metadata: true,
            };
        }
        let id = crates::CrateId("f".repeat(32));
        let known: HashSet<_> = catalog
            .tracks
            .iter()
            .map(|track| track.id.clone())
            .collect();
        for edit in [
            crates::Edit::Create {
                id: id.clone(),
                name: "Prepared set".into(),
                parent: None,
                before: None,
            },
            crates::Edit::AddMembers {
                id: id.clone(),
                members: catalog
                    .tracks
                    .iter()
                    .map(|track| track.id.clone())
                    .rev()
                    .collect(),
                before: None,
            },
            crates::Edit::SetFavorite { id, favorite: true },
        ] {
            catalog
                .crates
                .apply(catalog.crates.revision(), &edit, |id| known.contains(id))
                .unwrap();
        }
        catalog
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn fields() -> Metadata {
    Metadata {
        title: "Prepared".into(),
        artist: "Fixture".into(),
        bpm: Bpm::new(120.0, Origin::User),
        key: "Am".into(),
        duration: Some(1.0),
        last_play: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(123)),
    }
}
fn rewrite_manifest(path: &Path, mutate: impl FnOnce(&mut serde_json::Value)) {
    let file = path.join("manifest.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    mutate(&mut value);
    fs::write(file, serde_json::to_vec(&value).unwrap()).unwrap();
}
fn no_stages(files: &Files) {
    assert!(!fs::read_dir(&files.0).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".omatainer-portable-")));
}

#[test]
fn schema_twelve_backups_migrate_and_current_anchors_survive_collected_restore() {
    let files = Files::new();
    let cancel = AtomicBool::new(false);
    let original = files.catalog();
    let backup = files.0.join("legacy-12");
    export(&original, &backup, Some(400_000), &cancel, None, |_| {}).unwrap();
    let mut catalog: serde_json::Value = serde_json::from_slice(&fs::read(backup.join("catalog.json")).unwrap()).unwrap();
    catalog["schema"] = 12.into();
    let legacy = serde_json::to_vec(&catalog).unwrap(); fs::write(backup.join("catalog.json"), &legacy).unwrap();
    let digest: [u8; 32] = Sha256::digest(&legacy).into();
    rewrite_manifest(&backup, |manifest| {
        manifest["library_schema"] = 12.into(); manifest["catalog_sha256"] = serde_json::to_value(digest).unwrap();
    });
    assert_eq!(inspect(&backup, &cancel, |_| {}).unwrap().collected, 2);
    let restored = restore(&backup, &files.0.join("legacy-restored"), &cancel, None, |_| {}).unwrap();
    let result = read(&restored.catalog).unwrap();
    assert_eq!(result.schema, SCHEMA);
    assert_eq!(result.crates, original.crates);
    assert_eq!(fs::read(backup.join("catalog.json")).unwrap(), legacy);
    for (a, b) in result.tracks.iter().zip(&original.tracks) {
        assert_eq!(a.versions[a.current].preparation, b.versions[b.current].preparation);
    }
    rewrite_manifest(&backup, |manifest| manifest["library_schema"] = SCHEMA.into());
    assert!(inspect(&backup, &cancel, |_| {}).unwrap_err().contains("schema does not match"));
    let grid = Grid::new(0.01, 120.0).unwrap().with_anchor(1.0, 0.51).unwrap().with_anchor(2.0, 0.91).unwrap();
    let mut anchored = original.clone();
    for track in &mut anchored.tracks { track.versions[track.current].preparation.grid = Some(grid); }
    let backup = files.0.join("current-anchors");
    export(&anchored, &backup, Some(400_000), &cancel, None, |_| {}).unwrap();
    let restored = restore(&backup, &files.0.join("current-restored"), &cancel, None, |_| {}).unwrap();
    for track in read(&restored.catalog).unwrap().tracks {
        assert_eq!(track.versions[track.current].preparation.grid, Some(grid));
        assert_eq!(track.versions.len(), 3);
        assert_eq!(track.versions[1].preparation.grid, Some(grid));
    }
    no_stages(&files);
}

#[test]
fn metadata_snapshot_is_exact_and_optional_collection_preserves_every_prepared_version() {
    let files = Files::new();
    let catalog = files.catalog();
    let cancel = AtomicBool::new(false);
    let captured = serde_json::to_vec(&catalog).unwrap();
    let sources: Vec<_> = catalog
        .tracks
        .iter()
        .map(|track| {
            if let LibSource::File(path) = &track.source {
                (
                    path.clone(),
                    FileFingerprint::read(path).unwrap(),
                    fs::read(path).unwrap(),
                )
            } else {
                unreachable!()
            }
        })
        .collect();
    let metadata = files.0.join("metadata");
    export(&catalog, &metadata, None, &cancel, None, |_| {}).unwrap();
    assert_eq!(fs::read(metadata.join("catalog.json")).unwrap(), captured);
    assert_eq!(inspect(&metadata, &cancel, |_| {}).unwrap().collected, 0);
    let output = files.0.join("metadata-restored");
    let restored = restore(&metadata, &output, &cancel, None, |_| {}).unwrap();
    assert_eq!(
        serde_json::to_vec(&read(&restored.catalog).unwrap()).unwrap(),
        captured
    );
    let backup = files.0.join("collected");
    let result = export(&catalog, &backup, Some(400_000), &cancel, None, |_| {}).unwrap();
    assert_eq!(
        (result.summary.collected, result.summary.bytes),
        (2, 200_000)
    );
    assert_eq!(fs::read_dir(backup.join("media")).unwrap().count(), 1);
    for (path, fingerprint, bytes) in &sources {
        assert_eq!(FileFingerprint::read(path), Some(*fingerprint));
        assert_eq!(fs::read(path).unwrap(), *bytes);
    }
    assert_eq!(
        serde_json::to_vec(&catalog).unwrap(),
        captured,
        "captured publication was mutated"
    );
    for (path, _, _) in &sources {
        fs::remove_file(path).unwrap();
    }
    let destination = files.0.join("new-volume");
    let restored = restore(&backup, &destination, &cancel, None, |_| {}).unwrap();
    assert_eq!(restored.outcome, SaveOutcome::Durable);
    let result = read(&restored.catalog).unwrap();
    assert_eq!(result.crates, catalog.crates);
    assert_eq!(result.watched_roots, catalog.watched_roots);
    for (track, original) in result.tracks.iter().zip(&catalog.tracks) {
        assert_eq!(
            (&track.id, &track.annotations, track.locks),
            (&original.id, &original.annotations, original.locks)
        );
        let current = &track.versions[track.current];
        let old = &original.versions[original.current];
        assert_eq!(
            (
                &current.metadata,
                current.preparation,
                &current.analysis,
                &current.tags,
                &current.audio_identity
            ),
            (
                &old.metadata,
                old.preparation,
                &old.analysis,
                &old.tags,
                &old.audio_identity
            )
        );
        assert_eq!(track.versions.len(), original.versions.len() + 1);
        assert_eq!(track.versions[0], original.versions[0]);
        assert!(track.previous_locations.contains(&PreviousLocation {
            source: original.source.clone(),
            fingerprint: old.fingerprint.unwrap()
        }));
        let LibSource::File(path) = &track.source else {
            panic!()
        };
        assert!(path.starts_with(&destination));
        assert_eq!(FileFingerprint::read(path), current.fingerprint);
        assert_eq!(
            content::hash_file(path, current.fingerprint.unwrap(), || true).unwrap(),
            current.content_hash.unwrap()
        );
    }
    let mut empty = Catalog::default();
    empty.merge_import(result.clone()).unwrap();
    let before = serde_json::to_vec(&catalog).unwrap();
    let mut conflicting = catalog.clone();
    assert!(conflicting
        .merge_import(result)
        .unwrap_err()
        .contains("different location"));
    assert_eq!(serde_json::to_vec(&conflicting).unwrap(), before);
    no_stages(&files);
}

#[test]
fn collection_limits_cancellation_and_source_replacement_leave_no_published_result() {
    let files = Files::new();
    let catalog = files.catalog();
    for limit in [0, 100, MAX_COLLECTION + 1] {
        let destination = files.0.join(format!("limit-{limit}"));
        assert!(export(
            &catalog,
            &destination,
            Some(limit),
            &AtomicBool::new(false),
            None,
            |_| {}
        )
        .is_err());
        assert!(!destination.exists());
        no_stages(&files);
    }
    let cancel = AtomicBool::new(false);
    let destination = files.0.join("cancelled");
    let mut measured = false;
    assert!(export(
        &catalog,
        &destination,
        Some(400_000),
        &cancel,
        None,
        |progress| {
            if progress.bytes > 0 {
                measured = true;
                cancel.store(true, Ordering::Release);
            }
        }
    )
    .is_err());
    assert!(measured);
    assert!(!destination.exists());
    no_stages(&files);
    let source = files.0.join("a.wav");
    let destination = files.0.join("swapped");
    let mut changed = false;
    assert!(export(
        &catalog,
        &destination,
        Some(400_000),
        &AtomicBool::new(false),
        None,
        |progress| {
            if progress.bytes > 0 && !changed {
                fs::rename(&source, files.0.join("original.wav")).unwrap();
                fs::write(&source, vec![99u8; 200_000]).unwrap();
                changed = true;
            }
        }
    )
    .is_err());
    assert!(changed);
    assert!(!destination.exists());
    no_stages(&files);
}

#[test]
fn hostile_headers_names_payloads_and_directory_changes_are_rejected_read_only() {
    let files = Files::new();
    let catalog = files.catalog();
    let cancel = AtomicBool::new(false);
    for case in 0..9 {
        let backup = files.0.join(format!("backup-{case}"));
        export(&catalog, &backup, Some(400_000), &cancel, None, |_| {}).unwrap();
        match case {
            0 => rewrite_manifest(&backup, |value| value["schema"] = 2.into()),
            1 => rewrite_manifest(&backup, |value| value["unexpected"] = true.into()),
            2 => rewrite_manifest(&backup, |value| {
                value["entries"][0]["asset"] = "../../a.wav".into()
            }),
            3 => rewrite_manifest(&backup, |value| {
                let duplicate = value["entries"][0].clone();
                value["entries"].as_array_mut().unwrap().push(duplicate);
            }),
            4 => {
                fs::write(backup.join("catalog.json"), b"{}").unwrap();
            }
            5 => {
                let asset = fs::read_dir(backup.join("media"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                fs::write(asset, vec![9u8; 200_000]).unwrap();
            }
            6 => {
                let asset = fs::read_dir(backup.join("media"))
                    .unwrap()
                    .next()
                    .unwrap()
                    .unwrap()
                    .path();
                fs::remove_file(&asset).unwrap();
                std::os::unix::fs::symlink(files.0.join("a.wav"), asset).unwrap();
            }
            7 => {
                fs::write(backup.join("media/extra.wav"), b"extra").unwrap();
            }
            _ => rewrite_manifest(&backup, |value| {
                value["entries"][0]["bytes"] = 199_999.into()
            }),
        }
        let before = fs::read(backup.join("manifest.json")).unwrap();
        assert!(inspect(&backup, &cancel, |_| {}).is_err(), "case {case}");
        let destination = files.0.join(format!("restore-{case}"));
        assert!(
            restore(&backup, &destination, &cancel, None, |_| {}).is_err(),
            "case {case}"
        );
        assert!(!destination.exists());
        assert_eq!(fs::read(backup.join("manifest.json")).unwrap(), before);
        no_stages(&files);
    }
}

#[test]
fn restore_refuses_full_history_existing_destinations_and_publish_races_atomically() {
    let files = Files::new();
    let mut catalog = files.catalog();
    let cancel = AtomicBool::new(false);
    for track in &mut catalog.tracks {
        let fingerprint = track.versions[track.current].fingerprint.unwrap();
        track.previous_locations = (0..64)
            .map(|n| PreviousLocation {
                source: LibSource::File(files.0.join(format!("{}-old-{n}.wav", track.id.0))),
                fingerprint,
            })
            .collect();
    }
    let backup = files.0.join("full-history");
    export(&catalog, &backup, Some(400_000), &cancel, None, |_| {}).unwrap();
    let destination = files.0.join("full-history-restored");
    assert!(restore(&backup, &destination, &cancel, None, |_| {})
        .err()
        .unwrap()
        .contains("history is full"));
    assert!(!destination.exists());
    let existing = files.0.join("existing");
    fs::create_dir(&existing).unwrap();
    fs::write(existing.join("mine"), b"retained").unwrap();
    assert!(export(&catalog, &existing, None, &cancel, None, |_| {}).is_err());
    assert!(restore(&backup, &existing, &cancel, None, |_| {}).is_err());
    assert_eq!(fs::read(existing.join("mine")).unwrap(), b"retained");
    let destination = files.0.join("raced");
    let mut created = false;
    assert!(export(
        &catalog,
        &destination,
        Some(400_000),
        &cancel,
        None,
        |progress| {
            if progress.files == progress.total && progress.total > 0 && !created {
                fs::create_dir(&destination).unwrap();
                fs::write(destination.join("mine"), b"raced owner").unwrap();
                created = true;
            }
        }
    )
    .is_err());
    assert_eq!(fs::read(destination.join("mine")).unwrap(), b"raced owner");
    assert!(restore(&backup, &backup.join("nested"), &cancel, None, |_| {}).is_err());
    no_stages(&files);
}

#[test]
fn captured_snapshot_ignores_later_owner_edits_and_protection_cancels_real_copy() {
    let files = Files::new();
    let mut owner = std::sync::Arc::new(files.catalog());
    let captured = owner.clone();
    std::sync::Arc::make_mut(&mut owner).tracks[0]
        .annotations
        .rating = 1;
    let backup = files.0.join("fixed-snapshot");
    export(
        &captured,
        &backup,
        None,
        &AtomicBool::new(false),
        None,
        |_| {},
    )
    .unwrap();
    assert_eq!(
        read(&backup.join("catalog.json")).unwrap().tracks[0]
            .annotations
            .rating,
        5
    );
    let performance = crate::engine::performance::Handle::default();
    let work = performance.optional_work().unwrap();
    let cancel = work.cancel();
    let destination = files.0.join("protected");
    let mut entered = false;
    let result = export(
        &captured,
        &destination,
        Some(400_000),
        &cancel,
        Some(&work),
        |progress| {
            if progress.bytes > 0 && !entered {
                performance.set_enabled(true).unwrap();
                entered = true;
            }
        },
    );
    assert!(entered && result.is_err());
    assert!(!destination.exists());
    no_stages(&files);
}

#[test]
fn cancelled_restore_copy_and_replaced_private_output_leave_originals_unchanged() {
    let files = Files::new();
    let catalog = files.catalog();
    let cancel = AtomicBool::new(false);
    let backup = files.0.join("backup");
    export(&catalog, &backup, Some(400_000), &cancel, None, |_| {}).unwrap();
    let before = fs::read(backup.join("catalog.json")).unwrap();
    let destination = files.0.join("cancelled-restore");
    let mut verified = false;
    let mut copied = false;
    assert!(restore(&backup, &destination, &cancel, None, |progress| {
        if progress.files == progress.total && progress.total == 2 {
            verified = true;
        } else if verified && progress.files == 0 && progress.bytes > 0 {
            copied = true;
            cancel.store(true, Ordering::Release);
        }
    })
    .is_err());
    assert!(verified && copied);
    assert!(!destination.exists());
    assert_eq!(fs::read(backup.join("catalog.json")).unwrap(), before);
    no_stages(&files);
    let destination = files.0.join("swapped-output");
    let mut swapped = false;
    assert!(export(
        &catalog,
        &destination,
        Some(400_000),
        &AtomicBool::new(false),
        None,
        |progress| {
            if progress.bytes > 0 && !swapped {
                let stage = fs::read_dir(&files.0)
                    .unwrap()
                    .filter_map(|entry| entry.ok())
                    .find(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".omatainer-portable-")
                    })
                    .unwrap()
                    .path();
                let output = stage
                    .join("media")
                    .join(format!("{}.part", catalog.tracks[0].id.0));
                fs::remove_file(&output).unwrap();
                fs::write(output, vec![99u8; 200_000]).unwrap();
                swapped = true;
            }
        }
    )
    .is_err());
    assert!(swapped);
    assert!(!destination.exists());
    no_stages(&files);
}
