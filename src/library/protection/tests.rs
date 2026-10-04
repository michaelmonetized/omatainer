use super::*;
use crate::engine::{beatgrid::Grid, media_source::BuiltinStem};
use std::sync::atomic::{AtomicUsize, Ordering};

fn metadata() -> Metadata {
    Metadata {
        title: "Prepared".into(),
        artist: "Artist".into(),
        key: "Am".into(),
        bpm: Bpm::new(120.0, Origin::Heuristic),
        duration: Some(10.0),
        last_play: None,
    }
}
fn catalog() -> Catalog {
    let mut c = Catalog::default();
    for stem in [BuiltinStem::Drums, BuiltinStem::Harmony] {
        c.upsert(LibSource::Builtin(stem), None, metadata())
            .unwrap();
    }
    c
}
fn all() -> Patch {
    Patch {
        grid: Some(true),
        bpm: Some(true),
        metadata: Some(true),
    }
}

#[test]
fn reviewed_locks_preserve_automatic_metadata_and_grid_but_allow_cue_and_history_capture() {
    let mut c = catalog();
    let source = c.tracks[0].source.clone();
    let grid = Grid::new(0.125, 123.0).unwrap();
    c.tracks[0].versions[0].preparation.grid = Some(grid);
    let target = Target::capture(&c.tracks[0]);
    c.protect(&[target.clone()], all()).unwrap();
    let mut automatic = metadata();
    automatic.bpm = Bpm::new(150.0, Origin::Heuristic);
    automatic.duration = Some(25.0);
    automatic.title = "".into();
    automatic.key = "".into();
    c.upsert(source.clone(), None, automatic).unwrap();
    assert_eq!(c.tracks[0].versions[0].metadata, target.metadata);
    let mut preparation = c.tracks[0].versions[0].preparation;
    preparation.grid = Some(Grid::new(2.0, 90.0).unwrap());
    preparation.cue = 3.5;
    preparation.hotcues[0] = Some(4.0);
    c.update_preparation(
        &source,
        None,
        Some(preparation),
        Some(SystemTime::UNIX_EPOCH),
    );
    assert_eq!(c.tracks[0].versions[0].preparation.grid, Some(grid));
    assert_eq!(c.tracks[0].versions[0].preparation.cue, 3.5);
    assert_eq!(c.tracks[0].versions[0].preparation.hotcues[0], Some(4.0));
    assert_eq!(
        c.tracks[0].versions[0].metadata.last_play,
        Some(SystemTime::UNIX_EPOCH)
    );
    let target = Target::capture(&c.tracks[0]);
    c.protect(
        &[target],
        Patch {
            grid: Some(false),
            bpm: Some(false),
            metadata: Some(false),
        },
    )
    .unwrap();
    c.update_preparation(&source, None, Some(preparation), None);
    assert_eq!(c.tracks[0].versions[0].preparation.grid, preparation.grid);
}

#[test]
fn batch_review_rejects_changed_versions_and_duplicate_targets_before_touching_any_lock() {
    let mut c = catalog();
    let targets: Vec<_> = c.tracks.iter().map(Target::capture).collect();
    c.tracks[1].versions[0].metadata.bpm = Bpm::new(130.0, Origin::User);
    assert!(c
        .protect(&targets, all())
        .unwrap_err()
        .contains("changed after review"));
    assert!(c.tracks.iter().all(|track| track.locks.is_empty()));
    assert!(c
        .protect(&[targets[0].clone(), targets[0].clone()], all())
        .is_err());
    assert!(c.tracks[0].locks.is_empty());
}

#[test]
fn protection_survives_catalog_reopen_and_cannot_be_hidden_in_schema_ten() {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "omatainer-protection-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("catalog.json");
    let mut store = Store::open(path.clone()).unwrap();
    store.catalog = catalog();
    let targets: Vec<_> = store.catalog.tracks.iter().map(Target::capture).collect();
    store.catalog.protect(&targets, all()).unwrap();
    store.save().unwrap();
    drop(store);
    let reopened = Store::open(path.clone()).unwrap();
    assert_eq!(reopened.catalog.schema, 11);
    assert!(reopened.catalog.tracks.iter().all(|track| track.locks
        == Locks {
            grid: true,
            bpm: true,
            metadata: true
        }));
    drop(reopened);
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["schema"] = 10.into();
    let bytes = serde_json::to_vec(&legacy).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    assert!(read(&path)
        .unwrap_err()
        .contains("require library schema 11"));
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    for track in legacy["tracks"].as_array_mut().unwrap() {
        track.as_object_mut().unwrap().remove("locks");
    }
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let migrated = read(&path).unwrap();
    assert_eq!(migrated.schema, 11);
    assert!(migrated.tracks.iter().all(|track| track.locks.is_empty()));
    std::fs::remove_dir_all(root).unwrap();
}
