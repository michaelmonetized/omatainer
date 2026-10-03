use super::*;
use crate::library::crates::{CrateId, Edit};
use std::sync::atomic::{AtomicU64, Ordering};

struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "omatainer-annotations-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn metadata(index: usize) -> Metadata {
    Metadata {
        title: format!("Track {index}"),
        artist: "Artist".into(),
        bpm: Bpm::UNKNOWN,
        key: String::new(),
        duration: None,
        last_play: None,
    }
}

#[test]
fn ten_thousand_stable_tracks_survive_batch_edit_restart_filter_and_metadata_export() {
    let files = Files::new();
    let path = files.0.join("catalog.json");
    let mut store = Store::open(path.clone()).unwrap();
    for index in 0..10_000 {
        store
            .catalog
            .upsert(
                LibSource::File(files.0.join(format!("missing-{index}.wav"))),
                None,
                metadata(index),
            )
            .unwrap();
    }
    let ids: Vec<_> = store
        .catalog
        .tracks
        .iter()
        .map(|track| track.id.clone())
        .collect();
    let patch = Patch {
        rating: Some(4),
        color: Some(Some([255, 102, 0])),
        group: Some("Peak 日本語".into()),
        tags: Some(vec!["clean".into(), "request".into()]),
        notes: Some("Wedding request\nPlay after dinner".into()),
    };
    assert!(store.catalog.annotate(&ids, &patch).unwrap());
    store.save().unwrap();
    drop(store);
    let reopened = Store::open(path).unwrap();
    let (ordinary, rule) =
        Rule::search("rating>=4 tag:clean color:#FF6600 group:peak note:dinner").unwrap();
    assert!(ordinary.is_empty());
    assert_eq!(
        reopened
            .catalog
            .tracks
            .iter()
            .filter(|track| rule.matches(&track.annotations))
            .count(),
        10_000
    );
    assert_eq!(
        reopened
            .catalog
            .tracks
            .iter()
            .map(|track| track.id.clone())
            .collect::<Vec<_>>(),
        ids
    );
    let export = files.0.join("export.json");
    fs::write(&export, serde_json::to_vec(&reopened.catalog).unwrap()).unwrap();
    let exported = read(&export).unwrap();
    assert_eq!(exported.tracks, reopened.catalog.tracks);
}

#[test]
fn annotation_batches_are_atomic_and_file_replacement_preserves_track_annotations() {
    let files = Files::new();
    let audio = files.0.join("source.wav");
    fs::write(&audio, b"own unchanged audio bytes").unwrap();
    let source = LibSource::File(audio.clone());
    let mut catalog = Catalog::default();
    let first = FileFingerprint::read(&audio).unwrap();
    catalog
        .upsert(source.clone(), Some(first), metadata(1))
        .unwrap();
    let id = catalog.track(&source).unwrap().id.clone();
    let patch = Patch {
        rating: Some(5),
        notes: Some("Requested 日本語\nClean edit".into()),
        ..Patch::default()
    };
    let baseline = catalog.tracks.clone();
    assert!(catalog
        .annotate(&[id.clone(), TrackId("a".repeat(32))], &patch)
        .is_err());
    assert_eq!(catalog.tracks, baseline);
    assert!(catalog.annotate(&[id.clone(), id.clone()], &patch).is_err());
    assert_eq!(catalog.tracks, baseline);
    catalog.annotate(&[id.clone()], &patch).unwrap();
    assert_eq!(fs::read(&audio).unwrap(), b"own unchanged audio bytes");
    fs::write(&audio, b"replacement audio file").unwrap();
    catalog
        .upsert(
            source.clone(),
            Some(FileFingerprint::read(&audio).unwrap()),
            metadata(2),
        )
        .unwrap();
    let track = catalog.track(&source).unwrap();
    assert_eq!(track.id, id);
    assert_eq!(track.annotations.rating, 5);
    assert_eq!(track.annotations.notes, "Requested 日本語\nClean edit");
    let invalid = Patch {
        rating: Some(6),
        ..Patch::default()
    };
    let before = catalog.tracks.clone();
    assert!(catalog.annotate(&[id], &invalid).is_err());
    assert_eq!(catalog.tracks, before);
}

#[test]
fn saved_rules_reopen_and_protect_manual_membership() {
    let files = Files::new();
    let mut store = Store::open(files.0.join("catalog.json")).unwrap();
    let source = LibSource::File(files.0.join("missing.wav"));
    store
        .catalog
        .upsert(source.clone(), None, metadata(0))
        .unwrap();
    let track = store.catalog.track(&source).unwrap().id.clone();
    let id = CrateId("b".repeat(32));
    store
        .catalog
        .edit_crates(
            0,
            &Edit::Create {
                id: id.clone(),
                name: "Top tracks".into(),
                parent: None,
                before: None,
            },
        )
        .unwrap();
    let rule = Rule {
        minimum_rating: 4,
        tag: "clean".into(),
        ..Rule::default()
    };
    store
        .catalog
        .edit_crates(
            1,
            &Edit::SetAnnotationRule {
                id: id.clone(),
                rule: Some(rule.clone()),
            },
        )
        .unwrap();
    assert!(store
        .catalog
        .edit_crates(
            2,
            &Edit::AddMembers {
                id: id.clone(),
                members: vec![track.clone()],
                before: None
            }
        )
        .is_err());
    store
        .catalog
        .annotate(
            &[track.clone()],
            &Patch {
                rating: Some(5),
                tags: Some(vec!["CLEAN".into()]),
                ..Patch::default()
            },
        )
        .unwrap();
    store.save().unwrap();
    let reopened = read(&files.0.join("catalog.json")).unwrap();
    assert!(reopened
        .crates
        .node(&id)
        .unwrap()
        .annotation_rule
        .as_ref()
        .unwrap()
        .matches(&reopened.track(&source).unwrap().annotations));
    store
        .catalog
        .edit_crates(
            2,
            &Edit::SetAnnotationRule {
                id: id.clone(),
                rule: None,
            },
        )
        .unwrap();
    store
        .catalog
        .edit_crates(
            3,
            &Edit::AddMembers {
                id: id.clone(),
                members: vec![track.clone()],
                before: None,
            },
        )
        .unwrap();
    assert!(store
        .catalog
        .edit_crates(
            4,
            &Edit::SetAnnotationRule {
                id: id.clone(),
                rule: Some(rule)
            }
        )
        .is_err());
    assert_eq!(store.catalog.crates.node(&id).unwrap().members, vec![track]);
}

#[test]
fn old_schema_defaults_migrate_and_new_fields_cannot_hide_in_legacy_records() {
    let files = Files::new();
    let path = files.0.join("legacy.json");
    let mut catalog = Catalog::default();
    catalog
        .upsert(
            LibSource::File(files.0.join("missing.wav")),
            None,
            metadata(0),
        )
        .unwrap();
    let mut legacy = serde_json::to_value(&catalog).unwrap();
    legacy["schema"] = 8.into();
    fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let loaded = read(&path).unwrap();
    assert_eq!(loaded.schema, 9);
    assert!(loaded.tracks[0].annotations.is_empty());
    legacy["tracks"][0]["annotations"] = serde_json::to_value(Annotations::default()).unwrap();
    let bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&path, &bytes).unwrap();
    assert!(read(&path)
        .unwrap_err()
        .contains("require library schema 9"));
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn bounded_international_fields_and_query_errors_are_explicit() {
    let fields = Annotations {
        rating: 4,
        group: "日本語 Peak".into(),
        tags: vec!["Clean".into()],
        notes: "Requested\nDinner".into(),
        color: Some([255, 102, 0]),
    };
    fields.validate().unwrap();
    let (_, rule) = Rule::search("rating>=4 tag:clean color:#FF6600").unwrap();
    assert!(rule.matches(&fields));
    for query in [
        "rating>=6",
        "rating>=bad",
        "tag:",
        "tag:a tag:b",
        "color:#xyz",
        "note:",
    ] {
        assert!(Rule::search(query).is_err(), "{query}");
    }
    assert_eq!(Rule::search("Mixed  Space").unwrap().0, "Mixed  Space");
    assert!(Annotations {
        tags: vec!["clean".into(), "CLEAN".into()],
        ..fields.clone()
    }
    .validate()
    .is_err());
    assert!(Annotations {
        notes: "bad\0note".into(),
        ..fields
    }
    .validate()
    .is_err());
}

#[test]
fn malformed_reserved_predicates_are_errors_and_plain_field_names_remain_searchable() {
    for query in [
        "color:red",
        "color:",
        "rating>4",
        "rating<4",
        "rating=4",
        "rating:4",
        "rating!=4",
        "rating>=bad",
    ] {
        assert!(Rule::search(query).is_err(), "{query}");
    }
    for query in ["rating", "color", "colorful track", "rating day"] {
        assert_eq!(
            Rule::search(query).unwrap(),
            (query.into(), Rule::default())
        );
    }
    let (_, rule) = Rule::search("rating>=4 color:#FF6600 tag:clean").unwrap();
    assert_eq!(rule.minimum_rating, 4);
    assert_eq!(rule.color, Some([255, 102, 0]));
}
