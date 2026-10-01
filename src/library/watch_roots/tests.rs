use super::*;

fn binding(profile: &str, path: &str, uuid: &str) -> Binding {
    Binding {
        profile: profile.into(),
        configured: path.into(),
        source: LibSource::Removable {
            volume_id: uuid.into(),
            relative_path: PathBuf::new(),
        },
    }
}
fn batch(book: &Book, profile: &str, configured: &[&str], observed: Vec<Binding>) -> Batch {
    Batch {
        expected: book.revision(),
        profile: profile.into(),
        configured: configured.iter().map(PathBuf::from).collect(),
        observed,
    }
}
#[test]
fn offline_bindings_survive_and_root_removal_preserves_other_profiles() {
    let mut book = Book::default();
    let a = binding("Studio", "/mnt/old", "AAAA-1111");
    book.apply(&batch(&book, "Studio", &["/mnt/old"], vec![a.clone()]))
        .unwrap();
    let revision = book.revision();
    assert!(!book
        .apply(&batch(&book, "Studio", &["/mnt/old"], vec![]))
        .unwrap());
    assert_eq!(book.binding("Studio", Path::new("/mnt/old")), Some(&a));
    assert_eq!(book.revision(), revision);
    let b = binding("Performance", "/mnt/dj", "BBBB-2222");
    book.apply(&batch(&book, "Performance", &["/mnt/dj"], vec![b.clone()]))
        .unwrap();
    book.apply(&batch(&book, "Studio", &[], vec![])).unwrap();
    assert!(book.binding("Studio", Path::new("/mnt/old")).is_none());
    assert_eq!(book.binding("Performance", Path::new("/mnt/dj")), Some(&b));
    let bytes = serde_json::to_vec(&book).unwrap();
    assert_eq!(serde_json::from_slice::<Book>(&bytes).unwrap(), book);
}
#[test]
fn stale_or_foreign_observations_and_volume_rebinding_are_inert() {
    let mut book = Book::default();
    let first = batch(
        &book,
        "Studio",
        &["/mnt/a"],
        vec![binding("Studio", "/mnt/a", "AAAA-1111")],
    );
    book.apply(&first).unwrap();
    let before = book.clone();
    assert!(book.matches(&first));
    assert!(book.apply(&first).is_err());
    assert_eq!(book, before);
    for observed in [
        binding("Studio", "/mnt/a", "BBBB-2222"),
        binding("Other", "/mnt/a", "AAAA-1111"),
        binding("Studio", "/mnt/unselected", "AAAA-1111"),
    ] {
        assert!(book
            .apply(&batch(&book, "Studio", &["/mnt/a"], vec![observed]))
            .is_err());
        assert_eq!(book, before);
    }
    // Explicit removal, persisted first, permits a deliberate later enrollment.
    book.apply(&batch(&book, "Studio", &[], vec![])).unwrap();
    book.apply(&batch(
        &book,
        "Studio",
        &["/mnt/a"],
        vec![binding("Studio", "/mnt/a", "BBBB-2222")],
    ))
    .unwrap();
}
#[test]
fn local_path_bookmark_can_upgrade_to_observed_volume_identity() {
    let mut book = Book::default();
    let local = Binding {
        profile: "Studio".into(),
        configured: "/mnt/a".into(),
        source: LibSource::File("/mnt/a".into()),
    };
    book.apply(&batch(&book, "Studio", &["/mnt/a"], vec![local]))
        .unwrap();
    let volume = binding("Studio", "/mnt/a", "AAAA-1111");
    assert!(book
        .apply(&batch(&book, "Studio", &["/mnt/a"], vec![volume.clone()]))
        .unwrap());
    assert_eq!(book.binding("Studio", Path::new("/mnt/a")), Some(&volume));
}
#[test]
fn malformed_batch_and_exhaustion_preserve_original() {
    let mut book = Book::default();
    let before = book.clone();
    let mut invalid = batch(
        &book,
        "Studio",
        &["/mnt/a"],
        vec![binding("Studio", "/mnt/a", "AAAA-1111")],
    );
    invalid.observed[0].source = LibSource::Provider {
        provider: "other".into(),
        media_id: "1".into(),
    };
    assert!(book.apply(&invalid).is_err());
    assert_eq!(book, before);
    assert!(book
        .apply(&batch(&book, "Studio", &["/mnt/a", "/mnt/a"], vec![]))
        .is_err());
    assert_eq!(book, before);
    book.revision = u64::MAX;
    let before = book.clone();
    assert!(book
        .apply(&batch(
            &book,
            "Studio",
            &["/mnt/a"],
            vec![binding("Studio", "/mnt/a", "AAAA-1111")]
        ))
        .is_err());
    assert_eq!(book, before);
}
#[test]
fn catalog_import_never_activates_foreign_root_bookmarks() {
    let mut own = Catalog::default();
    let mut foreign = Catalog::default();
    foreign
        .watched_roots
        .apply(&batch(
            &foreign.watched_roots,
            "Studio",
            &["/mnt/foreign"],
            vec![binding("Studio", "/mnt/foreign", "AAAA-1111")],
        ))
        .unwrap();
    own.merge_import(foreign).unwrap();
    assert_eq!(own.watched_roots, Book::default());
}
#[test]
fn schema_six_migration_preserves_tracks_crates_and_original_bytes() {
    let root = std::env::temp_dir().join(format!(
        "omat-roots-migration-{}",
        crate::performance_history::storage::new_id().unwrap()
    ));
    std::fs::create_dir(&root).unwrap();
    let path = root.join("catalog.json");
    let mut old = Catalog::default();
    old.upsert(
        LibSource::Builtin(crate::engine::media_source::BuiltinStem::Drums),
        None,
        Metadata {
            title: "Prepared built-in".into(),
            artist: "Fixture".into(),
            bpm: Bpm::UNKNOWN,
            key: "Am".into(),
            duration: Some(2.0),
            last_play: Some(SystemTime::UNIX_EPOCH),
        },
    )
    .unwrap()
    .preparation
    .cue = 0.25;
    old.edit_crates(
        old.crates.revision(),
        &crates::Edit::Create {
            id: crates::CrateId("b".repeat(32)),
            name: "Preserved set".into(),
            parent: None,
            before: None,
        },
    )
    .unwrap();
    let mut value = serde_json::to_value(&old).unwrap();
    value["schema"] = 6.into();
    value.as_object_mut().unwrap().remove("watched_roots");
    let bytes = serde_json::to_vec(&value).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let migrated = read(&path).unwrap();
    assert_eq!(migrated.schema, SCHEMA);
    assert_eq!(migrated.watched_roots, Book::default());
    assert_eq!(migrated.tracks, old.tracks);
    assert_eq!(
        serde_json::to_value(migrated.crates).unwrap(),
        serde_json::to_value(old.crates).unwrap()
    );
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    value["watched_roots"] = serde_json::json!({"revision":0,"bindings":[]});
    std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(
        read(&path).is_err(),
        "old schemas cannot smuggle new root fields"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn namespace_enrollment_preserves_track_and_only_verified_identical_content_restores_cues() {
    use crate::engine::media_analysis::tests::{wav, Files};
    let files = Files::new();
    let reference = files.source("old.wav", &wav(8000, 8000, 1, false));
    let mut catalog = Catalog::default();
    let metadata = Metadata {
        title: "Saved title".into(),
        artist: "Artist".into(),
        bpm: Bpm::new(125., Origin::User),
        key: "Am".into(),
        duration: Some(1.),
        last_play: Some(SystemTime::UNIX_EPOCH),
    };
    let version = catalog
        .upsert(
            reference.source.clone(),
            Some(reference.fingerprint),
            metadata.clone(),
        )
        .unwrap();
    version.preparation.hotcues[0] = Some(0.25);
    version.content_hash = Some([7; 32]);
    let preparation = version.preparation;
    let track = catalog.track(&reference.source).unwrap().id.clone();
    let volume = LibSource::Removable {
        volume_id: "TEST-A".into(),
        relative_path: "music/old.wav".into(),
    };
    let adoption = Adoption {
        old: reference.source.clone(),
        expected: reference.fingerprint,
        source: volume.clone(),
    };
    catalog.adopt_volume(&adoption).unwrap();
    assert_eq!(catalog.track(&volume).unwrap().id, track);
    assert_eq!(
        catalog
            .version(&reference.source, Some(reference.fingerprint))
            .unwrap()
            .preparation,
        preparation
    );
    // A remount changes its transient file fingerprint. No path/UUID alone
    // transfers cues or last-play data to the newly observed version.
    let new = files.source("new.wav", &wav(8000, 8000, 1, false));
    catalog
        .upsert(
            volume.clone(),
            Some(new.fingerprint),
            Metadata {
                bpm: Bpm::UNKNOWN,
                duration: None,
                last_play: None,
                ..metadata.clone()
            },
        )
        .unwrap();
    assert_eq!(catalog.track(&volume).unwrap().versions.len(), 2);
    assert_eq!(catalog.track(&volume).unwrap().id, track);
    assert_eq!(
        catalog
            .version(&volume, Some(new.fingerprint))
            .unwrap()
            .preparation,
        Preparation::default()
    );
    assert!(catalog
        .version(&volume, Some(new.fingerprint))
        .unwrap()
        .metadata
        .last_play
        .is_none());
    assert_eq!(
        catalog.preparation_for_content(&volume, new.fingerprint, [8; 32]),
        None
    );
    assert_eq!(
        catalog.preparation_for_content(&volume, new.fingerprint, [7; 32]),
        Some(preparation)
    );
    catalog
        .qualify_verified_content(&track, &volume, new.fingerprint, [7; 32])
        .unwrap();
    let restored = catalog.version(&volume, Some(new.fingerprint)).unwrap();
    assert_eq!(restored.preparation, preparation);
    assert_eq!(restored.metadata.last_play, metadata.last_play);
    let saved = serde_json::to_vec(&catalog).unwrap();
    assert!(catalog
        .qualify_verified_content(&track, &volume, new.fingerprint, [8; 32])
        .is_err());
    assert_eq!(serde_json::to_vec(&catalog).unwrap(), saved);
    assert!(catalog
        .preparation_for_content(&volume, new.fingerprint, [8; 32])
        .is_none());
    let old = crate::sampler_bank::SourceRef {
        track,
        source: reference.source,
        fingerprint: reference.fingerprint,
        content_hash: Some([7; 32]),
    };
    let resolved = old.resolve(&catalog).unwrap();
    assert_eq!(resolved.source, volume);
    assert_eq!(resolved.fingerprint, new.fingerprint);
}

#[test]
fn stale_or_conflicting_namespace_enrollment_leaves_complete_catalog_unchanged() {
    use crate::engine::media_analysis::tests::{wav, Files};
    let files = Files::new();
    let first = files.source("a.wav", &wav(8000, 100, 1, false));
    let next = files.source("b.wav", &wav(8000, 100, 1, false));
    let metadata = Metadata {
        title: "A".into(),
        artist: String::new(),
        bpm: Bpm::UNKNOWN,
        key: String::new(),
        duration: None,
        last_play: None,
    };
    let mut catalog = Catalog::default();
    catalog
        .upsert(
            first.source.clone(),
            Some(first.fingerprint),
            metadata.clone(),
        )
        .unwrap();
    let source = LibSource::Removable {
        volume_id: "TEST-A".into(),
        relative_path: "a.wav".into(),
    };
    let adoption = Adoption {
        old: first.source.clone(),
        expected: first.fingerprint,
        source: source.clone(),
    };
    catalog
        .upsert(
            first.source.clone(),
            Some(next.fingerprint),
            metadata.clone(),
        )
        .unwrap();
    let before = serde_json::to_vec(&catalog).unwrap();
    assert!(catalog.adopt_volume(&adoption).is_err());
    assert_eq!(serde_json::to_vec(&catalog).unwrap(), before);
    catalog
        .upsert(first.source, Some(first.fingerprint), metadata.clone())
        .unwrap();
    catalog
        .upsert(source, Some(next.fingerprint), metadata)
        .unwrap();
    let before = serde_json::to_vec(&catalog).unwrap();
    assert!(catalog.adopt_volume(&adoption).is_err());
    assert_eq!(serde_json::to_vec(&catalog).unwrap(), before);
}
