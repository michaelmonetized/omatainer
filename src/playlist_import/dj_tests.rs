use super::tests::{audio, input};
use super::*;
use crate::{
    engine::{media_analysis::tests::Files, media_source::LibSource},
    library::crates::Edit,
};

fn rekordbox(files: &Files) -> String {
    let a = url::Url::from_file_path(files.0.join("東京.flac")).unwrap();
    let b = url::Url::from_file_path(files.0.join("two.flac")).unwrap();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><DJ_PLAYLISTS Version="1.0.0"><COLLECTION Entries="2">
      <TRACK TrackID="1" Name="東京" Artist="Björk" Location="{a}" AverageBpm="120" Tonality="Am" Rating="204" Genre="House" Comments="Keep the intro">
        <TEMPO Inizio="0.5" Bpm="120" Metro="4/4" Battito="1"/><TEMPO Inizio="4.5" Bpm="128" Metro="4/4" Battito="1"/>
        <POSITION_MARK Name="Drop" Type="0" Start="3" Num="0" Red="1" Green="2" Blue="3"/>
        <POSITION_MARK Name="Loop" Type="4" Start="4" End="6" Num="1"/>
      </TRACK><TRACK TrackID="2" Name="Two" Location="{b}"/>
      </COLLECTION><PLAYLISTS><NODE Type="0" Name="ROOT" Count="2"><NODE Type="0" Name="Sets" Count="1">
      <NODE Type="1" Name="Friday" KeyType="0" Entries="3"><TRACK Key="2"/><TRACK Key="1"/><TRACK Key="2"/></NODE></NODE>
      <NODE Type="0" Name="Empty" Count="0"/></NODE></PLAYLISTS></DJ_PLAYLISTS>"#
    )
}

#[test]
fn rekordbox_folders_order_metadata_state_and_repeated_import_survive() {
    let files = Files::new();
    audio(&files, "東京.flac");
    audio(&files, "two.flac");
    let request = input(&files, "odd-extension.data", &rekordbox(&files));
    let mut catalog = Catalog::default();
    let inspected = review(&request, &catalog, || true).unwrap();
    assert_eq!(
        inspected
            .playlists
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["Sets", "Friday", "Empty"]
    );
    assert_eq!(inspected.playlists[1].folders, ["Sets"]);
    assert!(inspected.rows[2].status.contains("Duplicate"));
    let id = apply(&inspected, &[1, 2], &mut catalog, || true).unwrap();
    assert_eq!(catalog.crates.nodes().len(), 3);
    assert_eq!(catalog.crates.node(&id[0]).unwrap().members.len(), 2);
    let track = catalog
        .track(&LibSource::File(files.0.join("東京.flac")))
        .unwrap();
    let preparation = track.versions[0].preparation;
    assert_eq!(track.annotations.rating, 4);
    assert_eq!(track.annotations.tags, ["House"]);
    assert_eq!(preparation.hotcues[0], Some(3.0));
    assert_eq!(preparation.hotcue_styles[0].name.as_str(), "Drop");
    assert_eq!(preparation.hotcue_styles[0].color, Some([1, 2, 3]));
    assert_eq!(preparation.saved_loops.cue_loops[1], Some(2));
    assert_eq!(preparation.saved_loops.slots[1].unwrap().length, 2.0);
    assert!((preparation.grid.unwrap().bpm_at(5.0).unwrap() - 128.0).abs() < 1e-9);
    assert_eq!(
        track.versions[0].metadata.bpm.origin,
        crate::ui::bpm::Origin::Imported
    );
    assert_eq!(
        catalog
            .imports
            .iter()
            .find(|r| r.key == "root/0/0")
            .unwrap()
            .references
            .len(),
        3
    );
    let stored = files.0.join("catalog.json");
    std::fs::write(&stored, serde_json::to_vec(&catalog).unwrap()).unwrap();
    let mut reopened = crate::library::read(&stored).unwrap();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let backup = files.0.join("backup");
    let exported =
        crate::library::backup::export(&reopened, &backup, None, &cancel, None, |_| {}).unwrap();
    assert_eq!(
        crate::library::backup::inspect(&backup, &cancel, |_| {})
            .unwrap()
            .crates,
        3
    );
    let restored =
        crate::library::backup::restore(&backup, &files.0.join("restore"), &cancel, None, |_| {})
            .unwrap();
    assert_eq!(
        crate::library::read(&restored.catalog).unwrap().imports,
        reopened.imports
    );
    assert!(exported.catalog.exists());
    reopened
        .edit_crates(
            reopened.crates.revision(),
            &Edit::Rename {
                id: id[0].clone(),
                name: "My Friday".into(),
            },
        )
        .unwrap();
    let again = review(&request, &reopened, || true).unwrap();
    let ids = apply(&again, &[1, 2], &mut reopened, || true).unwrap();
    assert_eq!(ids, id);
    assert_eq!(reopened.crates.node(&id[0]).unwrap().name, "My Friday");
    assert_eq!(reopened.crates.nodes().len(), 3);
    assert_eq!(
        reopened
            .track(&LibSource::File(files.0.join("東京.flac")))
            .unwrap()
            .versions[0]
            .preparation,
        preparation
    );
    std::fs::write(
        &request.path,
        rekordbox(&files).replace("Friday", "Saturday"),
    )
    .unwrap();
    let changed = review(&request, &reopened, || true).unwrap();
    let before = serde_json::to_vec(&reopened).unwrap();
    assert!(apply(&changed, &[1], &mut reopened, || true)
        .unwrap_err()
        .contains("new snapshot"));
    assert_eq!(serde_json::to_vec(&reopened).unwrap(), before);
    apply_snapshot(&changed, &[1], &mut reopened, true, || true).unwrap();
    assert_eq!(reopened.crates.nodes().len(), 5);
    assert_eq!(reopened.crates.node(&id[0]).unwrap().name, "My Friday");
}

#[test]
fn traktor_nml_paths_hierarchy_cues_and_explicit_foreign_volume_mapping() {
    let files = Files::new();
    audio(&files, "one.flac");
    let directory = format!("{}:", files.0.to_string_lossy().replace('/', "/:"));
    let directory = if directory.ends_with("/:") {
        directory
    } else {
        format!("{}/:", directory.trim_end_matches(':'))
    };
    let text = format!(
        r#"<NML VERSION="19"><COLLECTION ENTRIES="1"><ENTRY TITLE="One" ARTIST="DJ"><LOCATION DIR="{directory}" FILE="one.flac" VOLUME=""/><INFO GENRE="Techno"/><TEMPO BPM="120"/>
      <CUE_V2 NAME="AutoGrid" TYPE="4" START="500" LEN="0" HOTCUE="-1"/><CUE_V2 NAME="Drop" TYPE="0" START="1000" LEN="0" HOTCUE="0"/><CUE_V2 NAME="Roll" TYPE="5" START="2000" LEN="1000" HOTCUE="1"/></ENTRY></COLLECTION>
      <PLAYLISTS><NODE TYPE="FOLDER" NAME="$ROOT"><SUBNODES COUNT="1"><NODE TYPE="FOLDER" NAME="Sets"><SUBNODES COUNT="1"><NODE TYPE="PLAYLIST" NAME="Saturday"><PLAYLIST ENTRIES="1" TYPE="LIST"><ENTRY><PRIMARYKEY TYPE="TRACK" KEY="{directory}one.flac"/></ENTRY></PLAYLIST></NODE></SUBNODES></NODE></SUBNODES></NODE></PLAYLISTS></NML>"#
    );
    let request = input(&files, "collection.nml", &text);
    let mut catalog = Catalog::default();
    let inspected = review(&request, &catalog, || true).unwrap();
    assert!(inspected.rows[0].ready(), "{}", inspected.rows[0].status);
    apply(&inspected, &[1], &mut catalog, || true).unwrap();
    let prep = catalog.tracks[0].versions[0].preparation;
    assert_eq!(prep.grid.unwrap().downbeat(), 0.5);
    assert_eq!(prep.hotcues[0], Some(1.0));
    assert_eq!(prep.saved_loops.slots[1].unwrap().length, 1.0);
    let foreign = text
        .replace(&directory, "/:Music/:")
        .replace("VOLUME=\"\"", "VOLUME=\"Macintosh HD\"")
        .replace("KEY=\"/:Music/:", "KEY=\"Macintosh HD/:Music/:");
    let mut request = input(&files, "foreign.nml", &foreign);
    let unresolved = review(&request, &Catalog::default(), || true).unwrap();
    assert!(!unresolved.rows[0].ready());
    request.mapping = Some(Mapping {
        from: "Macintosh HD/Music".into(),
        to: files.0.clone(),
    });
    let mapped = review(&request, &Catalog::default(), || true).unwrap();
    assert!(mapped.rows[0].ready(), "{}", mapped.rows[0].status);
}

fn record(tag: &[u8; 4], value: &[u8]) -> Vec<u8> {
    tag.iter()
        .copied()
        .chain((value.len() as u32).to_be_bytes())
        .chain(value.iter().copied())
        .collect()
}
fn utf16(value: &str) -> Vec<u8> {
    value.encode_utf16().flat_map(u16::to_be_bytes).collect()
}
#[test]
fn legacy_serato_content_versions_truncation_and_root_relative_paths() {
    let files = Files::new();
    audio(&files, "東京.flac");
    let mut bytes = record(b"vrsn", &utf16("1.0/Serato ScratchLive Crate"));
    let snapshot = Snapshot::discover().unwrap();
    let location = snapshot.identify(&files.0.join("東京.flac")).unwrap();
    let root = snapshot.volume_root(&location).unwrap();
    let reference = files
        .0
        .join("東京.flac")
        .strip_prefix(root)
        .unwrap()
        .to_string_lossy()
        .to_string();
    bytes.extend(record(b"otrk", &record(b"ptrk", &utf16(&reference))));
    let request = Input {
        path: files.0.join("Sets%%Friday.crate"),
        mapping: None,
    };
    std::fs::write(&request.path, &bytes).unwrap();
    let inspected = review(&request, &Catalog::default(), || true).unwrap();
    assert_eq!(inspected.playlists[1].folders, ["Sets"]);
    assert!(inspected.rows[0].ready());
    for truncated in [0, 4, 7, bytes.len() - 1] {
        assert!(serato::parse(&bytes[..truncated], &request.path, &|| true).is_err());
    }
    let unknown = record(b"vrsn", &utf16("4.0/Serato Library"));
    assert!(serato::parse(&unknown, &request.path, &|| true)
        .unwrap_err()
        .contains("Unsupported Serato"));
    let traversal = [
        record(b"vrsn", &utf16("1.0/Serato ScratchLive Crate")),
        record(b"otrk", &record(b"ptrk", &utf16("../escape.flac"))),
    ]
    .concat();
    assert!(serato::parse(&traversal, &request.path, &|| true).is_err());
    assert!(serato::parse(&bytes, &request.path, &|| false).is_err());
}

#[test]
fn dj_xml_refuses_unknown_versions_duplicate_identities_counts_and_corrupt_values() {
    let files = Files::new();
    let text = rekordbox(&files);
    for invalid in [
        text.replace("Version=\"1.0.0\"", "Version=\"99\""),
        text.replace("TrackID=\"2\"", "TrackID=\"1\""),
        text.replace("Entries=\"2\"", "Entries=\"3\""),
        text.replace("AverageBpm=\"120\"", "AverageBpm=\"NaN\""),
        text.replace("Start=\"4\" End=\"6\"", "Start=\"6\" End=\"4\""),
    ] {
        let tree = crate::interchange_xml::parse(invalid.as_bytes(), &|| true).unwrap();
        assert!(djxml::parse(&tree, &|| true).is_err());
    }
    assert!(djxml::parse(
        &crate::interchange_xml::parse(text.as_bytes(), &|| true).unwrap(),
        &|| false
    )
    .is_err());
}

#[test]
fn apple_persistent_folder_identity_orders_parents_and_refuses_cycles() {
    let source = r#"<plist version="1.0"><dict><key>Tracks</key><dict></dict><key>Playlists</key><array>
      <dict><key>Name</key><string>Child</string><key>Playlist Persistent ID</key><string>B</string><key>Parent Persistent ID</key><string>A</string><key>Playlist Items</key><array></array></dict>
      <dict><key>Name</key><string>Parent</string><key>Playlist Persistent ID</key><string>A</string><key>Folder</key><true/></dict>
      </array></dict></plist>"#;
    let parsed = plist::parse(source.as_bytes(), &|| true).unwrap();
    assert_eq!(
        parsed.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["Parent", "Child"]
    );
    assert_eq!(parsed[1].folders, ["Parent"]);
    let cyclic = source.replace(
        "<key>Folder</key><true/>",
        "<key>Folder</key><true/><key>Parent Persistent ID</key><string>B</string>",
    );
    assert!(plist::parse(cyclic.as_bytes(), &|| true).is_err());
    let missing = source.replace(
        "<string>A</string><key>Playlist Items",
        "<string>Missing</string><key>Playlist Items",
    );
    assert!(plist::parse(missing.as_bytes(), &|| true).is_err());
    let files = Files::new();
    let request = input(&files, "folders.xml", source);
    let inspected = review(&request, &Catalog::default(), || true).unwrap();
    let mut catalog = Catalog::default();
    apply(&inspected, &[1], &mut catalog, || true).unwrap();
    assert_eq!(catalog.crates.nodes().len(), 2);
    let mut saved = serde_json::to_value(&catalog).unwrap();
    saved["schema"] = 17.into();
    std::fs::write(
        files.0.join("downgraded.json"),
        serde_json::to_vec(&saved).unwrap(),
    )
    .unwrap();
    assert!(crate::library::read(&files.0.join("downgraded.json"))
        .unwrap_err()
        .contains("schema 18"));
}

#[test]
fn repeated_sibling_names_and_source_only_metadata_are_visible_and_durable() {
    let files = Files::new();
    audio(&files, "東京.flac");
    audio(&files, "two.flac");
    let xml = rekordbox(&files)
        .replace("Name=\"Friday\"", "Name=\"Same\"")
        .replace("Name=\"Empty\"", "Name=\"Same\"")
        .replace(
            "Artist=\"Björk\"",
            "Artist=\"Björk\" Album=\"Source album\"",
        );
    let request = input(&files, "metadata.xml", &xml);
    let mut catalog = Catalog::default();
    let inspected = review(&request, &catalog, || true).unwrap();
    assert!(inspected
        .warnings(1)
        .iter()
        .any(|w| w.contains("Source album")));
    apply(&inspected, &[1, 2], &mut catalog, || true).unwrap();
    let leaf = catalog
        .imports
        .iter()
        .find(|p| p.key == "root/0/0")
        .unwrap();
    assert_eq!(leaf.references[1].details.title, "東京");
    assert_eq!(leaf.references[1].details.artist, "Björk");
    assert!(leaf.references[1]
        .details
        .warnings
        .iter()
        .any(|w| w.contains("Source album")));
    let repeated = xml.replace("Name=\"Same\" Count=\"0\"", "Name=\"Sets\" Count=\"0\"");
    let changed = input(&files, "two.xml", &repeated);
    let inspected = review(&changed, &catalog, || true).unwrap();
    assert_eq!(inspected.playlists[0].destination, "Sets (import 2)");
    assert_eq!(inspected.playlists[2].destination, "Sets (import 3)");
    apply(&inspected, &[1, 2], &mut catalog, || true).unwrap();
}
#[test]
fn provenance_refuses_injected_identity_fields_and_oversized_serialization() {
    use crate::library::imports::{Collection, Details, Reference};
    let mut record = Collection {
        source: LibSource::File("/known/source.xml".into()),
        digest: [1; 32],
        format: "rekordbox XML 1.0.0".into(),
        key: "root/0".into(),
        crate_id: crate::library::crates::CrateId("1".repeat(32)),
        references: vec![],
    };
    crate::library::imports::validate(&[record.clone()], &[]).unwrap();
    record.key.push('\n');
    assert!(crate::library::imports::validate(&[record.clone()], &[]).is_err());
    record.key = "root/0".into();
    let details = Arc::new(Details {
        warnings: vec!["x".repeat(4000)],
        ..Default::default()
    });
    record.references = vec![
        Reference {
            reference: "track.flac".into(),
            track: None,
            details
        };
        9000
    ];
    assert!(crate::library::imports::validate(&[record], &[])
        .unwrap_err()
        .contains("32 MiB"));
}

#[test]
fn m3u_magic_works_with_an_unusual_filename_and_retains_original_titles() {
    let files = Files::new();
    audio(&files, "one.flac");
    let request = input(
        &files,
        "set.data",
        "#EXTM3U\n#EXTINF:30,DJ - First\none.flac\n#EXTINF:30,DJ - Second\none.flac\n",
    );
    let inspected = review(&request, &Catalog::default(), || true).unwrap();
    assert_eq!(inspected.rows[0].title, "First");
    assert_eq!(inspected.rows[1].title, "Second");
    let mut catalog = Catalog::default();
    apply(&inspected, &[0], &mut catalog, || true).unwrap();
    assert_eq!(catalog.imports[0].references[0].details.title, "First");
    assert_eq!(catalog.imports[0].references[1].details.title, "Second");
    assert_eq!(catalog.crates.nodes()[0].members.len(), 1);
}
