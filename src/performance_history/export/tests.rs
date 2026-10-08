use super::*;
use crate::engine::{history_measurement::{Classification, Episode, Observation}, media_source::FileFingerprint, performance};

fn session() -> Session {
    let mut session=Session::new("a".repeat(32),1,1000,0).unwrap();
    let source=Source::Catalog{track_id:"b".repeat(32),version:3,title:"=SUM(1,2) \"音\"".into(),artist:"Guest, artist".into()};
    session.observe(Observation{session:1,episode:Episode{load:2,generation:1,deck:0},first_frame:0,frames:480,sample_rate:48_000,classification:Classification::Active,wall_ns:2000},source).unwrap();
    session.mark(0,1,Some(false)).unwrap();
    session.external(1,"Guest vinyl".into(),"Artist".into()).unwrap();
    session.end(3000,480,true,32).unwrap();session
}

#[test]
fn text_csv_and_json_preserve_measurement_manual_marks_and_private_identity() {
    let session=session();let handle=performance::Handle::default();let permit=handle.optional_work().unwrap();
    let csv=String::from_utf8(encode(&session,Format::Csv,false,None,&permit).unwrap()).unwrap();
    assert_eq!(csv.lines().count(),3);assert!(csv.ends_with("\r\n"));
    assert!(csv.contains("\"'=SUM(1,2) \"\"音\"\"\""));
    assert!(csv.contains("\"manual_unplayed\",\"0.010000\",\"0.000000\",\"2000\",\"2000\""));
    assert!(csv.contains("\"external\""));assert!(csv.contains("\"manual_played\",\"0.000000\""));
    assert!(csv.contains("\"ended\",\"1000\",\"3000\",\"true\",\"32\""));
    let text=String::from_utf8(encode(&session,Format::Text,false,None,&permit).unwrap()).unwrap();
    assert!(text.contains("Incomplete: true"));assert!(text.contains("0.010 s measured"));assert!(text.contains("manual_unplayed"));
    let json=encode(&session,Format::Json,false,None,&permit).unwrap();assert_eq!(json,session.export().unwrap());
    for bytes in [csv.into_bytes(),text.into_bytes(),json] {let value=String::from_utf8(bytes).unwrap();assert!(!value.contains("load_key"));assert!(!value.contains("fingerprint"));assert!(!value.contains("file://"));}
    let cancelled=handle.optional_work().unwrap();handle.set_enabled(true).unwrap();
    assert!(encode(&session,Format::Csv,false,None,&cancelled).is_err());
}

#[test]
fn playlist_requires_consent_and_exact_online_versions_and_encodes_actual_file_uris() {
    let root=std::env::temp_dir().join(format!("setlist-playlist-{}",crate::performance_history::storage::new_id().unwrap()));std::fs::create_dir(&root).unwrap();
    let path=root.join("音 mix #1%?.mp3");std::fs::write(&path,b"original audio bytes").unwrap();
    let source=LibSource::File(path.clone());let fingerprint=FileFingerprint::read(&path).unwrap();
    let mut catalog=Catalog::default();
    catalog.upsert(source.clone(),Some(fingerprint),crate::library::Metadata{title:"Track".into(),artist:"Artist".into(),bpm:crate::ui::bpm::Bpm::new(120.0,crate::ui::bpm::Origin::User),key:String::new(),duration:Some(20.0),last_play:None}).unwrap();
    let track=catalog.track(&source).unwrap();let id=track.id.0.clone();
    let mut session=Session::new("a".repeat(32),1,1000,0).unwrap();
    session.loaded(2,0,Source::Catalog{track_id:id,version:0,title:"Track".into(),artist:"Artist".into()}).unwrap();session.mark(0,1,Some(true)).unwrap();
    let handle=performance::Handle::default();let permit=handle.optional_work().unwrap();
    assert!(encode(&session,Format::M3u8,false,Some(&catalog),&permit).is_err());
    let output=String::from_utf8(encode(&session,Format::M3u8,true,Some(&catalog),&permit).unwrap()).unwrap();
    assert!(output.starts_with("#EXTM3U\n"));assert!(output.contains("#EXTINF:-1,Artist — Track\nfile:///"));
    assert!(output.contains("%E9%9F%B3%20mix%20%231%25%3F.mp3"));assert!(!output.contains("fingerprint"));
    let store=super::super::storage::Store::open(root.join("history")).unwrap();let destination=root.join("set.m3u8");
    assert_eq!(store.export_as(&session,&destination,Format::M3u8,true,Some(&catalog),&permit).unwrap(),super::super::storage::Commit::Durable);
    assert_eq!(std::fs::read_to_string(&destination).unwrap(),output);
    assert!(store.export_as(&session,&destination,Format::M3u8,true,Some(&catalog),&permit).is_err());
    std::fs::write(&path,b"replacement audio with different bytes").unwrap();
    assert!(encode(&session,Format::M3u8,true,Some(&catalog),&permit).unwrap_err().contains("exact recorded version"));
    session.mark(1,1,Some(false)).unwrap();assert!(encode(&session,Format::M3u8,true,Some(&catalog),&permit).unwrap_err().contains("no played entries"));
    session.external(2,"External".into(),String::new()).unwrap();assert!(encode(&session,Format::M3u8,true,Some(&catalog),&permit).unwrap_err().contains("external or unresolved"));
    let changed_destination=root.join("changed.m3u8");assert!(store.export_as(&session,&changed_destination,Format::M3u8,true,Some(&catalog),&permit).is_err());assert!(!changed_destination.exists());
    drop(store);std::fs::remove_dir_all(root).unwrap();
}
