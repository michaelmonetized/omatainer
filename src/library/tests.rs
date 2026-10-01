use super::*;
use crate::engine::{media_source::BuiltinStem, preparation::Loop};
use std::sync::atomic::{AtomicU64, Ordering};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omatainer-library85-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> PathBuf {
        self.0.join("library.json")
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn metadata() -> Metadata {
    Metadata {
        title: "Prepared track".into(),
        artist: "Artist".into(),
        bpm: Bpm::new(127.5, Origin::User),
        key: "F#m".into(),
        duration: Some(300.25),
        last_play: Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1234567)),
    }
}
fn preparation() -> Preparation {
    Preparation {
        grid: None,
        cue: 4.5,
        hotcue_styles: [crate::engine::cue_metadata::Style::default(); 8],
        hotcues: [
            Some(0.0),
            Some(12.25),
            None,
            Some(99.5),
            None,
            None,
            None,
            Some(250.0),
        ],
        loop_region: Some(Loop {
            start: 16.0,
            length: 8.0,
            enabled: true,
        }),
    }
}
fn sources(dir: &Dir) -> Vec<LibSource> {
    vec![
        LibSource::File(dir.0.join("track.wav")),
        LibSource::Removable {
            volume_id: "volume-uuid".into(),
            relative_path: "music/track.wav".into(),
        },
        LibSource::Provider {
            provider: "catalog-a".into(),
            media_id: "track.wav".into(),
        },
        LibSource::Builtin(BuiltinStem::Drums),
    ]
}
fn mixed(dir: &Dir) -> Catalog {
    let mut catalog = Catalog::default();
    for source in sources(dir) {
        catalog
            .upsert(source, None, metadata())
            .unwrap()
            .preparation = preparation();
    }
    catalog
}
#[test]
fn mixed_namespaces_ids_metadata_and_preparation_survive_restart() {
    let dir = Dir::new();
    let original = mixed(&dir);
    {
        let mut store = Store::open(dir.store()).unwrap();
        store.catalog = original.clone();
        store.save().unwrap();
    }
    let store = Store::open(dir.store()).unwrap();
    assert_eq!(store.catalog.tracks, original.tracks);
    assert_eq!(store.catalog.index.len(), 4);
    assert_eq!(
        store
            .catalog
            .tracks
            .iter()
            .map(|t| &t.id)
            .collect::<HashSet<_>>()
            .len(),
        4
    );
}
#[test]
fn migration_preserves_every_v1_field_and_backs_up_original() {
    let dir = Dir::new();
    let original = mixed(&dir);
    let old = serde_json::json!({"schema": 1, "tracks": original.tracks.iter().map(|t| serde_json::json!({
        "id": t.id, "source": t.source, "fingerprint": t.versions[0].fingerprint,
        "metadata": t.versions[0].metadata, "preparation": t.versions[0].preparation,
    })).collect::<Vec<_>>()});
    let bytes = serde_json::to_vec(&old).unwrap();
    fs::write(dir.store(), &bytes).unwrap();
    let mut store = Store::open(dir.store()).unwrap();
    assert_eq!(store.catalog.tracks, original.tracks);
    store.save().unwrap();
    assert_eq!(
        fs::read(dir.store().with_extension("backup.json")).unwrap(),
        bytes
    );
    assert_eq!(read(&dir.store()).unwrap().tracks, original.tracks);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(dir.store()).unwrap()).unwrap()
            ["schema"],
        4
    );
}
#[test]
fn interrupted_atomic_updates_retain_complete_old_or_new_and_prepared_backup() {
    for checkpoint in 0..4 {
        let dir = Dir::new();
        let original = mixed(&dir);
        let mut store = Store::open(dir.store()).unwrap();
        store.catalog = original.clone();
        store.save().unwrap();
        store.catalog.tracks[0].versions[0].preparation.cue = 22.0;
        assert!(store
            .save_with(|at| if at == checkpoint {
                Err("simulated interruption".into())
            } else {
                Ok(())
            })
            .is_err());
        drop(store);
        let reopened = Store::open(dir.store()).unwrap();
        let expected = if checkpoint < 3 { 4.5 } else { 22.0 };
        assert_eq!(
            reopened.catalog.tracks[0].versions[0].preparation.cue,
            expected
        );
        assert_eq!(reopened.catalog.tracks[0].id, original.tracks[0].id);
        if checkpoint >= 2 {
            assert_eq!(
                read(&dir.store().with_extension("backup.json"))
                    .unwrap()
                    .tracks,
                original.tracks
            );
        }
        // An orphan from another process is neither loaded nor deleted.
        let orphan = dir.0.join("library.tmp-crashed");
        fs::write(&orphan, b"partial").unwrap();
        assert_eq!(read(&dir.store()).unwrap().tracks, reopened.catalog.tracks);
        assert_eq!(fs::read(orphan).unwrap(), b"partial");
    }
}
#[test]
fn malformed_future_unknown_fields_and_invalid_imports_fail_closed() {
    let dir = Dir::new();
    let good = mixed(&dir);
    let base = serde_json::to_value(&good).unwrap();
    let mut future = base.clone();
    future["schema"] = 99.into();
    let mut unknown = base.clone();
    unknown["tracks"][0]["versions"][0]["beat_grid"] = serde_json::json!({"future": true});
    let mut invalid = base.clone();
    invalid["tracks"][0]["versions"][0]["preparation"]["cue"] = (-1).into();
    for bytes in [
        b"{broken".to_vec(),
        serde_json::to_vec(&future).unwrap(),
        serde_json::to_vec(&unknown).unwrap(),
        serde_json::to_vec(&invalid).unwrap(),
    ] {
        fs::write(dir.store(), &bytes).unwrap();
        assert!(Store::open(dir.store()).is_err());
        assert_eq!(fs::read(dir.store()).unwrap(), bytes);
    }
    let mut catalog = good.clone();
    let mut conflict = good.clone();
    conflict.tracks[0].versions[0].preparation.cue = 30.0;
    assert!(catalog.merge_import(conflict).is_err());
    assert_eq!(catalog.tracks, good.tracks);
}
#[test]
fn replaced_bytes_archive_old_preparation_without_inheritance_or_identity_change() {
    let dir = Dir::new();
    let path = dir.0.join("same.wav");
    fs::write(&path, b"first").unwrap();
    let old = FileFingerprint::read(&path);
    let mut store = Store::open(dir.store()).unwrap();
    store
        .catalog
        .upsert(LibSource::File(path.clone()), old, metadata())
        .unwrap()
        .preparation = preparation();
    store.save().unwrap();
    let id = store.catalog.tracks[0].id.clone();
    fs::write(&path, b"a different recording").unwrap();
    let new = FileFingerprint::read(&path);
    assert_ne!(old, new);
    let next = store
        .catalog
        .upsert(
            LibSource::File(path.clone()),
            new,
            Metadata {
                bpm: Bpm::UNKNOWN,
                duration: None,
                last_play: None,
                ..metadata()
            },
        )
        .unwrap();
    assert_eq!(next.preparation, Preparation::default());
    assert_eq!(next.metadata.duration, None);
    assert_eq!(next.metadata.last_play, None);
    store.save().unwrap();
    drop(store);
    let store = Store::open(dir.store()).unwrap();
    assert_eq!(store.catalog.tracks[0].id, id);
    assert_eq!(
        store
            .catalog
            .version(&LibSource::File(path), old)
            .unwrap()
            .preparation,
        preparation()
    );
    assert_eq!(store.catalog.tracks[0].versions.len(), 2);
}
#[test]
fn writer_exclusion_external_replacement_and_namespace_validation_preserve_store() {
    let dir = Dir::new();
    let mut store = Store::open(dir.store()).unwrap();
    store.catalog = mixed(&dir);
    store.save().unwrap();
    assert!(Store::open(dir.store()).is_err());
    let external = b"{\"schema\":999}";
    fs::write(dir.store(), external).unwrap();
    store.catalog.tracks[0].versions[0].preparation.cue = 90.0;
    assert!(store.save().unwrap_err().contains("outside"));
    assert_eq!(fs::read(dir.store()).unwrap(), external);
    for source in [
        LibSource::File("relative.wav".into()),
        LibSource::Removable {
            volume_id: "disk".into(),
            relative_path: "../escape.wav".into(),
        },
        LibSource::Provider {
            provider: String::new(),
            media_id: "x".into(),
        },
    ] {
        assert!(Catalog::default().upsert(source, None, metadata()).is_err());
    }
}

#[test]
fn crash_writer_child() {
    let Some(path) = std::env::var_os("OMATAINER_TEST85_CRASH_PATH") else {
        return;
    };
    let checkpoint: u8 = std::env::var("OMATAINER_TEST85_CHECKPOINT")
        .unwrap()
        .parse()
        .unwrap();
    let mut store = Store::open(PathBuf::from(path)).unwrap();
    store.catalog.tracks[0].versions[0].preparation.cue = 77.0;
    store
        .save_with(|at| {
            if at == checkpoint {
                let signal = std::env::var_os("OMATAINER_TEST85_SIGNAL").unwrap();
                File::create(signal).unwrap().sync_all().unwrap();
                loop {
                    std::thread::park();
                }
            }
            Ok(())
        })
        .unwrap();
}

#[test]
fn killed_writer_recovers_complete_catalog_and_preparation_at_each_commit_stage() {
    for checkpoint in 0..4 {
        let dir = Dir::new();
        let original = mixed(&dir);
        {
            let mut store = Store::open(dir.store()).unwrap();
            store.catalog = original.clone();
            store.save().unwrap();
        }
        let signal = dir.0.join("checkpoint");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "library::tests::crash_writer_child",
                "--nocapture",
            ])
            .env("OMATAINER_TEST85_CRASH_PATH", dir.store())
            .env("OMATAINER_TEST85_CHECKPOINT", checkpoint.to_string())
            .env("OMATAINER_TEST85_SIGNAL", &signal)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !signal.exists()
            && std::time::Instant::now() < deadline
            && child.try_wait().unwrap().is_none()
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let reached = signal.exists();
        let _ = child.kill();
        let status = child.wait().unwrap();
        assert!(
            reached,
            "child failed before checkpoint {checkpoint}: {status}"
        );
        assert!(!status.success());
        let mut reopened = Store::open(dir.store()).unwrap();
        assert_eq!(reopened.catalog.tracks[0].id, original.tracks[0].id);
        assert_eq!(
            reopened.catalog.tracks[0].versions[0].preparation.cue,
            if checkpoint == 3 { 77.0 } else { 4.5 }
        );
        for (track, old) in reopened
            .catalog
            .tracks
            .iter()
            .skip(1)
            .zip(original.tracks.iter().skip(1))
        {
            assert_eq!(track, old);
        }
        if checkpoint >= 2 {
            assert_eq!(
                read(&dir.store().with_extension("backup.json"))
                    .unwrap()
                    .tracks,
                original.tracks
            );
        }
        // A new writer can continue, and never parses a crashed temporary file.
        reopened.save().unwrap();
    }
}

#[test]
fn missing_primary_with_existing_backup_is_not_initialized_over_prepared_data() {
    let dir = Dir::new();
    let backup = dir.store().with_extension("backup.json");
    let original = serde_json::to_vec(&mixed(&dir)).unwrap();
    fs::write(&backup, &original).unwrap();
    let error = Store::open(dir.store()).err().unwrap();
    assert!(error.contains("primary DJ library is missing"));
    assert!(!dir.store().exists());
    assert_eq!(fs::read(backup).unwrap(), original);
}

#[test]
fn symlink_store_or_external_symlink_swap_never_modifies_target_or_backup() {
    let dir = Dir::new();
    let outside = dir.0.join("outside.json");
    let bytes = serde_json::to_vec(&mixed(&dir)).unwrap();
    fs::write(&outside, &bytes).unwrap();
    std::os::unix::fs::symlink(&outside, dir.store()).unwrap();
    assert!(Store::open(dir.store()).is_err());
    assert_eq!(fs::read(&outside).unwrap(), bytes);
    fs::remove_file(dir.store()).unwrap();
    let mut store = Store::open(dir.store()).unwrap();
    store.catalog = mixed(&dir);
    store.save().unwrap();
    fs::remove_file(dir.store()).unwrap();
    std::os::unix::fs::symlink(&outside, dir.store()).unwrap();
    store.catalog.tracks[0].versions[0].preparation.cue = 80.0;
    assert!(store.save().is_err());
    assert!(fs::symlink_metadata(dir.store())
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(outside).unwrap(), bytes);
    assert!(!dir.store().with_extension("backup.json").exists());
}

#[test]
fn retry_after_backup_checkpoint_noop_rename_retires_owned_link_and_allows_later_saves() {
    let dir = Dir::new();
    let mut store = Store::open(dir.store()).unwrap();
    store.catalog = mixed(&dir);
    store.save().unwrap();
    store.catalog.tracks[0].versions[0].preparation.cue = 50.0;
    assert!(store
        .save_with(|at| if at == 2 {
            Err("stop after backup".into())
        } else {
            Ok(())
        })
        .is_err());
    // Primary and backup are now the same old inode. Retrying rename of another
    // link to that inode is a successful POSIX no-op, not removal of its source.
    store.save().unwrap();
    let temporary = dir
        .store()
        .with_extension(format!("backup-{}", std::process::id()));
    assert!(!temporary.exists());
    store.catalog.tracks[0].versions[0].preparation.cue = 60.0;
    store.save().unwrap();
    assert!(!temporary.exists());
    assert_eq!(
        read(&dir.store()).unwrap().tracks[0].versions[0]
            .preparation
            .cue,
        60.0
    );
    assert_eq!(
        read(&dir.store().with_extension("backup.json"))
            .unwrap()
            .tracks[0]
            .versions[0]
            .preparation
            .cue,
        50.0
    );
}

#[test]
fn eight_named_colored_cues_survive_restart_move_and_late_old_location_receipts() {
    use crate::engine::cue_metadata::{Name, Style};
    let dir = Dir::new();
    let original_path = dir.0.join("original.wav");
    let moved_path = dir.0.join("moved.wav");
    fs::write(&original_path, b"same track bytes, regardless of path").unwrap();
    let source = LibSource::File(original_path.clone());
    let old_fp = FileFingerprint::read(&original_path).unwrap();
    let mut store = Store::open(dir.store()).unwrap();
    let prepared = Preparation {
        hotcues: std::array::from_fn(|i| Some(i as f64 * 2.25)),
        hotcue_styles: std::array::from_fn(|i| Style {
            name: Name::new(&format!("Part {} • 演奏", i + 1)).unwrap(),
            color: Some([i as u8 * 30, 88, 220]),
        }),
        ..Preparation::default()
    };
    store.catalog.upsert(source.clone(), Some(old_fp), metadata()).unwrap().preparation = prepared;
    store.catalog.qualify_cues(&source, Some(old_fp));
    assert!(store.catalog.version(&source, Some(old_fp)).unwrap().content_hash.is_some());
    let id = store.catalog.track(&source).unwrap().id.clone();
    store.save().unwrap();
    drop(store);
    fs::rename(&original_path, &moved_path).unwrap();
    let mut restarted = Store::open(dir.store()).unwrap();
    assert_eq!(restarted.catalog.version(&source, Some(old_fp)).unwrap().preparation, prepared);
    restarted.catalog.relocate(&Relocate { id: id.clone(), source: source.clone(), fingerprint: old_fp,
        destination: moved_path.clone() }).unwrap();
    restarted.save().unwrap();
    drop(restarted);
    let mut restarted = Store::open(dir.store()).unwrap();
    let moved = LibSource::File(moved_path.clone());
    let moved_fp = FileFingerprint::read(&moved_path).unwrap();
    assert_eq!(restarted.catalog.tracks.len(), 1);
    assert_eq!(restarted.catalog.track(&moved).unwrap().id, id);
    assert_eq!(restarted.catalog.version(&moved, Some(moved_fp)).unwrap().preparation, prepared);
    assert_eq!(restarted.catalog.version(&source, Some(old_fp)).unwrap().preparation, prepared);
    // An old loaded/undo-retained receipt may finish after the move. It must
    // update the same identity and cannot recreate a duplicate old-path track.
    let mut late = prepared;
    late.hotcue_styles[7].name = Name::new("Last chorus").unwrap();
    restarted.catalog.upsert(source.clone(), Some(old_fp), metadata()).unwrap();
    restarted.catalog.update_preparation(&source, Some(old_fp), Some(late), None);
    assert_eq!(restarted.catalog.tracks.len(), 1);
    assert_eq!(restarted.catalog.version(&moved, Some(moved_fp)).unwrap().preparation, late);
    // Unrelated bytes newly occupying the old path get a distinct track, never
    // the relocated preparation. Old receipts still resolve by exact identity.
    fs::write(&original_path, b"unrelated replacement").unwrap();
    let replacement_fp = FileFingerprint::read(&original_path).unwrap();
    restarted.catalog.upsert(source.clone(), Some(replacement_fp), metadata()).unwrap();
    assert_eq!(restarted.catalog.tracks.len(), 2);
    assert_ne!(restarted.catalog.track(&source).unwrap().id, id);
    assert_eq!(restarted.catalog.version(&source, Some(replacement_fp)).unwrap().preparation, Preparation::default());
    restarted.catalog.update_preparation(&source, Some(old_fp), Some(prepared), None);
    assert_eq!(restarted.catalog.version(&moved, Some(moved_fp)).unwrap().preparation, prepared);
    restarted.save().unwrap();
    drop(restarted);
    let mut restored = Store::open(dir.store()).unwrap();
    assert_eq!(restored.catalog.track(&moved).unwrap().id, id);
    assert_eq!(restored.catalog.equivalent_current(&source, Some(old_fp)),
        Some((&moved, Some(moved_fp))));
    // Stable identity does not qualify later replacement bytes at the new path.
    fs::write(&moved_path, b"different bytes at relocated path").unwrap();
    let changed = FileFingerprint::read(&moved_path).unwrap();
    restored.catalog.upsert(moved.clone(), Some(changed), metadata()).unwrap();
    assert_eq!(restored.catalog.track(&moved).unwrap().id, id);
    assert!(restored.catalog.equivalent_current(&source, Some(old_fp)).is_none());
}

#[test]
fn relocation_rejects_unverified_missing_different_or_conflicting_tracks_without_changes() {
    let dir = Dir::new();
    let from = dir.0.join("before.wav");
    let to = dir.0.join("after.wav");
    fs::write(&from, b"original").unwrap();
    fs::write(&to, b"different").unwrap();
    let source = LibSource::File(from.clone());
    let fp = FileFingerprint::read(&from).unwrap();
    let mut catalog = Catalog::default();
    catalog.upsert(source.clone(), Some(fp), metadata()).unwrap().preparation = preparation();
    catalog.qualify_cues(&source, Some(fp));
    let request = Relocate { id: catalog.track(&source).unwrap().id.clone(), source: source.clone(),
        fingerprint: fp, destination: to.clone() };
    let unchanged = serde_json::to_value(&catalog).unwrap();
    assert!(catalog.relocate(&request).unwrap_err().contains("different bytes"));
    assert_eq!(serde_json::to_value(&catalog).unwrap(), unchanged);
    fs::write(&to, b"original").unwrap();
    catalog.upsert(LibSource::File(to.clone()), FileFingerprint::read(&to), metadata()).unwrap();
    let unchanged = serde_json::to_value(&catalog).unwrap();
    assert!(catalog.relocate(&request).unwrap_err().contains("already belongs"));
    assert_eq!(serde_json::to_value(&catalog).unwrap(), unchanged);
    let mut legacy = Catalog::default();
    legacy.upsert(source.clone(), Some(fp), metadata()).unwrap().preparation = preparation();
    let mut request = request;
    request.id = legacy.track(&source).unwrap().id.clone();
    fs::remove_file(&from).unwrap();
    let unchanged = serde_json::to_value(&legacy).unwrap();
    assert!(legacy.relocate(&request).unwrap_err().contains("not verified before the move"));
    assert_eq!(serde_json::to_value(&legacy).unwrap(), unchanged);
}

#[test]
fn content_hash_checks_exact_bytes_and_cancel_change_and_symlink_boundaries() {
    let dir = Dir::new();
    let path = dir.0.join("hash.wav");
    fs::write(&path, b"abc").unwrap();
    let fp = FileFingerprint::read(&path).unwrap();
    let hash = content::hash_file(&path, fp, || true).unwrap();
    assert_eq!(hash.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert!(content::hash_file(&path, fp, || false).unwrap_err().contains("cancelled"));
    let link = dir.0.join("symlink.wav");
    std::os::unix::fs::symlink(&path, &link).unwrap();
    assert!(content::hash_file(&link, fp, || true).is_err());
    let mut changed = false;
    assert!(content::hash_file(&path, fp, || {
        if !changed { fs::write(&path, b"different data").unwrap(); changed = true; }
        true
    }).is_err());
}

#[test]
fn schema_two_migrates_default_styles_and_no_invented_hash_without_overwriting_old_file() {
    let dir = Dir::new();
    let original = mixed(&dir);
    let mut old = serde_json::to_value(&original).unwrap();
    old["schema"] = 2.into();
    for track in old["tracks"].as_array_mut().unwrap() {
        track.as_object_mut().unwrap().remove("previous_locations");
        for version in track["versions"].as_array_mut().unwrap() {
            version.as_object_mut().unwrap().remove("content_hash");
            version["preparation"].as_object_mut().unwrap().remove("hotcue_styles");
            version["preparation"].as_object_mut().unwrap().remove("grid");
        }
    }
    let bytes = serde_json::to_vec(&old).unwrap();
    fs::write(dir.store(), &bytes).unwrap();
    let mut store = Store::open(dir.store()).unwrap();
    assert_eq!(store.catalog.tracks, original.tracks);
    assert_eq!(fs::read(dir.store()).unwrap(), bytes);
    store.save().unwrap();
    assert_eq!(fs::read(dir.store().with_extension("backup.json")).unwrap(), bytes);
    assert_eq!(read(&dir.store()).unwrap().tracks, original.tracks);
}

#[test]
fn manual_beatgrid_survives_reanalysis_restart_and_verified_relocation_with_cues_aligned() {
    use crate::engine::beatgrid::Grid;
    let dir = Dir::new();
    let path = dir.0.join("pickup-ambiguous-tempo.wav");
    let moved_path = dir.0.join("relocated-pickup.wav");
    fs::write(&path, b"immutable source with leading silence and a pickup").unwrap();
    let source = LibSource::File(path.clone());
    let fingerprint = FileFingerprint::read(&path).unwrap();
    let grid = Grid::new(1.25, 120.0).unwrap();
    let mut prepared = preparation();
    prepared.grid = Some(grid);
    prepared.hotcues = std::array::from_fn(|i| grid.seconds_at(i as f64 - 2.0));
    let mut initial = metadata();
    initial.bpm = Bpm::new(60.0, Origin::Heuristic);
    let mut store = Store::open(dir.store()).unwrap();
    store.catalog.upsert(source.clone(), Some(fingerprint), initial.clone()).unwrap().preparation = prepared;
    store.catalog.qualify_cues(&source, Some(fingerprint));
    let identity = store.catalog.track(&source).unwrap().id.clone();
    for suggestion in [240.0, 61.0, 119.5] {
        initial.bpm = Bpm::new(suggestion, Origin::Heuristic);
        let version = store.catalog.upsert(source.clone(), Some(fingerprint), initial.clone()).unwrap();
        assert_eq!(version.metadata.bpm.value(), Some(suggestion));
        assert_eq!(version.preparation.grid, Some(grid));
        assert_eq!(version.preparation.hotcues, prepared.hotcues);
    }
    store.save().unwrap();
    drop(store);
    fs::rename(&path, &moved_path).unwrap();
    let mut reopened = Store::open(dir.store()).unwrap();
    reopened.catalog.relocate(&Relocate { id: identity.clone(), source: source.clone(), fingerprint, destination: moved_path.clone() }).unwrap();
    reopened.save().unwrap();
    drop(reopened);
    let reopened = Store::open(dir.store()).unwrap();
    let source = LibSource::File(moved_path.clone());
    let version = reopened.catalog.version(&source, FileFingerprint::read(&moved_path)).unwrap();
    assert_eq!(reopened.catalog.track(&source).unwrap().id, identity);
    assert_eq!(version.preparation, prepared);
    for (i, seconds) in version.preparation.hotcues.into_iter().enumerate() {
        assert_eq!(version.preparation.grid.unwrap().beat_at(seconds.unwrap()), Some(i as f64 - 2.0));
    }
}

#[test]
fn schema_three_cue_metadata_migrates_without_an_invented_beatgrid() {
    let dir = Dir::new();
    let original = mixed(&dir);
    let mut old = serde_json::to_value(&original).unwrap();
    old["schema"] = 3.into();
    for track in old["tracks"].as_array_mut().unwrap() {
        for version in track["versions"].as_array_mut().unwrap() {
            version["preparation"].as_object_mut().unwrap().remove("grid");
        }
    }
    let bytes = serde_json::to_vec(&old).unwrap();
    fs::write(dir.store(), &bytes).unwrap();
    let mut store = Store::open(dir.store()).unwrap();
    assert_eq!(store.catalog.tracks, original.tracks);
    assert!(store.catalog.tracks.iter().all(|t| t.versions.iter().all(|v| v.preparation.grid.is_none())));
    assert_eq!(fs::read(dir.store()).unwrap(), bytes);
    store.save().unwrap();
    assert_eq!(fs::read(dir.store().with_extension("backup.json")).unwrap(), bytes);
    assert_eq!(read(&dir.store()).unwrap().schema, 4);
}
