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
        cue: 4.5,
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
        2
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
