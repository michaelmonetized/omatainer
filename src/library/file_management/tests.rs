use super::*;
use crate::engine::media_analysis::tests::{wav, Files};
use crate::library::crates::{CrateId, Edit};

fn fixture(files: &Files) -> Catalog {
    let mut catalog = Catalog::default();
    for index in 0..3 {
        let proof = files.source(&format!("track{index}.wav"), &wav(8000, 8000, 1, false));
        let version = catalog
            .upsert(
                proof.source.clone(),
                Some(proof.fingerprint),
                Metadata {
                    title: format!("Track {index}"),
                    artist: "Retained artist".into(),
                    bpm: Bpm::new(127.5, Origin::User),
                    key: "F#m".into(),
                    duration: Some(1.0),
                    last_play: None,
                },
            )
            .unwrap();
        version.preparation.cue = index as f64 / 10.0;
        version.preparation.hotcues[7] = Some(0.75 - index as f64 / 10.0);
        let LibSource::File(path) = &proof.source else {
            panic!("Local fixture source")
        };
        version.content_hash = Some(content::hash_file(path, proof.fingerprint, || true).unwrap());
    }
    let crate_id = CrateId(format!("{:032x}", 7));
    catalog
        .edit_crates(
            0,
            &Edit::Create {
                id: crate_id.clone(),
                name: "Keep order".into(),
                parent: None,
                before: None,
            },
        )
        .unwrap();
    let members = catalog
        .tracks
        .iter()
        .map(|track| track.id.clone())
        .collect();
    catalog
        .edit_crates(
            catalog.crates.revision(),
            &Edit::AddMembers {
                id: crate_id.clone(),
                members,
                before: None,
            },
        )
        .unwrap();
    let imported = files.0.join("original.m3u");
    fs::write(&imported, "#EXTM3U\ntrack0.wav\ntrack1.wav\ntrack2.wav\n").unwrap();
    catalog.imports.push(imports::Collection {
        source: LibSource::File(imported),
        digest: [7; 32],
        format: "m3u".into(),
        key: "original playlist".into(),
        crate_id,
        references: catalog
            .tracks
            .iter()
            .map(|track| imports::Reference {
                reference: track.id.0.clone(),
                track: Some(track.id.clone()),
                details: std::sync::Arc::new(imports::Details {
                    title: format!("Original {}", track.id.0),
                    ..Default::default()
                }),
            })
            .collect(),
    });
    catalog.validate().unwrap();
    catalog
}
fn targets(catalog: &Catalog, indices: &[usize]) -> Vec<Target> {
    indices
        .iter()
        .map(|&index| Target::capture(&catalog.tracks[index]))
        .collect()
}
fn source_bytes(catalog: &Catalog) -> Vec<Vec<u8>> {
    catalog
        .tracks
        .iter()
        .map(|track| {
            let LibSource::File(path) = &track.source else {
                panic!("Local source")
            };
            fs::read(path).unwrap()
        })
        .collect()
}
fn wire(catalog: &Catalog) -> Vec<u8> {
    serde_json::to_vec(catalog).unwrap()
}

#[test]
fn reviewed_reference_removal_preserves_source_audio_order_import_provenance_and_unselected_preparation(
) {
    let files = Files::new();
    let original = fixture(&files);
    let original_bytes = source_bytes(&original);
    let removed = original
        .remove_file_references(&targets(&original, &[1]), original.crates.revision())
        .unwrap();
    assert_eq!(
        removed.tracks,
        vec![original.tracks[0].clone(), original.tracks[2].clone()]
    );
    assert_eq!(
        removed.crates.nodes()[0].members,
        vec![original.tracks[0].id.clone(), original.tracks[2].id.clone()]
    );
    assert_eq!(removed.crates.revision(), original.crates.revision() + 1);
    assert_eq!(removed.imports[0].references[1].track, None);
    assert_eq!(
        removed.imports[0].references[1].details,
        original.imports[0].references[1].details
    );
    assert_eq!(source_bytes(&original), original_bytes);
    let path = files.0.join("references.json");
    let mut store = Store::open(path.clone()).unwrap();
    store.catalog = removed;
    store.save().unwrap();
    drop(store);
    let reopened = read(&path).unwrap();
    assert_eq!(
        reopened.tracks,
        vec![original.tracks[0].clone(), original.tracks[2].clone()]
    );
    assert_eq!(source_bytes(&original), original_bytes);
    let mut stale = targets(&original, &[1]);
    let current = stale[0].track.current;
    stale[0].track.versions[current].preparation.cue = 0.33;
    assert!(original
        .remove_file_references(&stale, original.crates.revision())
        .is_err());
    assert!(original
        .remove_file_references(&targets(&original, &[1]), original.crates.revision() + 1)
        .is_err());
    println!(
        "LIBRARY_FILES_REFERENCE {}",
        serde_json::json!({"actual_filesystem":true,"reference_only":true,"source_bytes_untouched":true,"unselected_preparation_preserved":true,"import_provenance_retained":true,"save_reopen":true,"stale_refusal":true,"physical_devices_opened":false})
    );
}

#[test]
fn exact_duplicate_merge_retains_both_prepared_versions_stable_identity_and_first_membership_position(
) {
    let files = Files::new();
    let original = fixture(&files);
    let before = source_bytes(&original);
    let a = original.tracks[0].clone();
    let b = original.tracks[1].clone();
    let candidates = likely_duplicates(&original, &a.id).unwrap();
    assert!(candidates
        .iter()
        .any(|candidate| candidate.first == a.id && candidate.second == b.id));
    let merged = original
        .merge_file_duplicates(
            &targets(&original, &[0, 1]),
            original.crates.revision(),
            &a.id,
            &b.id,
            &AtomicBool::new(false),
        )
        .unwrap();
    let kept = merged.tracks.iter().find(|track| track.id == a.id).unwrap();
    assert_eq!(kept.source, b.source);
    assert_eq!(kept.versions[kept.current], b.versions[b.current]);
    assert_eq!(
        merged.version(&a.source, a.versions[a.current].fingerprint),
        Some(&a.versions[a.current])
    );
    assert_eq!(
        merged.version(&b.source, b.versions[b.current].fingerprint),
        Some(&b.versions[b.current])
    );
    assert_eq!(merged.tracks[1], original.tracks[2]);
    assert_eq!(
        merged.crates.nodes()[0].members,
        vec![a.id.clone(), original.tracks[2].id.clone()]
    );
    assert_eq!(merged.imports[0].references[1].track, Some(a.id.clone()));
    assert_eq!(source_bytes(&original), before);
    let path = files.0.join("merged.json");
    let mut store = Store::open(path.clone()).unwrap();
    store.catalog = merged;
    store.save().unwrap();
    drop(store);
    let reopened = read(&path).unwrap();
    assert_eq!(
        reopened.version(&a.source, a.versions[a.current].fingerprint),
        Some(&a.versions[a.current])
    );
    assert_eq!(
        reopened.version(&b.source, b.versions[b.current].fingerprint),
        Some(&b.versions[b.current])
    );
    assert_eq!(source_bytes(&original), before);
    println!(
        "LIBRARY_FILES_DUPLICATE {}",
        serde_json::json!({"actual_byte_verification":true,"both_prepared_versions_preserved":true,"explicit_identity_and_preparation":true,"memberships_and_imports_remapped":true,"source_audio_untouched":true,"save_reopen":true,"physical_devices_opened":false})
    );
}

#[test]
fn verified_copy_adoption_keeps_stable_ids_preparation_prior_locations_and_survives_catalog_reopen()
{
    let files = Files::new();
    let original = fixture(&files);
    let original_bytes = source_bytes(&original);
    let destination = files.0.join("copies");
    fs::create_dir(&destination).unwrap();
    let selected = targets(&original, &[0, 1]);
    let cancel = AtomicBool::new(false);
    let mut staged = transfer::stage(&original, &selected, &destination, &cancel).unwrap();
    let installed = staged.install(&cancel).unwrap();
    let candidate = original
        .adopt_file_copies(&installed, original.crates.revision(), &cancel)
        .unwrap();
    for (index, copy) in installed.iter().enumerate() {
        let track = candidate
            .track(&LibSource::File(copy.destination.clone()))
            .unwrap();
        assert_eq!(track.id, original.tracks[index].id);
        assert_eq!(
            track.versions[track.current].preparation,
            original.tracks[index].versions[original.tracks[index].current].preparation
        );
        assert_eq!(fs::read(&copy.destination).unwrap(), original_bytes[index]);
        assert!(track
            .previous_locations
            .iter()
            .any(|old| old.source == original.tracks[index].source));
    }
    assert_eq!(candidate.tracks[2], original.tracks[2]);
    assert_eq!(source_bytes(&original), original_bytes);
    let path = files.0.join("copied.json");
    let mut store = Store::open(path.clone()).unwrap();
    store.catalog = candidate;
    store.save().unwrap();
    staged.retain_installed().unwrap();
    drop(staged);
    drop(store);
    let reopened = read(&path).unwrap();
    for (index, copy) in installed.iter().enumerate() {
        let track = reopened
            .track(&LibSource::File(copy.destination.clone()))
            .unwrap();
        assert_eq!(track.id, original.tracks[index].id);
        assert_eq!(
            track.versions[track.current].preparation,
            original.tracks[index].versions[original.tracks[index].current].preparation
        );
        assert_eq!(fs::read(&copy.destination).unwrap(), original_bytes[index]);
    }
    assert_eq!(source_bytes(&original), original_bytes);
    println!(
        "LIBRARY_FILES_COPY {}",
        serde_json::json!({"actual_descriptor_bound_copy":true,"independent_written_byte_verification":true,"atomic_no_overwrite":true,"stable_id_and_preparation":true,"originals_preserved":true,"save_reopen":true,"physical_devices_opened":false})
    );
}

#[test]
fn cancelled_stale_different_bytes_and_conflicting_names_refuse_without_overwriting_or_deleting_unselected_files(
) {
    let files = Files::new();
    let mut catalog = fixture(&files);
    let original_wire = wire(&catalog);
    let destination = files.0.join("copies");
    fs::create_dir(&destination).unwrap();
    assert!(transfer::stage(
        &catalog,
        &targets(&catalog, &[0, 1]),
        &destination,
        &AtomicBool::new(true)
    )
    .is_err());
    assert!(fs::read_dir(&destination).unwrap().next().is_none());
    let sentinel = destination.join("track1.wav");
    fs::write(&sentinel, b"unselected destination").unwrap();
    let sentinel_fp = FileFingerprint::read(&sentinel).unwrap();
    assert!(transfer::stage(
        &catalog,
        &targets(&catalog, &[0, 1]),
        &destination,
        &AtomicBool::new(false)
    )
    .is_err());
    assert_eq!(FileFingerprint::read(&sentinel), Some(sentinel_fp));
    assert_eq!(fs::read(&sentinel).unwrap(), b"unselected destination");
    assert_eq!(wire(&catalog), original_wire);
    let LibSource::File(changed) = catalog.tracks[1].source.clone() else {
        panic!("Local fixture")
    };
    fs::write(&changed, b"different selected bytes").unwrap();
    let fp = FileFingerprint::read(&changed).unwrap();
    catalog
        .upsert(
            LibSource::File(changed),
            Some(fp),
            catalog.tracks[1].versions[catalog.tracks[1].current]
                .metadata
                .clone(),
        )
        .unwrap();
    let before = wire(&catalog);
    assert!(catalog
        .merge_file_duplicates(
            &targets(&catalog, &[0, 1]),
            catalog.crates.revision(),
            &catalog.tracks[0].id,
            &catalog.tracks[0].id,
            &AtomicBool::new(false)
        )
        .err()
        .unwrap()
        .contains("different bytes"));
    assert_eq!(wire(&catalog), before);
    let other = files.0.join("other");
    fs::create_dir(&other).unwrap();
    let same = other.join("track0.wav");
    fs::write(&same, b"unselected same filename").unwrap();
    catalog
        .upsert(
            LibSource::File(same.clone()),
            FileFingerprint::read(&same),
            catalog.tracks[0].versions[catalog.tracks[0].current]
                .metadata
                .clone(),
        )
        .unwrap();
    assert!(transfer::stage(
        &catalog,
        &targets(&catalog, &[0, 3]),
        &destination,
        &AtomicBool::new(false)
    )
    .err()
    .unwrap()
    .contains("share a filename"));
    assert_eq!(fs::read(&same).unwrap(), b"unselected same filename");
    assert_eq!(fs::read(&sentinel).unwrap(), b"unselected destination");
    println!(
        "LIBRARY_FILES_REFUSAL {}",
        serde_json::json!({"actual_filesystem":true,"cancellation":true,"stale_selection":true,"different_bytes_refused":true,"duplicate_filenames_refused":true,"unselected_bytes_preserved":true,"physical_devices_opened":false})
    );
}

#[test]
fn actual_mid_install_collision_rolls_back_only_owned_copies_and_preserves_external_destination_and_catalog(
) {
    let files = Files::new();
    let original = fixture(&files);
    let before = wire(&original);
    let original_bytes = source_bytes(&original);
    let destination = files.0.join("copies");
    fs::create_dir(&destination).unwrap();
    let cancel = AtomicBool::new(false);
    let mut staged = transfer::stage(
        &original,
        &targets(&original, &[0, 1]),
        &destination,
        &cancel,
    )
    .unwrap();
    let sentinel = destination.join("track1.wav");
    let error = staged
        .install_for_test(&cancel, |index| {
            if index == 1 {
                assert!(destination.join("track0.wav").exists());
                fs::write(&sentinel, b"external unselected file").unwrap();
            }
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("not overwritten"), "{error}");
    drop(staged);
    assert!(!destination.join("track0.wav").exists());
    assert_eq!(fs::read(&sentinel).unwrap(), b"external unselected file");
    assert_eq!(fs::read_dir(&destination).unwrap().count(), 1);
    assert_eq!(wire(&original), before);
    assert_eq!(source_bytes(&original), original_bytes);
    println!(
        "LIBRARY_FILES_PARTIAL_COPY {}",
        serde_json::json!({"actual_atomic_install_collision":true,"first_copy_installed_then_owned_rollback":true,"external_destination_preserved":true,"catalog_and_original_audio_unchanged":true,"physical_devices_opened":false})
    );
}
