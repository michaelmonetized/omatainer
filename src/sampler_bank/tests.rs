use super::*;

fn id(value: u8) -> BankId {
    BankId::from_bytes([value; 16]).unwrap()
}
fn bank(value: u8, name: &str) -> Definition {
    Definition {
        id: id(value),
        name: name.into(),
        slots: std::array::from_fn(|_| Slot::default()),
    }
}

#[test]
fn definition_ids_not_names_authorize_replacement_and_import_ids_are_distinct() {
    let mut collection = Collection::default();
    collection.put(bank(1, "same name"), None).unwrap();
    collection.put(bank(2, "same name"), None).unwrap();
    assert_eq!(collection.banks.len(), 2);
    let before = collection.clone();
    assert!(collection.put(bank(1, "renamed"), None).is_err());
    assert!(collection.put(bank(3, "same name"), Some(id(1))).is_err());
    assert_eq!(collection, before);
    collection.put(bank(1, "renamed"), Some(id(1))).unwrap();
    assert_eq!(collection.banks[1].name, "same name");
    let first = BankId::new().unwrap();
    let second = BankId::new().unwrap();
    assert_ne!(first, second);
    assert_ne!(first, id(1));
    assert_eq!(
        serde_json::from_str::<BankId>(&serde_json::to_string(&first).unwrap()).unwrap(),
        first
    );
    for value in [
        "",
        "A0000000000000000000000000000001",
        "00000000000000000000000000000000",
        "é000000000000000000000000000001",
    ] {
        assert!(serde_json::from_value::<BankId>(serde_json::json!(value)).is_err());
    }
    collection.revision = u64::MAX;
    let before = collection.clone();
    assert!(collection.put(bank(1, "overflow"), Some(id(1))).is_err());
    assert_eq!(collection, before);
}

#[test]
fn strict_bounds_and_source_frame_ranges_never_clamp_or_substitute() {
    let mut value = bank(1, "Mixed kit");
    value.slots[15] = Slot {
        source: Some(Source::Factory {
            bank: Factory::Hits,
            slot: 15,
        }),
        controls: Controls {
            gain: 0.5,
            start_seconds: 0.25,
            end_seconds: Some(0.75),
        },
    };
    value.validate().unwrap();
    assert_eq!(
        value.slots[15].controls.frames(16_000, 16_000).unwrap(),
        (4_000.0, 12_000.0)
    );
    assert_eq!(
        value.slots[15].controls.frames(96_000, 96_000).unwrap(),
        (24_000.0, 72_000.0)
    );
    assert!(value.slots[15].controls.frames(16_000, 8_000).is_err());
    assert!(Controls {
        gain: f32::NAN,
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(Controls {
        start_seconds: -0.1,
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(Controls {
        start_seconds: 0.1,
        end_seconds: Some(0.1),
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(Controls {
        end_seconds: Some(f64::INFINITY),
        ..Default::default()
    }
    .validate()
    .is_err());
    assert!(Controls::default().frames(48_000, 0).is_err());
    assert!(Controls::default().frames(0, 48_000).is_err());
    let collection = Collection {
        banks: vec![value],
        ..Default::default()
    };
    let serialized = serde_json::to_value(&collection).unwrap();
    let mut changed = serialized.clone();
    changed["future"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Collection>(changed).is_err());
    let mut changed = serialized.clone();
    changed["schema"] = serde_json::json!(2);
    assert!(serde_json::from_value::<Collection>(changed)
        .unwrap()
        .validate()
        .is_err());
    let mut changed = serialized;
    changed["banks"][0]["slots"][15]["source"]["slot"] = serde_json::json!(16);
    assert!(serde_json::from_value::<Collection>(changed)
        .unwrap()
        .validate()
        .is_err());
    assert!(Collection {
        banks: (0..65).map(|i| bank(i + 1, "bounded")).collect(),
        ..Default::default()
    }
    .validate()
    .is_err());
}

#[test]
fn source_resolution_follows_verified_relocation_but_refuses_replacement_bytes() {
    use std::fs;
    let directory = std::env::temp_dir().join(format!(
        "omatainer-bank-identity-{}",
        BankId::new().unwrap()
    ));
    fs::create_dir(&directory).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let original = directory.join("original.wav");
    let moved = directory.join("moved.wav");
    fs::write(
        &original,
        b"fixed identity fixture bytes, not decoder evidence",
    )
    .unwrap();
    fs::copy(&original, &moved).unwrap();
    let fingerprint = FileFingerprint::read(&original).unwrap();
    let source = LibSource::File(original.clone());
    let metadata = crate::library::Metadata {
        title: "identity fixture".into(),
        artist: String::new(),
        bpm: crate::ui::bpm::Bpm::UNKNOWN,
        key: String::new(),
        duration: None,
        last_play: None,
    };
    let mut catalog = Catalog::default();
    catalog
        .upsert(source.clone(), Some(fingerprint), metadata.clone())
        .unwrap();
    let reference = SourceRef {
        track: catalog.track(&source).unwrap().id.clone(),
        source: source.clone(),
        fingerprint,
        content_hash: None,
    };
    assert_eq!(reference.resolve(&catalog).unwrap(), reference);
    catalog
        .relocate(&crate::library::Relocate {
            id: reference.track.clone(),
            source,
            fingerprint,
            destination: moved.clone(),
        })
        .unwrap();
    fs::remove_file(original).unwrap();
    let resolved = reference.resolve(&catalog).unwrap();
    assert_eq!(resolved.path().unwrap(), moved);
    assert_eq!(resolved.track, reference.track);
    assert!(resolved.content_hash.is_some());
    let mut conflict = reference.clone();
    conflict.content_hash = Some([0; 32]);
    assert!(conflict.resolve(&catalog).is_err());
    fs::write(&moved, b"different replacement source").unwrap();
    catalog
        .upsert(
            LibSource::File(moved.clone()),
            FileFingerprint::read(&moved),
            metadata,
        )
        .unwrap();
    assert!(reference
        .resolve(&catalog)
        .unwrap_err()
        .contains("content changed"));
    let mut invalid = reference;
    invalid.source = LibSource::Builtin(crate::engine::media_source::BuiltinStem::Drums);
    assert!(invalid.validate().is_err());
    assert!(invalid.path().is_err());
}
