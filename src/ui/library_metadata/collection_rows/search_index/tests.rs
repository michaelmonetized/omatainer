use super::*;
use crate::ui::bpm::Bpm;
use crate::engine::media_source::LibSource;
fn catalog(count:usize)->Arc<Catalog> {
    let mut catalog=Catalog::default();
    for index in 0..count {
        catalog.upsert(LibSource::File(format!("/synthetic-index/{index}.wav").into()),None,crate::library::Metadata {title:format!("Track {index:06} Café"),artist:if index%2==0 {"Even Artist"} else {"Odd Artist"}.into(),key:"Am".into(),bpm:Bpm::hint(100.0+(index%100) as f32),duration:Some(120.0+(index%60) as f64),last_play:None}).unwrap();
        catalog.tracks[index].annotations.rating=(index%6) as u8;
    }
    Arc::new(catalog)
}
fn rows(catalog:&Catalog)->Arc<Vec<LibItem>> {Arc::new(catalog.tracks.iter().map(|track|LibItem::from_stored(track.source.clone(),&track.versions[track.current])).collect())}
#[test]
fn exact_incremental_metadata_reuses_unicode_records_across_reorder_and_tracks_live_play_membership() {
    let first=catalog(100);let library=rows(&first);let index=Index::build(&library,&first,None).unwrap();assert_eq!(index.prepared,100);
    let mut next=(*first).clone();next.tracks[20].annotations.rating=5;let next=Arc::new(next);
    let next_rows=rows(&next);let changed=Index::build(&next_rows,&next,Some((&index,&library,&first))).unwrap();assert_eq!(changed.prepared,1);
    assert_eq!(changed.delta_for(&Arc::downgrade(&library),&Arc::downgrade(&first)),Some([20].as_slice()));
    assert!(changed.delta_for(&Arc::downgrade(&next_rows),&Arc::downgrade(&next)).is_none());
    assert!(changed.matches(&Query::parse("rating:5 title:cafe\u{301}").unwrap(),20,false));
    let mut reordered=(*next_rows).clone();reordered.reverse();let reordered=Arc::new(reordered);
    let reorder=Index::build(&reordered,&next,Some((&changed,&next_rows,&next))).unwrap();assert_eq!(reorder.prepared,0);assert!(reorder.delta.is_none());
    assert!(reorder.matches(&Query::parse("rating:5 title:café").unwrap(),79,false));
    let mut played=(*next_rows).clone();played[20].last_play=Some(std::time::SystemTime::UNIX_EPOCH);let played=Arc::new(played);
    let play=Index::build(&played,&next,Some((&changed,&next_rows,&next))).unwrap();assert_eq!(play.prepared,0);
    assert_eq!(play.delta_for(&Arc::downgrade(&next_rows),&Arc::downgrade(&next)),Some([20].as_slice()));
    let mut replayed=(*played).clone();replayed[20].last_play=Some(std::time::SystemTime::UNIX_EPOCH+std::time::Duration::from_secs(1));let replayed=Arc::new(replayed);
    let replay=Index::build(&replayed,&next,Some((&play,&played,&next))).unwrap();assert_eq!(replay.prepared,0);
    assert_eq!(replay.delta_for(&Arc::downgrade(&played),&Arc::downgrade(&next)),Some([20].as_slice()));
    let mut replaced=(*played).clone();replaced[20].fingerprint=Some(crate::engine::media_source::FileFingerprint::read(std::path::Path::new("Cargo.toml")).unwrap());let replaced=Arc::new(replaced);
    let replacement=Index::build(&replaced,&next,Some((&play,&played,&next))).unwrap();assert_eq!(replacement.prepared,1);assert!(replacement.delta.is_none());
}
#[test]
fn oversized_normalized_metadata_refuses_a_search_index_without_shortening_fields() {
    let mut catalog=(*catalog(35000)).clone();for track in &mut catalog.tracks {track.annotations.notes="é".repeat(2048);}
    let catalog=Arc::new(catalog);let library=rows(&catalog);
    assert!(Index::build(&library,&catalog,None).err().unwrap().contains("128 MiB"));
}

#[test]
fn prepared_sort_keys_match_existing_unicode_numeric_missing_and_stable_tie_order() {
    use crate::preferences::library_layout::{Column,Sort};
    let mut catalog=(*catalog(120)).clone();
    for (index,track) in catalog.tracks.iter_mut().enumerate(){track.annotations.color=(index%3==0).then_some([index as u8,10,20]);track.annotations.group=if index%2==0 {"Straße"} else {"STRASSE"}.into();track.annotations.notes=format!("{}{:03}","É".repeat(80),120-index);track.annotations.tags=vec![if index%2==0 {"Clean Édit"} else {"Explicit"}.into()];}
    let catalog=Arc::new(catalog);let mut library=(*rows(&catalog)).clone();for (index,item) in library.iter_mut().enumerate(){if index%5==0 {item.length=None;item.bpm=Bpm::UNKNOWN;}item.last_play=(index%7==0).then_some(std::time::SystemTime::UNIX_EPOCH+std::time::Duration::from_secs(index as u64));}
    let library=Arc::new(library);let index=Index::build(&library,&catalog,None).unwrap();let history=crate::ui::play_history::History::default();
    for column in Column::ALL {for descending in [false,true] {
        let sorts=[Some(Sort {column,descending}),Some(Sort {column:if column==Column::Title {Column::Bpm} else {Column::Title},descending:!descending})];
        let mut expected:Vec<_>=(0..120).rev().collect();let mut actual=expected.clone();crate::ui::library_layout::sort::order(&mut expected,&library,&catalog,&history,sorts);actual.sort_by(|a,b|index.compare(*a,*b,&library,&history,sorts));assert_eq!(actual,expected,"{column:?} descending={descending}");
        assert_eq!(crate::engine::test_alloc::measure(||{let _=index.compare(10,20,&library,&history,sorts);}),Default::default());
    }}
}
