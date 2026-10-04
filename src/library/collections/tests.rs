use super::*;
use crate::{
    engine::{
        beatgrid::Grid,
        media_analysis::tests::{wav, Files},
    },
    library::crates::{CrateId, Edit},
    track_analysis::{Fields, Patch},
};

fn id(n: u32) -> CrateId {
    CrateId(format!("{n:032x}"))
}
fn meta(name: &str) -> Metadata {
    Metadata {
        title: name.into(),
        artist: "retained artist".into(),
        bpm: Bpm::new(127.5, Origin::User),
        key: "F#m".into(),
        duration: Some(1.0),
        last_play: Some(SystemTime::UNIX_EPOCH),
    }
}
fn fixture(files: &Files) -> Catalog {
    let mut catalog = Catalog::default();
    for n in 0..3 {
        let proof = files.source(&format!("track{n}.wav"), &wav(8000, 8000, 1, false));
        let version = catalog
            .upsert(
                proof.source.clone(),
                Some(proof.fingerprint),
                meta(&format!("Track {n}")),
            )
            .unwrap();
        version.preparation.grid = Some(Grid::new(-0.25, 128.0).unwrap());
        version.preparation.hotcues[7] = Some(0.75);
        let mut proof = proof;
        proof.track = catalog.track(&proof.source).unwrap().id.clone();
        let LibSource::File(path) = &proof.source else {
            unreachable!()
        };
        proof.content_hash = Some(content::hash_file(path, proof.fingerprint, || true).unwrap());
        catalog
            .apply_analysis(&Patch {
                level: None,
                reference: proof,
                fields: Fields {
                    bpm: true,
                    duration: true,
                    waveform: false, level: false },
                at_unix_ms: 77,
                bpm: None,
                duration: 1.0,
                waveform: None,
            })
            .unwrap();
    }
    catalog
}
fn edit(c: &mut Catalog, op: Edit<TrackId>) {
    c.edit_crates(c.crates.revision(), &op).unwrap();
}
fn create(c: &mut Catalog, n: u32, parent: Option<u32>) {
    edit(
        c,
        Edit::Create {
            id: id(n),
            name: format!("Crate {n}"),
            parent: parent.map(id),
            before: None,
        },
    );
}
fn snapshot(c: &Catalog) -> Vec<u8> {
    serde_json::to_vec(c).unwrap()
}

#[test]
fn schema_eleven_migrates_without_inventing_favorites_and_old_headers_reject_new_flags() {
    let files = Files::new();
    let mut catalog = fixture(&files);
    create(&mut catalog,1,None);
    let path = files.0.join("favorites.json");
    let mut old = serde_json::to_value(&catalog).unwrap();
    old["schema"] = 11.into();
    let bytes = serde_json::to_vec(&old).unwrap();
    fs::write(&path,&bytes).unwrap();
    let mut store = Store::open(path.clone()).unwrap();
    assert_eq!(store.catalog.schema,SCHEMA);
    assert!(!store.catalog.crates.nodes()[0].favorite);
    assert_eq!(fs::read(&path).unwrap(),bytes);
    store.catalog.edit_crates(store.catalog.crates.revision(),&Edit::SetFavorite { id:id(1),favorite:true }).unwrap();
    store.save().unwrap();
    drop(store);
    assert!(Store::open(path.clone()).unwrap().catalog.crates.nodes()[0].favorite);
    assert_eq!(fs::read(path.with_extension("backup.json")).unwrap(),bytes);
    for value in [serde_json::Value::Bool(true),serde_json::Value::Bool(false),serde_json::Value::Null] {
        old["crates"]["nodes"][0]["favorite"] = value;
        let bytes = serde_json::to_vec(&old).unwrap();
        fs::write(&path,&bytes).unwrap();
        assert!(read(&path).unwrap_err().contains("require library schema 12"));
        assert_eq!(fs::read(&path).unwrap(),bytes);
    }
}

#[test]
fn nested_overlapping_ordered_crates_roundtrip_without_touching_sources_or_versions() {
    let files = Files::new();
    let mut catalog = fixture(&files);
    let versions = catalog.tracks.clone();
    let media: Vec<_> = catalog
        .tracks
        .iter()
        .map(|track| {
            let LibSource::File(path) = &track.source else {
                unreachable!()
            };
            (
                path.clone(),
                FileFingerprint::read(path).unwrap(),
                fs::read(path).unwrap(),
            )
        })
        .collect();
    let members: Vec<_> = catalog
        .tracks
        .iter()
        .map(|track| track.id.clone())
        .collect();
    create(&mut catalog, 1, None);
    create(&mut catalog, 2, Some(1));
    create(&mut catalog, 3, None);
    edit(
        &mut catalog,
        Edit::AddMembers {
            id: id(1),
            members: members.clone(),
            before: None,
        },
    );
    edit(
        &mut catalog,
        Edit::AddMembers {
            id: id(2),
            members: members[..2].to_vec(),
            before: None,
        },
    );
    edit(
        &mut catalog,
        Edit::MoveMembers {
            source: id(1),
            destination: id(1),
            members: vec![members[2].clone()],
            before: Some(members[0].clone()),
        },
    );
    edit(
        &mut catalog,
        Edit::MoveMembers {
            source: id(2),
            destination: id(3),
            members: vec![members[1].clone()],
            before: None,
        },
    );
    edit(
        &mut catalog,
        Edit::Rename {
            id: id(3),
            name: "Closing tracks".into(),
        },
    );
    edit(
        &mut catalog,
        Edit::MoveCrate {
            id: id(3),
            parent: Some(id(1)),
            before: Some(id(2)),
        },
    );
    assert_eq!(
        catalog.crates.node(&id(1)).unwrap().members,
        vec![members[2].clone(), members[0].clone(), members[1].clone()]
    );
    assert_eq!(
        catalog.crates.node(&id(1)).unwrap().children,
        vec![id(3), id(2)]
    );
    assert_eq!(
        catalog.crates.node(&id(2)).unwrap().members,
        vec![members[0].clone()]
    );
    let path = files.0.join("catalog.json");
    let mut store = Store::open(path.clone()).unwrap();
    store.catalog = catalog;
    store.save().unwrap();
    let exact = snapshot(&store.catalog);
    drop(store);
    let mut store = Store::open(path).unwrap();
    assert_eq!(snapshot(&store.catalog), exact);
    let revision = store.catalog.crates.revision();
    assert!(!store
        .catalog
        .edit_crates(
            revision,
            &Edit::AddMembers {
                id: id(1),
                members: vec![members[0].clone()],
                before: None
            }
        )
        .unwrap());
    assert_eq!(store.catalog.crates.revision(), revision);
    edit(&mut store.catalog, Edit::DeleteSubtree { id: id(3) });
    store.save().unwrap();
    assert!(store.catalog.crates.node(&id(3)).is_none());
    assert_eq!(
        store.catalog.tracks, versions,
        "deleting collections must not delete media/version records"
    );
    for (path, fingerprint, bytes) in media {
        assert_eq!(FileFingerprint::read(&path), Some(fingerprint));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn import_merges_track_and_crate_identities_atomically_and_rejects_all_conflicts() {
    let files = Files::new();
    let mut left = fixture(&files);
    create(&mut left, 1, None);
    let mut incoming = Catalog::default();
    let source = LibSource::File(files.0.join("import.wav"));
    incoming
        .upsert(source.clone(), None, meta("Imported"))
        .unwrap();
    create(&mut incoming, 2, None);
    let imported_id = incoming.track(&source).unwrap().id.clone();
    edit(
        &mut incoming,
        Edit::AddMembers {
            id: id(2),
            members: vec![imported_id],
            before: None,
        },
    );
    left.merge_import(incoming.clone()).unwrap();
    let expected = snapshot(&left);
    left.merge_import(incoming.clone()).unwrap();
    assert_eq!(
        snapshot(&left),
        expected,
        "idempotent import cannot advance order revision"
    );
    let mut conflict = incoming.clone();
    edit(
        &mut conflict,
        Edit::Rename {
            id: id(2),
            name: "Conflicting imported identity".into(),
        },
    );
    conflict
        .upsert(
            LibSource::File(files.0.join("must-not-leak.wav")),
            None,
            meta("Must not leak"),
        )
        .unwrap();
    assert!(left.merge_import(conflict).is_err());
    assert_eq!(snapshot(&left), expected);
    let mut unknown = serde_json::to_value(&incoming).unwrap();
    unknown["crates"]["nodes"][0]["members"][0] = serde_json::json!("f".repeat(32));
    assert!(left
        .merge_import(serde_json::from_value(unknown).unwrap())
        .is_err());
    assert_eq!(snapshot(&left), expected);
    let mut duplicate = incoming.clone();
    duplicate.tracks[0].source = LibSource::File(files.0.join("different-location.wav"));
    assert!(left.merge_import(duplicate).is_err());
    assert_eq!(snapshot(&left), expected);
    let path = files.0.join("imported.json");
    let mut store = Store::open(path.clone()).unwrap();
    store.catalog = left;
    store.save().unwrap();
    drop(store);
    assert_eq!(snapshot(&Store::open(path).unwrap().catalog), expected);
}

#[test]
fn migrations_have_empty_collections_and_current_schema_requires_explicit_valid_forest() {
    for schema in 1..=5 {
        let files = Files::new();
        let original = fixture(&files);
        let mut old = serde_json::to_value(&original).unwrap();
        old["schema"] = schema.into();
        old.as_object_mut().unwrap().remove("crates");
        old.as_object_mut().unwrap().remove("watched_roots");
        if schema == 1 {
            old["tracks"]=serde_json::Value::Array(original.tracks.iter().map(|track| serde_json::json!({
            "id":track.id,"source":track.source,"fingerprint":track.versions[0].fingerprint,"metadata":track.versions[0].metadata,"preparation":track.versions[0].preparation
        })).collect());
        }
        let path = files.0.join("old.json");
        let bytes = serde_json::to_vec(&old).unwrap();
        fs::write(&path, &bytes).unwrap();
        let mut store = Store::open(path.clone()).unwrap();
        assert_eq!(store.catalog.schema, SCHEMA);
        assert!(store.catalog.crates.nodes().is_empty());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        for (actual, expected) in store.catalog.tracks.iter().zip(&original.tracks) {
            assert_eq!(actual.id, expected.id);
            assert_eq!(
                actual.versions[0].preparation,
                expected.versions[0].preparation
            );
            if schema > 1 {
                assert_eq!(actual.versions, expected.versions);
            }
        }
        store.save().unwrap();
        assert_eq!(fs::read(path.with_extension("backup.json")).unwrap(), bytes);
        assert_eq!(read(&path).unwrap().schema, SCHEMA);
    }
    for corrupt in 0..4 {
        let files = Files::new();
        let mut value = serde_json::to_value(fixture(&files)).unwrap();
        match corrupt {
            0 => {
                value.as_object_mut().unwrap().remove("crates");
            }
            1 => value["schema"] = 99.into(),
            2 => value["unexpected"] = true.into(),
            _ => value["crates"]["roots"] = serde_json::json!(["1".repeat(32)]),
        }
        let path = files.0.join("invalid.json");
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(Store::open(path.clone()).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn refresh_replacement_missing_media_and_verified_relocation_preserve_membership() {
    let files = Files::new();
    let mut catalog = fixture(&files);
    create(&mut catalog, 1, None);
    let track = catalog.tracks[0].clone();
    let fingerprint = track.versions[0].fingerprint.unwrap();
    edit(
        &mut catalog,
        Edit::AddMembers {
            id: id(1),
            members: vec![track.id.clone()],
            before: None,
        },
    );
    let forest = catalog.crates.clone();
    catalog
        .upsert(
            track.source.clone(),
            Some(fingerprint),
            meta("rescan title"),
        )
        .unwrap();
    assert_eq!(catalog.crates, forest);
    let LibSource::File(path) = &track.source else {
        unreachable!()
    };
    let moved = files.0.join("verified-move.wav");
    fs::copy(path, &moved).unwrap();
    fs::remove_file(path).unwrap();
    catalog.validate().unwrap();
    assert_eq!(
        catalog.crates, forest,
        "missing audio cannot remove a membership"
    );
    catalog
        .relocate(&Relocate {
            id: track.id.clone(),
            source: track.source.clone(),
            fingerprint,
            destination: moved.clone(),
        })
        .unwrap();
    assert_eq!(catalog.crates, forest);
    assert_eq!(
        catalog.track(&LibSource::File(moved.clone())).unwrap().id,
        track.id
    );
    let previous = catalog
        .track(&LibSource::File(moved.clone()))
        .unwrap()
        .versions
        .clone();
    fs::write(&moved, wav(8000, 16000, 1, false)).unwrap();
    let new_fp = FileFingerprint::read(&moved).unwrap();
    catalog
        .upsert(
            LibSource::File(moved.clone()),
            Some(new_fp),
            meta("replacement"),
        )
        .unwrap();
    let current = catalog.track(&LibSource::File(moved)).unwrap();
    assert_eq!(current.id, track.id);
    assert!(current.versions[current.current].analysis.is_none());
    assert_eq!(&current.versions[..previous.len()], previous);
    assert_eq!(catalog.crates, forest);
}
