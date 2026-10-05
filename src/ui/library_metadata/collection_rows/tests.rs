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

#[test]
fn membership_discovery_includes_direct_manual_annotation_and_typed_rules_only() {
    use crate::library::{crates::{CrateId,Edit},annotations,smart_crates::{Rule,Combine,Condition,NumberField}};
    let (_files,catalog,_) = fixture();
    let mut catalog = (*catalog).clone();
    catalog.tracks[0].annotations.rating = 5;
    let ids: Vec<_> = (1..=4).map(|i|CrateId(format!("{i:032x}"))).collect();
    for (index,id) in ids.iter().enumerate() {
        catalog.edit_crates(catalog.crates.revision(),&Edit::Create { id:id.clone(),name:format!("Crate {index}"),parent:(index==1).then(||ids[0].clone()),before:None }).unwrap();
    }
    catalog.edit_crates(catalog.crates.revision(),&Edit::AddMembers { id:ids[1].clone(),members:vec![catalog.tracks[0].id.clone()],before:None }).unwrap();
    catalog.edit_crates(catalog.crates.revision(),&Edit::SetAnnotationRule { id:ids[2].clone(),rule:Some(annotations::Rule { minimum_rating:4,..Default::default() }) }).unwrap();
    catalog.edit_crates(catalog.crates.revision(),&Edit::SetSmartRule { id:ids[3].clone(),rule:Some(Rule { combine:Combine::All,conditions:vec![Condition::Number { field:NumberField::Rating,minimum:4.0,maximum:5.0 }] }) }).unwrap();
    let catalog = Arc::new(catalog);
    let rows = Arc::new(catalog.tracks.iter().rev().map(|track|LibItem::from_stored(track.source.clone(),&track.versions[track.current])).collect());
    let index = CollectionRows::build(&rows,&catalog);
    let mut membership = index.crates_containing(&catalog.tracks[0].id,&rows,&catalog).unwrap();
    membership.sort_unstable();
    assert_eq!(membership,[1,2,3],"a parent crate does not inherit a child's direct membership");
    assert!(index.crates_containing(&catalog.tracks[1].id,&rows,&catalog).unwrap().is_empty());
    assert!(index.crates_containing(&catalog.tracks[0].id,&Arc::new((*rows).clone()),&catalog).is_none());
    let mut changed = (*catalog).clone();
    changed.tracks[0].annotations.rating = 1;
    let changed = Arc::new(changed);
    let update = CollectionRows::build_incremental(&rows,&changed,Some(&index));
    assert_eq!(update.crates_containing(&changed.tracks[0].id,&rows,&changed).unwrap(),[1]);
    assert!(index.crates_containing(&changed.tracks[0].id,&rows,&changed).is_none());
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

#[test]
fn smart_crate_100000_track_updates_evaluate_changed_members_only_while_audio_runs() {
    use crate::library::{crates::{CrateId,Edit},smart_crates::{Rule,Combine,Condition,NumberField}};
    use std::sync::atomic::{AtomicBool,Ordering};
    struct StopOnDrop(Arc<AtomicBool>);
    impl Drop for StopOnDrop { fn drop(&mut self) { self.0.store(true,Ordering::Release); } }
    let stop = Arc::new(AtomicBool::new(false)); let stopping = stop.clone();
    let _stop_on_failure=StopOnDrop(stop.clone());
    let audio = std::thread::spawn(move || {
        let (engine, mut rt) = crate::engine::Engine::headless_for_test(48000, 256);
        engine.send(crate::engine::Command::DeckPlay { deck:0 }).unwrap();
        let mut buffer = [0.0;512]; rt.process(&mut buffer);
        let mut callbacks = 0usize;
        while !stopping.load(Ordering::Acquire) {
            assert_eq!(crate::engine::test_alloc::measure(||rt.process(&mut buffer)),crate::engine::test_alloc::Counts::default());
            assert!(buffer.iter().all(|sample|sample.is_finite()));
            callbacks += 1;
            std::thread::sleep(Duration::from_micros(100));
        }
        callbacks
    });
    let mut catalog = Catalog::default();
    for index in 0..100000 {
        catalog.upsert(crate::engine::media_source::LibSource::File(format!("/synthetic-smart/{index}.wav").into()), None,
            crate::library::Metadata { title: format!("Track {index}"), artist: "Fixture".into(), bpm: Bpm::hint(100.0+(index%100) as f32), key: "C".into(), duration: Some(180.0),last_play:None }).unwrap();
        catalog.tracks[index].annotations.rating = (index%6) as u8;
    }
    let id = CrateId("00000000000000000000000000000205".into());
    catalog.crates.apply(0,&Edit::Create { id:id.clone(),name:"Smart".into(),parent:None,before:None },|_|true).unwrap();
    catalog.crates.apply(1,&Edit::SetSmartRule { id:id.clone(),rule:Some(Rule { combine:Combine::All,conditions:vec![
        Condition::Number { field:NumberField::Bpm,minimum:120.0,maximum:128.0 },
        Condition::Number { field:NumberField::Rating,minimum:4.0,maximum:5.0 },
    ] }) },|_|true).unwrap();
    let catalog = Arc::new(catalog);
    let rows = Arc::new(catalog.tracks.iter().map(|track|LibItem::from_stored(track.source.clone(),&track.versions[track.current])).collect::<Vec<_>>());
    let first = Instant::now(); let index = CollectionRows::build(&rows,&catalog); let first_elapsed = first.elapsed();
    let expected: Vec<_> = (0..100000).filter(|i|(20..=28).contains(&(i%100)) && i%6>=4).collect();
    assert_eq!(index.smart_rows(&id,&rows,&catalog).unwrap(),expected);
    assert_eq!(index.smart[&id].evaluated,100000);
    let mut changed = (*catalog).clone(); changed.tracks[20].annotations.rating = 5;
    let changed = Arc::new(changed);
    let started = Instant::now(); let update = CollectionRows::build_incremental(&rows,&changed,Some(&index)); let elapsed=started.elapsed();
    assert_eq!(update.smart[&id].evaluated,1);
    let mut expected=expected;expected.push(20);expected.sort_unstable();
    assert_eq!(update.smart_rows(&id,&rows,&changed).unwrap(),expected);
    let mut next_rows=(*rows).clone();next_rows.reverse();
    let next_rows=Arc::new(next_rows);let reordered=CollectionRows::build_incremental(&next_rows,&changed,Some(&update));
    assert_eq!(reordered.smart[&id].evaluated,0);
    assert_eq!(reordered.smart_rows(&id,&next_rows,&changed).unwrap(),(0..100000).filter(|i|expected.binary_search(&(99999-i)).is_ok()).collect::<Vec<_>>());
    let refreshed=CollectionRows::build(&rows,&changed);assert_eq!(refreshed.smart[&id].evaluated,100000);
    assert_eq!(refreshed.smart_rows(&id,&rows,&changed),update.smart_rows(&id,&rows,&changed));
    assert!(update.smart_rows(&id,&rows,&catalog).is_none());
    stop.store(true,Ordering::Release);let callbacks=audio.join().unwrap();
    assert!(callbacks>100);
    eprintln!("Smart crates: 100000 tracks, initial {first_elapsed:?}, one-track update {elapsed:?}, {callbacks} concurrent zero-heap software callbacks");
    assert!(first_elapsed<Duration::from_secs(5) && elapsed<Duration::from_secs(5));
}

#[test]
fn real_catalog_owner_updates_smart_annotations_incrementally_and_manual_refresh_recomputes() {
    use crate::library::{crates::{CrateId,Edit},smart_crates::{Rule,Combine,Condition,NumberField},annotations::Patch};
    use crate::ui::library_metadata::{CollectionAction,CollectionOutcome};
    let (files,catalog,_)=fixture();let mut catalog=(*catalog).clone();
    let id=CrateId("00000000000000000000000000000205".into());
    catalog.edit_crates(0,&Edit::Create { id:id.clone(),name:"Rated".into(),parent:None,before:None }).unwrap();
    catalog.edit_crates(1,&Edit::SetSmartRule { id:id.clone(),rule:Some(Rule { combine:Combine::All,conditions:vec![Condition::Number { field:NumberField::Rating,minimum:4.0,maximum:5.0 }] }) }).unwrap();
    let member=catalog.tracks[0].id.clone();let source=catalog.tracks[0].source.clone();
    let path=files.0.join("catalog.json");let mut store=crate::library::Store::open(path.clone()).unwrap();store.catalog=catalog;store.save().unwrap();drop(store);
    let mut metadata=Metadata::new(Some(path));let mut rows=Arc::new(Vec::new());settle(&mut metadata,&mut rows);
    assert!(metadata.collection_rows().smart_rows(&id,&rows,&metadata.catalog).unwrap().is_empty());
    metadata.edit_crates(metadata.catalog.crates.revision(),CollectionAction::Annotate { ids:vec![member],patch:Patch { rating:Some(5),..Default::default() } }).unwrap();
    settle(&mut metadata,&mut rows);
    assert_eq!(metadata.take_collection_result().unwrap().outcome,CollectionOutcome::Durable { changed:true });
    let result=metadata.collection_rows();let members=result.smart_rows(&id,&rows,&metadata.catalog).unwrap();
    assert_eq!(members.len(),1);assert_eq!(rows[members[0]].source,source);assert_eq!(result.smart[&id].evaluated,1);
    metadata.edit_crates(metadata.catalog.crates.revision(),CollectionAction::Read).unwrap();settle(&mut metadata,&mut rows);
    assert_eq!(metadata.take_collection_result().unwrap().outcome,CollectionOutcome::Read);
    assert_eq!(metadata.collection_rows().smart[&id].evaluated,2);
    assert_eq!(metadata.collection_rows().smart_rows(&id,&rows,&metadata.catalog).unwrap().len(),1);
}

#[test]
fn analyzed_key_updates_smart_membership_without_row_edits_and_locks_restore_saved_key() {
    use crate::library::{crates::{CrateId,Edit},smart_crates::Rule};
    let (_files,catalog,_)=fixture();
    let mut catalog=(*catalog).clone();
    catalog.tracks[0].versions[0].metadata.key="G".into();
    catalog.tracks[1].versions[0].metadata.key="F#".into();
    let id=CrateId("7".repeat(32));
    catalog.edit_crates(catalog.crates.revision(),&Edit::Create {id:id.clone(),name:"C keys".into(),parent:None,before:None}).unwrap();
    catalog.edit_crates(catalog.crates.revision(),&Edit::SetSmartRule {id:id.clone(),rule:Some(Rule::default())}).unwrap();
    let rows=Arc::new(catalog.tracks.iter().rev().map(|track|LibItem::from_stored(track.source.clone(),&track.versions[0])).collect::<Vec<_>>());
    let baseline=Arc::new(catalog.clone());
    let before=CollectionRows::build(&rows,&baseline);
    assert!(before.smart_rows(&id,&rows,&baseline).unwrap().is_empty());
    let track=&catalog.tracks[0];let fingerprint=track.versions[0].fingerprint.unwrap();
    let location=crate::media_location::Location::resolve(&track.source).unwrap();
    let hash=crate::library::hash_project_source(&location.path,fingerprint,||true).unwrap();
    let reference=crate::sampler_bank::SourceRef {track:track.id.clone(),source:track.source.clone(),fingerprint,content_hash:Some(hash)};
    catalog.apply_analysis(&crate::track_analysis::Patch {reference,fields:crate::track_analysis::Fields {bpm:false,duration:false,waveform:false,level:false,key:true},at_unix_ms:1,bpm:None,duration:8.0,waveform:None,level:None,key:Some(crate::musical_key::Analysis {key:Some(crate::musical_key::Key {tonic:0,minor:false}),score:0.9,margin:0.2,frames:32})}).unwrap();
    let analyzed=Arc::new(catalog.clone());
    let changed=CollectionRows::build_incremental(&rows,&analyzed,Some(&before));
    assert_eq!(changed.smart[&id].evaluated,1);
    assert_eq!(changed.smart_rows(&id,&rows,&analyzed).unwrap(),&[1]);
    let mut order=[0,1];
    let sort=crate::preferences::library_layout::Sort {column:crate::preferences::library_layout::Column::Key,descending:false};
    crate::ui::library_layout::sort::order(&mut order,&rows,&analyzed,&crate::ui::play_history::History::default(),[Some(sort),None]);
    assert_eq!(order,[1,0]);
    catalog.tracks[0].locks.metadata=true;
    let locked=Arc::new(catalog);
    let restored=CollectionRows::build_incremental(&rows,&locked,Some(&changed));
    assert_eq!(restored.smart[&id].evaluated,1);
    assert!(restored.smart_rows(&id,&rows,&locked).unwrap().is_empty());
    crate::ui::library_layout::sort::order(&mut order,&rows,&locked,&crate::ui::play_history::History::default(),[Some(sort),None]);
    assert_eq!(order,[0,1]);
}
