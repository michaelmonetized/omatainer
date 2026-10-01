use super::*;
use crate::engine::media_analysis::tests::{wav, Files};
use crate::ui::{bpm::Bpm, library_metadata::Metadata};
use std::time::{Duration, Instant};

fn fixture() -> (Files, Arc<Catalog>, Arc<Vec<LibItem>>) {
    let files = Files::new();
    let mut catalog = Catalog::default();
    for name in ["First.wav", "Second.wav"] {
        let proof = files.source(name, &wav(8000, 1024, 1, false));
        catalog.upsert(proof.source, Some(proof.fingerprint), crate::library::Metadata {
            title: name.into(), artist: "Fixture".into(), bpm: Bpm::hint(120.0),
            key: "—".into(), duration: None, last_play: None,
        }).unwrap();
    }
    let rows = Arc::new(catalog.tracks.iter().rev().map(|track|
        LibItem::from_stored(track.source.clone(), &track.versions[track.current])).collect());
    (files, Arc::new(catalog), rows)
}

#[test]
fn exact_publication_pair_and_current_version_are_required_for_row_lookup() {
    let (_files, catalog, rows) = fixture();
    let index = CollectionRows::build(&rows, &catalog);
    assert_eq!(index.row(&catalog.tracks[0].id, &rows, &catalog), Some(1));
    assert_eq!(index.track_index(&catalog.tracks[0].id, &catalog), Some(0));
    let copied_rows = Arc::new((*rows).clone());
    let copied_catalog = Arc::new((*catalog).clone());
    assert_eq!(index.row(&catalog.tracks[0].id, &copied_rows, &catalog), None);
    assert_eq!(index.row(&catalog.tracks[0].id, &rows, &copied_catalog), None);
    assert_eq!(index.track_index(&catalog.tracks[0].id, &copied_catalog), None);

    let mut unavailable = (*rows).clone();
    unavailable[1].fingerprint = None;
    let unavailable = Arc::new(unavailable);
    let index = CollectionRows::build(&unavailable, &catalog);
    assert_eq!(index.row(&catalog.tracks[0].id, &unavailable, &catalog), None);
    assert_eq!(index.track_index(&catalog.tracks[0].id, &catalog), Some(0));
    let empty = Arc::new(Vec::new());
    let missing = CollectionRows::build(&empty, &catalog);
    assert_eq!(missing.row(&catalog.tracks[0].id, &empty, &catalog), None);
    assert_eq!(missing.track_index(&catalog.tracks[0].id, &catalog), Some(0));
}

fn settle(metadata: &mut Metadata, rows: &mut Arc<Vec<LibItem>>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        metadata.poll(rows).unwrap();
        if !metadata.active() { break; }
        assert!(Instant::now() < deadline, "row index owner did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn actual_owner_publishes_matching_map_and_retires_previous_table_off_gui() {
    let (files, catalog, _) = fixture();
    let path = files.0.join("catalog.json");
    let mut store = crate::library::Store::open(path.clone()).unwrap();
    store.catalog = (*catalog).clone();
    store.save().unwrap();
    drop(store);
    let mut metadata = Metadata::new(Some(path));
    let mut rows = Arc::new(Vec::new());
    settle(&mut metadata, &mut rows);
    assert!(metadata.collection_rows().is_for(&rows, &metadata.catalog));
    for track in &metadata.catalog.tracks {
        let row = metadata.collection_rows().row(&track.id, &rows, &metadata.catalog).unwrap();
        assert_eq!(rows[row].source, track.source);
    }
    let (send, dropped) = std::sync::mpsc::channel();
    *metadata.collection_rows.dropped.lock().unwrap() = Some(send);
    metadata.rebase();
    settle(&mut metadata, &mut rows);
    let owner = dropped.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_ne!(owner, std::thread::current().id());
    assert!(metadata.collection_rows().is_for(&rows, &metadata.catalog));
}
