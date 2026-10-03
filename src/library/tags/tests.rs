use super::*;
use crate::media_tags::{Field, TagSource};
use lofty::file::FileType;
use std::sync::atomic::{AtomicU64, Ordering};

// Modeled catalog identities; guarded real-media measurements are covered by
// media_tags and filesystem-worker tests. These tests never claim source I/O.
fn fingerprint(inode: u64) -> FileFingerprint {
    serde_json::from_value(serde_json::json!({"device":7,"inode":inode,"length":4096,
        "modified":[123,456],"changed":[123,inode]}))
    .unwrap()
}
fn metadata() -> Metadata {
    Metadata {
        title: "filename title".into(),
        artist: "filename artist".into(),
        bpm: Bpm::hint(96.0),
        key: "Am".into(),
        duration: None,
        last_play: None,
    }
}
fn fixture() -> (Catalog, Review) {
    let mut catalog = Catalog::default();
    let source = LibSource::File("/virtual/tags.wav".into());
    catalog
        .upsert(source.clone(), Some(fingerprint(1)), metadata())
        .unwrap();
    let review = Review {
        id: catalog.track(&source).unwrap().id.clone(),
        source,
        fingerprint: fingerprint(1),
    };
    (catalog, review)
}
fn field(value: &str) -> Option<Field> {
    Some(Field {
        value: value.into(),
        source: TagSource::Id3v2,
    })
}
fn observation(fingerprint: FileFingerprint) -> Observation {
    Observation {
        fingerprint,
        format: FileType::Wav,
        fields: Fields {
            title: field("雪の音"),
            artist: field("Björk"),
            bpm: field("126.75"),
            key: field("F#m"),
        },
        notices: vec![],
        rewrite_eligible: true,
    }
}
fn version<'a>(catalog: &'a Catalog, review: &Review) -> &'a Version {
    catalog
        .version(&review.source, Some(review.fingerprint))
        .unwrap()
}
fn analysis(catalog: &mut Catalog, review: &Review, bpm: Option<f32>) {
    catalog
        .apply_analysis(&crate::track_analysis::Patch {
            reference: crate::sampler_bank::SourceRef {
                track: review.id.clone(),
                source: review.source.clone(),
                fingerprint: review.fingerprint,
                content_hash: Some([1; 32]),
            },
            fields: crate::track_analysis::Fields {
                bpm: true,
                duration: true,
                waveform: false,
            },
            at_unix_ms: 123,
            bpm,
            duration: 12.5,
            waveform: None,
        })
        .unwrap();
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Dir(PathBuf);
impl Dir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "omatainer-tags109-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf {
        self.0.join("library.json")
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn loader_tempo_remains_below_embedded_tags_and_explicit_user_clears() {
    let (mut catalog, review) = fixture();
    catalog
        .observe_tags(&review, &observation(review.fingerprint))
        .unwrap();
    catalog
        .observe_loader_bpm(&review, Bpm::new(140.0, Origin::Heuristic))
        .unwrap();
    assert_eq!(
        version(&catalog, &review).metadata.bpm.origin,
        Origin::EmbeddedTag
    );
    let mut empty = observation(review.fingerprint);
    empty.fields.bpm = None;
    catalog.observe_tags(&review, &empty).unwrap();
    assert_eq!(
        version(&catalog, &review).metadata.bpm,
        Bpm::new(140.0, Origin::Heuristic)
    );
    catalog.observe_loader_bpm(&review, Bpm::UNKNOWN).unwrap();
    assert_eq!(
        version(&catalog, &review).metadata.bpm.origin,
        Origin::FilenameHint
    );
    catalog
        .apply_tag_sidecar(
            &review,
            &Patch {
                bpm: Some("".into()),
                ..Patch::default()
            },
        )
        .unwrap();
    catalog
        .observe_loader_bpm(&review, Bpm::new(150.0, Origin::Heuristic))
        .unwrap();
    assert_eq!(version(&catalog, &review).metadata.bpm, Bpm::USER_CLEARED);
    let before = catalog.clone();
    assert!(catalog
        .observe_loader_bpm(&review, Bpm::new(120.0, Origin::User))
        .is_err());
    assert_eq!(
        serde_json::to_vec(&catalog).unwrap(),
        serde_json::to_vec(&before).unwrap()
    );
}

#[test]
fn embedded_observations_win_filename_analysis_and_user_clears_survive_restart() {
    let (mut catalog, review) = fixture();
    analysis(&mut catalog, &review, Some(150.0));
    catalog
        .observe_tags_with_fallback(&review, &observation(review.fingerprint), &metadata())
        .unwrap();
    assert_eq!(
        version(&catalog, &review).metadata.bpm,
        Bpm::new(126.75, Origin::EmbeddedTag)
    );
    assert_eq!(version(&catalog, &review).metadata.title, "雪の音");
    assert_eq!(
        version(&catalog, &review)
            .tags
            .as_ref()
            .unwrap()
            .title_source(),
        "ID3v2"
    );
    analysis(&mut catalog, &review, None);
    catalog
        .upsert(review.source.clone(), Some(review.fingerprint), metadata())
        .unwrap();
    assert_eq!(
        version(&catalog, &review).metadata.bpm,
        Bpm::new(126.75, Origin::EmbeddedTag)
    );
    catalog
        .apply_tag_sidecar(
            &review,
            &Patch {
                title: Some("".into()),
                artist: Some("Ólafur".into()),
                bpm: Some("".into()),
                key: Some("".into()),
            },
        )
        .unwrap();
    for _ in 0..2 {
        catalog
            .observe_tags_with_fallback(&review, &observation(review.fingerprint), &metadata())
            .unwrap();
        analysis(&mut catalog, &review, Some(180.0));
        catalog
            .upsert(review.source.clone(), Some(review.fingerprint), metadata())
            .unwrap();
        let current = version(&catalog, &review);
        assert_eq!(current.metadata.title, "");
        assert_eq!(current.metadata.artist, "Ólafur");
        assert_eq!(current.metadata.key, "");
        assert_eq!(current.metadata.bpm, Bpm::USER_CLEARED);
        assert_eq!(current.metadata.bpm.cell(), "— user cleared");
    }
    let dir = Dir::new();
    {
        let mut store = Store::open(dir.path()).unwrap();
        store.catalog = catalog.clone();
        store.save().unwrap();
    }
    let reopened = Store::open(dir.path()).unwrap();
    assert_eq!(reopened.catalog.tracks, catalog.tracks);
    assert_eq!(reopened.catalog.schema, Catalog::default().schema);
}

#[test]
fn fresh_observation_removal_uses_captured_filename_and_recorded_analysis() {
    let (mut catalog, review) = fixture();
    catalog
        .observe_tags_with_fallback(&review, &observation(review.fingerprint), &metadata())
        .unwrap();
    analysis(&mut catalog, &review, Some(151.5));
    let mut empty = observation(review.fingerprint);
    empty.fields = Fields::default();
    catalog.observe_tags(&review, &empty).unwrap();
    let current = version(&catalog, &review);
    assert_eq!(current.metadata.title, "filename title");
    assert_eq!(current.metadata.artist, "filename artist");
    assert_eq!(current.metadata.bpm, Bpm::new(151.5, Origin::Heuristic));
    assert_eq!(
        current.tags.as_ref().unwrap().title_source(),
        "filename/catalog fallback"
    );
    catalog
        .observe_tags(&review, &observation(review.fingerprint))
        .unwrap();
    catalog
        .observe_tag_failure_with_fallback(
            &review,
            &format!("broken\n{}", "雪".repeat(5000)),
            &metadata(),
        )
        .unwrap();
    let tags = version(&catalog, &review).tags.as_ref().unwrap();
    assert_eq!(version(&catalog, &review).metadata.title, "雪の音");
    assert!(!tags.rewrite_eligible);
    assert_eq!(tags.notices.len(), 1);
    assert!(tags.valid());
    assert!(tags.notices[0].starts_with("Tag inspection failed: broken "));
}

#[test]
fn invalid_edits_and_stale_reviews_leave_catalog_unchanged() {
    let (mut catalog, review) = fixture();
    let original = serde_json::to_vec(&catalog).unwrap();
    for value in ["NaN", "inf", "1", "-2", "bad", "\n120"] {
        assert!(catalog
            .apply_tag_sidecar(
                &review,
                &Patch {
                    bpm: Some(value.into()),
                    ..Default::default()
                }
            )
            .is_err());
    }
    for value in ["x".repeat(4097), "bad\0title".into()] {
        assert!(catalog
            .apply_tag_sidecar(
                &review,
                &Patch {
                    title: Some(value),
                    ..Default::default()
                }
            )
            .is_err());
    }
    assert!(catalog
        .apply_tag_sidecar(&review, &Patch::default())
        .is_err());
    for stale in [
        Review {
            id: TrackId("f".repeat(32)),
            ..review.clone()
        },
        Review {
            source: LibSource::File("/elsewhere.wav".into()),
            ..review.clone()
        },
        Review {
            fingerprint: fingerprint(2),
            ..review.clone()
        },
    ] {
        assert!(catalog
            .apply_tag_sidecar(
                &stale,
                &Patch {
                    key: Some("Am".into()),
                    ..Default::default()
                }
            )
            .is_err());
        assert!(catalog
            .observe_tags(&stale, &observation(stale.fingerprint))
            .is_err());
    }
    assert_eq!(serde_json::to_vec(&catalog).unwrap(), original);
    // The editor does not impose an arbitrary upper bound absent from Bpm.
    catalog
        .apply_tag_sidecar(
            &review,
            &Patch {
                bpm: Some("1000.25".into()),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        version(&catalog, &review).metadata.bpm.value(),
        Some(1000.25)
    );
}

fn rewrite() -> VerifiedRewrite {
    let identity = Identity {
        algorithm: 1,
        digest: [9; 32],
        packets: 17,
        bytes: 1024,
    };
    VerifiedRewrite {
        old_hash: [1; 32],
        new_hash: [2; 32],
        new_fingerprint: fingerprint(2),
        original_audio: identity.clone(),
        staged_audio: identity,
    }
}
#[test]
fn verified_rewrite_preserves_identity_crates_and_old_playing_receipts_without_reusing_file_hash() {
    let (mut catalog, review) = fixture();
    analysis(&mut catalog, &review, Some(151.5));
    catalog.update_preparation(
        &review.source,
        Some(review.fingerprint),
        Some(Preparation {
            cue: 1.5,
            hotcues: [Some(2.0), None, None, None, None, None, None, None],
            ..Default::default()
        }),
        Some(SystemTime::UNIX_EPOCH),
    );
    let crate_id = crates::CrateId("a".repeat(32));
    catalog
        .edit_crates(
            0,
            &crates::Edit::Create {
                id: crate_id.clone(),
                name: "Show".into(),
                parent: None,
                before: None,
            },
        )
        .unwrap();
    catalog
        .edit_crates(
            catalog.crates.revision(),
            &crates::Edit::AddMembers {
                id: crate_id,
                members: vec![review.id.clone()],
                before: None,
            },
        )
        .unwrap();
    let original = version(&catalog, &review).clone();
    let crates = catalog.crates.clone();
    let proof = rewrite();
    let edit = Patch {
        title: Some("Edited title".into()),
        ..Default::default()
    };
    let observed = observation(proof.new_fingerprint);
    catalog
        .accept_tag_rewrite(&review, &proof, &observed, &edit)
        .unwrap();
    let current_review = Review {
        fingerprint: proof.new_fingerprint,
        ..review.clone()
    };
    let current = version(&catalog, &current_review);
    assert_eq!(catalog.track(&review.source).unwrap().id, review.id);
    assert_eq!(catalog.crates, crates);
    assert_eq!(current.preparation, original.preparation);
    assert_eq!(current.analysis, original.analysis);
    assert_eq!(current.metadata.last_play, original.metadata.last_play);
    assert_eq!(current.metadata.title, "Edited title");
    assert_eq!(current.content_hash, Some([2; 32]));
    assert_eq!(version(&catalog, &review).content_hash, Some([1; 32]));
    assert_eq!(
        catalog.equivalent_current(&review.source, Some(review.fingerprint)),
        Some((&review.source, Some(proof.new_fingerprint)))
    );
    let updated = Preparation {
        cue: 3.0,
        ..original.preparation
    };
    let played = Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(123));
    catalog.update_preparation(
        &review.source,
        Some(review.fingerprint),
        Some(updated),
        played,
    );
    assert_eq!(version(&catalog, &current_review).preparation, updated);
    assert_eq!(
        version(&catalog, &current_review).metadata.last_play,
        played
    );
    catalog
        .upsert(review.source.clone(), Some(review.fingerprint), metadata())
        .unwrap();
    assert_eq!(
        catalog.track(&review.source).unwrap().versions
            [catalog.track(&review.source).unwrap().current]
            .fingerprint,
        Some(proof.new_fingerprint)
    );
    // Journal recovery of a completed transaction preserves subsequent edits.
    catalog
        .apply_tag_sidecar(
            &current_review,
            &Patch {
                title: Some("Later edit".into()),
                ..Default::default()
            },
        )
        .unwrap();
    catalog
        .accept_tag_rewrite(&review, &proof, &observed, &edit)
        .unwrap();
    assert_eq!(
        version(&catalog, &current_review).metadata.title,
        "Later edit"
    );
    catalog.validate().unwrap();
    // A later unqualified external replacement gets no audio equivalence.
    catalog
        .upsert(review.source.clone(), Some(fingerprint(3)), metadata())
        .unwrap();
    assert!(catalog
        .equivalent_current(&review.source, Some(review.fingerprint))
        .is_none());
    assert_eq!(
        catalog
            .version(&review.source, Some(fingerprint(3)))
            .unwrap()
            .preparation,
        Preparation::default()
    );
    assert!(catalog
        .accept_tag_rewrite(&review, &proof, &observed, &edit)
        .is_err());
}

#[test]
fn mismatched_rewrite_payload_or_saved_digest_is_rejected_before_mutation() {
    let (mut catalog, review) = fixture();
    analysis(&mut catalog, &review, Some(151.5));
    let original = serde_json::to_vec(&catalog).unwrap();
    for kind in 0..4 {
        let mut proof = rewrite();
        match kind {
            0 => proof.staged_audio.digest[0] ^= 1,
            1 => proof.original_audio.algorithm = 99,
            2 => proof.old_hash = [7; 32],
            _ => proof.new_fingerprint = review.fingerprint,
        }
        assert!(catalog
            .accept_tag_rewrite(
                &review,
                &proof,
                &observation(proof.new_fingerprint),
                &Patch::default()
            )
            .is_err());
        assert_eq!(serde_json::to_vec(&catalog).unwrap(), original);
    }
}

#[test]
fn pre_tag_schemas_migrate_but_reject_even_null_new_fields_and_new_bpm_states() {
    let dir = Dir::new();
    let (catalog, _) = fixture();
    for schema in 1..=7 {
        let mut old = serde_json::to_value(&catalog).unwrap();
        old["schema"] = schema.into();
        if schema < 7 {
            old.as_object_mut().unwrap().remove("watched_roots");
        }
        if schema < 6 {
            old.as_object_mut().unwrap().remove("crates");
        }
        if schema == 1 {
            let track = &catalog.tracks[0];
            let version = &track.versions[0];
            old["tracks"] = serde_json::json!([{"id":track.id,"source":track.source,
                "fingerprint":version.fingerprint,"metadata":version.metadata,"preparation":version.preparation}]);
        }
        fs::write(dir.path(), serde_json::to_vec(&old).unwrap()).unwrap();
        assert_eq!(read(&dir.path()).unwrap().tracks, catalog.tracks);
        for field in ["tags", "audio_identity", "embedded_origin", "user_cleared"] {
            let mut bad = old.clone();
            let version = if schema == 1 {
                &mut bad["tracks"][0]
            } else {
                &mut bad["tracks"][0]["versions"][0]
            };
            match field {
                "embedded_origin" => version["metadata"]["bpm"]["origin"] = "EmbeddedTag".into(),
                "user_cleared" => {
                    version["metadata"]["bpm"]["origin"] = "User".into();
                    version["metadata"]["bpm"]["value"] = serde_json::Value::Null;
                }
                _ => version[field] = serde_json::Value::Null,
            }
            let bytes = serde_json::to_vec(&bad).unwrap();
            fs::write(dir.path(), &bytes).unwrap();
            assert!(
                read(&dir.path()).is_err(),
                "schema {schema} accepted {field}"
            );
            assert_eq!(fs::read(dir.path()).unwrap(), bytes);
        }
    }
}
