use super::*;
use crate::engine::media_analysis::tests::Files;
use crate::engine::media_source::LibSource;

fn audio(files:&Files,name:&str) {std::fs::write(files.0.join(name),include_bytes!("../../tests/fixtures/audio/tone.flac")).unwrap();}
fn input(files:&Files,name:&str,text:&str)->Input {let path=files.0.join(name);std::fs::write(&path,text).unwrap();Input{path,mapping:None}}
fn xml(tracks:&str,items:&str)->String {format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\"><plist version=\"1.0\"><dict><key>Tracks</key><dict>{tracks}</dict><key>Playlists</key><array><dict><key>Name</key><string>Björk &amp; 東京</string><key>Playlist Items</key><array>{items}</array></dict></array></dict></plist>")}
fn track(id:u64,location:&str,extra:&str)->String {format!("<key>{id}</key><dict><key>Track ID</key><integer>{id}</integer><key>Name</key><string>Track {id}</string><key>Artist</key><string>Artist</string><key>Location</key><string>{location}</string>{extra}</dict>")}
fn item(id:u64)->String {format!("<dict><key>Track ID</key><integer>{id}</integer></dict>")}

#[test]
fn unicode_relative_uri_order_duplicates_and_existing_preparation_survive() {
    let files=Files::new();audio(&files,"東京.flac");audio(&files,"Björk #1.flac");
    let uri=url::Url::from_file_path(files.0.join("Björk #1.flac")).unwrap();
    let input=input(&files,"Mix.m3u8",&format!("\u{feff}#EXTM3U\r\n#EXTINF:1,Artist - 東京\r\n東京.flac\r\n{uri}\r\n./東京.flac\r\nhttps://example.com/song.mp3\r\nmissing.flac\r\nprotected.m4p\r\n"));
    let mut catalog=Catalog::default();let before=review(&input,&catalog,||true).unwrap();
    assert_eq!(before.rows.len(),6);assert_eq!(before.rows.iter().filter(|r|r.ready()).count(),3);assert!(before.rows[2].status.contains("Duplicate"));assert!(before.rows[3].status.contains("Provider-only"));assert!(before.rows[5].status.contains("Protected"));
    let ids=apply(&before,&[0],&mut catalog,||true).unwrap();let members=catalog.crates.node(&ids[0]).unwrap().members.clone();assert_eq!(members.len(),2);assert_eq!(catalog.tracks.iter().find(|t|t.id==members[0]).unwrap().versions[0].metadata.title,"東京");
    let source=before.rows[0].media.as_ref().unwrap().location.source.clone();let fp=before.rows[0].media.as_ref().unwrap().fingerprint;
    catalog.upsert(source.clone(),Some(fp),before.rows[0].media.as_ref().unwrap().metadata.clone()).unwrap().preparation.cue=0.25;
    let prepared=catalog.track(&source).unwrap().clone();let again=review(&input,&catalog,||true).unwrap();assert!(again.rows[0].status.contains("existing"));
    catalog.edit_crates(catalog.crates.revision(),&Edit::Rename{id:ids[0].clone(),name:"Original".into()}).unwrap();
    apply(&again,&[0],&mut catalog,||true).unwrap();assert_eq!(catalog.track(&source).unwrap().versions,prepared.versions);
}

#[test]
fn apple_utf8_utf16_windows_mapping_protected_provider_missing_ids_and_static_order() {
    let files=Files::new();audio(&files,"東京.flac");audio(&files,"two.flac");
    let text=xml(&(track(7,"file://localhost/C:/Music/%E6%9D%B1%E4%BA%AC.flac","")+&track(9,"file://localhost/C:/Music/two.flac","")+&track(10,"","<key>Protected</key><true/>")+&track(11,"https://music.apple.com/example","<key>Track Type</key><string>Remote</string>")),&(item(9)+&item(7)+&item(9)+&item(10)+&item(11)+&item(99)));
    let mut input=input(&files,"Windows.xml",&text);let unmapped=review(&input,&Catalog::default(),||true).unwrap();assert!(unmapped.rows.iter().all(|r|!r.ready()));
    input.mapping=Some(Mapping{from:"C:\\Music".into(),to:files.0.clone()});
    let reviewed=review(&input,&Catalog::default(),||true).unwrap();assert_eq!(reviewed.playlists[0].name,"Björk & 東京");assert!(reviewed.rows[3].status.contains("Protected"));assert!(reviewed.rows[4].status.contains("Provider-only"));assert!(reviewed.rows[5].status.contains("absent"));
    let mut catalog=Catalog::default();let ids=apply(&reviewed,&[0],&mut catalog,||true).unwrap();let members=&catalog.crates.node(&ids[0]).unwrap().members;assert_eq!(members.len(),2);assert_eq!(catalog.tracks.iter().find(|t|&t.id==&members[0]).unwrap().source,reviewed.rows[0].media.as_ref().unwrap().location.source);
    let utf16=text.replace("UTF-8","UTF-16");let bytes:Vec<_>=[0xffu8,0xfe].into_iter().chain(utf16.encode_utf16().flat_map(u16::to_le_bytes)).collect();std::fs::write(&input.path,bytes).unwrap();let second=review(&input,&Catalog::default(),||true).unwrap();assert_eq!(second.rows.iter().filter(|r|r.ready()).count(),3);
}

#[test]
fn stale_playlist_media_and_catalog_versions_refuse_before_publication() {
    let files=Files::new();audio(&files,"one.flac");let input=input(&files,"Set.m3u","one.flac\n");let mut catalog=Catalog::default();let first=review(&input,&catalog,||true).unwrap();
    std::fs::write(&input.path,"#changed\none.flac\n").unwrap();assert!(apply(&first,&[0],&mut catalog,||true).unwrap_err().contains("Playlist changed"));assert!(catalog.tracks.is_empty());
    let next=review(&input,&catalog,||true).unwrap();std::fs::write(files.0.join("one.flac"),b"replaced").unwrap();assert!(apply(&next,&[0],&mut catalog,||true).is_err());assert!(catalog.tracks.is_empty());
    audio(&files,"one.flac");let next=review(&input,&catalog,||true).unwrap();let media=next.rows[0].media.as_ref().unwrap();catalog.upsert(media.location.source.clone(),Some(media.fingerprint),media.metadata.clone()).unwrap();assert!(apply(&next,&[0],&mut catalog,||true).unwrap_err().contains("Catalog source/version changed"));
    let baseline=catalog.tracks.clone();audio(&files,"one.flac");let blocked=review(&input,&catalog,||true).unwrap();assert!(!blocked.rows[0].ready());assert_eq!(catalog.tracks,baseline);
}

#[test]
fn selected_playlists_alone_add_tracks_and_unselected_media_changes_are_irrelevant() {
    let files=Files::new();audio(&files,"one.flac");audio(&files,"two.flac");
    let tracks=track(1,"one.flac","")+&track(2,"two.flac","");
    let playlists=format!("<dict><key>Name</key><string>One</string><key>Playlist Items</key><array>{}</array></dict><dict><key>Name</key><string>Two</string><key>Playlist Items</key><array>{}</array></dict>",item(1),item(2));
    let text=format!("<plist version=\"1.0\"><dict><key>Tracks</key><dict>{tracks}</dict><key>Playlists</key><array>{playlists}</array></dict></plist>");
    let input=input(&files,"Selected.xml",&text);let review=review(&input,&Catalog::default(),||true).unwrap();assert_eq!(review.rows.len(),2);
    std::fs::write(files.0.join("one.flac"),b"changed unselected source").unwrap();
    let mut catalog=Catalog::default();let ids=apply(&review,&[1],&mut catalog,||true).unwrap();assert_eq!(catalog.tracks.len(),1);assert_eq!(catalog.crates.node(&ids[0]).unwrap().name,"Two");assert_eq!(catalog.tracks[0].source,LibSource::File(files.0.join("two.flac")));
}

#[test]
fn malformed_entities_duplicate_keys_oversized_depth_invalid_encoding_and_cancel_refuse() {
    for text in ["<plist><dict><key>Tracks</key></dict></plist>","<!DOCTYPE plist [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><plist><string>&x;</string></plist>","<plist><dict><key>x</key><string>1</string><key>x</key><string>2</string></dict></plist>","<plist><string>&unknown;</string></plist>"] {assert!(plist::parse(text.as_bytes(),&||true).is_err());}
    let deep=format!("<plist>{}<string>x</string>{}</plist>","<array>".repeat(34),"</array>".repeat(34));assert!(plist::parse(deep.as_bytes(),&||true).is_err());
    assert!(m3u::parse(&[0xff,0xff],"bad",&||true).is_err());assert!(m3u::parse(b"one.flac","cancel",&||false).is_err());
    let files=Files::new();let input=input(&files,"Cancel.m3u","one.flac");assert!(review(&input,&Catalog::default(),||false).is_err());
    assert!(local_path("C:/Music/../escape.flac",&files.0,Some(&Mapping{from:"C:/Music".into(),to:files.0.clone()})).is_err());
}

#[test]
#[ignore="Local qualification command: OMATAINER_PLAYLIST_M3U and OMATAINER_PLAYLIST_XML name existing exported files"]
fn qualify_existing_exports_without_modifying_sources() {
    for variable in ["OMATAINER_PLAYLIST_M3U","OMATAINER_PLAYLIST_XML"] {
        let path=PathBuf::from(std::env::var(variable).unwrap());let before=std::fs::read(&path).unwrap();let reviewed=review(&Input{path:path.clone(),mapping:None},&Catalog::default(),||true).unwrap();
        assert!(!reviewed.playlists.is_empty());assert!(!reviewed.rows.is_empty());assert_eq!(std::fs::read(&path).unwrap(),before);
        eprintln!("{variable}: {} playlists, {} references, {} resolved",reviewed.playlists.len(),reviewed.rows.len(),reviewed.rows.iter().filter(|r|r.ready()).count());
        if variable.ends_with("M3U") {assert_eq!(reviewed.rows.iter().filter(|r|r.ready()).count(),12);let mut catalog=Catalog::default();let ids=apply(&reviewed,&[0],&mut catalog,||true).unwrap();assert_eq!(catalog.crates.node(&ids[0]).unwrap().members.len(),12);}
    }
}
