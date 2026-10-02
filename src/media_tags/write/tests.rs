use super::*;
use crate::{engine::performance::Handle, library::TrackId};
struct Fixture {
    root: PathBuf,
    media: PathBuf,
    recovery: PathBuf,
}
impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "omat-tag-write-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let media = root.join(name);
        fs::copy(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audio")
                .join(name),
            &media,
        )
        .unwrap();
        let recovery = root.join("recovery");
        Self {
            root,
            media,
            recovery,
        }
    }
    fn location(&self) -> Location {
        Snapshot::discover().unwrap().identify(&self.media).unwrap()
    }
    fn review(&self) -> Review {
        Review {
            id: TrackId("1".repeat(32)),
            source: self.location().source,
            fingerprint: FileFingerprint::read(&self.media).unwrap(),
        }
    }
    fn work(&self) -> WorkPermit {
        Handle::default().optional_work().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn patch() -> Patch {
    Patch {
        title: Some("夜明け · Café 🎹".into()),
        artist: Some("Björk / 作曲家".into()),
        bpm: Some("128".into()),
        key: Some("F#m".into()),
    }
}
#[test]
fn unicode_rewrites_all_supported_formats_keep_original_audio_until_catalog_confirmation() {
    for name in ["tone.mp3", "tone.flac", "tone-tags.wav", "tone-tags.aiff"] {
        let fixture = Fixture::new(name);
        let before = fs::read(&fixture.media).unwrap();
        let original = crate::engine::decode::decode_audio(&fixture.media)
            .unwrap()
            .sample;
        let applied = apply(
            &fixture.location(),
            &fixture.review(),
            &patch(),
            &fixture.recovery,
            &fixture.work(),
        )
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(
            applied.observation.fields.title.as_ref().unwrap().value,
            "夜明け · Café 🎹"
        );
        assert_eq!(
            applied.observation.fields.artist.as_ref().unwrap().value,
            "Björk / 作曲家"
        );
        assert_eq!(applied.proof.original_audio, applied.proof.staged_audio);
        assert_ne!(applied.proof.old_hash, applied.proof.new_hash);
        assert_eq!(
            fs::read(applied.record.stage_path(&fixture.location()).unwrap()).unwrap(),
            before
        );
        let after = crate::engine::decode::decode_audio(&fixture.media)
            .unwrap()
            .sample;
        assert_eq!(original.data, after.data);
        assert_eq!((original.sr, original.ch), (after.sr, after.ch));
        let recovered = recover(&fixture.recovery);
        assert!(
            matches!(&recovered[..], [Recovery::Applied(_)]),
            "{name}: {recovered:?}"
        );
        finalize(&applied.record).unwrap();
        assert!(recover(&fixture.recovery).is_empty());
        assert!(!applied
            .record
            .stage_path(&fixture.location())
            .unwrap()
            .exists());
        assert!(super::super::inspect(
            &fixture.location(),
            FileFingerprint::read(&fixture.media).unwrap(),
            &AtomicBool::new(false)
        )
        .is_ok());
    }
}
#[test]
fn durable_intent_recovers_before_and_after_atomic_exchange_without_guessing() {
    for stop in [0, 1] {
        let fixture = Fixture::new("tone.mp3");
        let bytes = fs::read(&fixture.media).unwrap();
        let work = fixture.work();
        let failed = apply_with(
            &fixture.location(),
            &fixture.review(),
            &patch(),
            &fixture.recovery,
            &work,
            |point| {
                if point == stop {
                    Err("simulated interruption".into())
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();
        let record = failed.record.unwrap();
        let recovered = recover(&fixture.recovery);
        if stop == 0 {
            assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
            assert!(matches!(&recovered[..], [Recovery::Unchanged(_)]));
            let again = apply(
                &fixture.location(),
                &fixture.review(),
                &patch(),
                &fixture.recovery,
                &work,
            )
            .unwrap_err();
            assert!(again.message.contains("unresolved"));
            discard_staged(&record).unwrap();
        } else {
            assert_ne!(fs::read(&fixture.media).unwrap(), bytes);
            assert!(matches!(&recovered[..], [Recovery::Applied(_)]));
            assert!(discard_staged(&record).is_err());
            finalize(&record).unwrap();
        }
        assert!(recover(&fixture.recovery).is_empty());
    }
}
#[test]
fn read_only_hardlinked_cancelled_and_stale_media_never_change_bytes() {
    let fixture = Fixture::new("tone.flac");
    let review = fixture.review();
    let bytes = fs::read(&fixture.media).unwrap();
    fs::set_permissions(&fixture.media, fs::Permissions::from_mode(0o444)).unwrap();
    let readonly = apply(
        &fixture.location(),
        &fixture.review(),
        &patch(),
        &fixture.recovery,
        &fixture.work(),
    )
    .unwrap_err();
    assert!(readonly.record.is_none());
    assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
    fs::set_permissions(&fixture.media, fs::Permissions::from_mode(0o640)).unwrap();
    fs::hard_link(&fixture.media, fixture.root.join("other.flac")).unwrap();
    assert!(apply(
        &fixture.location(),
        &fixture.review(),
        &patch(),
        &fixture.recovery,
        &fixture.work()
    )
    .unwrap_err()
    .message
    .contains("hard-linked"));
    fs::remove_file(fixture.root.join("other.flac")).unwrap();
    let cancelled = fixture.work();
    cancelled.cancel().store(true, Ordering::Release);
    assert!(apply(
        &fixture.location(),
        &fixture.review(),
        &patch(),
        &fixture.recovery,
        &cancelled
    )
    .unwrap_err()
    .record
    .is_none());
    assert!(apply(
        &fixture.location(),
        &review,
        &patch(),
        &fixture.recovery,
        &fixture.work()
    )
    .unwrap_err()
    .record
    .is_none());
    assert_eq!(fs::read(&fixture.media).unwrap(), bytes);
}
#[test]
fn fractional_bpm_is_explicit_sidecar_value_while_other_tags_are_really_written() {
    let fixture = Fixture::new("tone.mp3");
    let mut requested = patch();
    requested.bpm = Some("127.125".into());
    let applied = apply(
        &fixture.location(),
        &fixture.review(),
        &requested,
        &fixture.recovery,
        &fixture.work(),
    )
    .unwrap();
    assert!(applied.observation.fields.bpm.is_none());
    assert_eq!(applied.record.patch.bpm.as_deref(), Some("127.125"));
    assert!(applied.notices.iter().any(|n| n.contains("fractional BPM")));
    assert_eq!(
        applied.observation.fields.title.unwrap().value,
        "夜明け · Café 🎹"
    );
    finalize(&applied.record).unwrap();
    let fixture = Fixture::new("tone.flac");
    let applied = apply(
        &fixture.location(),
        &fixture.review(),
        &requested,
        &fixture.recovery,
        &fixture.work(),
    )
    .unwrap();
    assert_eq!(applied.observation.fields.bpm.unwrap().value, "127.125");
    finalize(&applied.record).unwrap();
}
#[test]
fn conflicting_external_changes_are_retained_with_recovery_record_and_original() {
    let fixture = Fixture::new("tone.mp3");
    let before = fs::read(&fixture.media).unwrap();
    let applied = apply(
        &fixture.location(),
        &fixture.review(),
        &patch(),
        &fixture.recovery,
        &fixture.work(),
    )
    .unwrap();
    let original_path = applied.record.stage_path(&fixture.location()).unwrap();
    fs::write(&fixture.media, b"external edit after exchange").unwrap();
    assert!(matches!(
        &recover(&fixture.recovery)[..],
        [Recovery::Conflict {
            record: Some(_),
            ..
        }]
    ));
    assert!(finalize(&applied.record).is_err());
    assert_eq!(
        fs::read(&fixture.media).unwrap(),
        b"external edit after exchange"
    );
    assert_eq!(fs::read(&original_path).unwrap(), before);
    assert!(applied.record.journal_path().exists());
}
#[test]
fn cancellation_after_durable_intent_is_inert_but_after_exchange_reports_installed() {
    for point in [0, 1] {
        let fixture = Fixture::new("tone.mp3");
        let before = fs::read(&fixture.media).unwrap();
        let work = fixture.work();
        let cancel = work.cancel();
        let result = apply_with(
            &fixture.location(),
            &fixture.review(),
            &patch(),
            &fixture.recovery,
            &work,
            |at| {
                if at == point {
                    cancel.store(true, Ordering::Release);
                }
                Ok(())
            },
        );
        if point == 0 {
            let record = result.unwrap_err().record.unwrap();
            assert_eq!(fs::read(&fixture.media).unwrap(), before);
            discard_staged(&record).unwrap();
        } else {
            let applied = result.unwrap();
            assert_ne!(fs::read(&fixture.media).unwrap(), before);
            finalize(&applied.record).unwrap();
        }
    }
}
#[test]
fn empty_patch_and_oversized_rewrite_refuse_without_touching_original() {
    let fixture = Fixture::new("tone.mp3");
    assert!(apply(
        &fixture.location(),
        &fixture.review(),
        &Patch::default(),
        &fixture.recovery,
        &fixture.work()
    )
    .unwrap_err()
    .record
    .is_none());
    let f = OpenOptions::new().write(true).open(&fixture.media).unwrap();
    f.set_len(MAX_REWRITE_BYTES + 1).unwrap();
    let expected = FileFingerprint::read(&fixture.media).unwrap();
    let rejected = apply(
        &fixture.location(),
        &fixture.review(),
        &patch(),
        &fixture.recovery,
        &fixture.work(),
    )
    .unwrap_err();
    assert!(rejected.message.contains("128 MiB"));
    assert_eq!(FileFingerprint::read(&fixture.media), Some(expected));
}

#[test]
fn replacement_in_the_final_exchange_race_is_retained_instead_of_deleted() {
    let fixture = Fixture::new("tone.mp3");
    let external = fixture.root.join("external.mp3");
    fs::write(&external, b"external replacement racing commit").unwrap();
    let failed = apply_with(
        &fixture.location(),
        &fixture.review(),
        &patch(),
        &fixture.recovery,
        &fixture.work(),
        |point| {
            if point == 2 {
                fs::rename(&external, &fixture.media).map_err(err)?;
            }
            Ok(())
        },
    )
    .unwrap_err();
    assert!(!failed.fallback_safe);
    let record = failed.record.unwrap();
    assert_eq!(
        fs::read(record.stage_path(&fixture.location()).unwrap()).unwrap(),
        b"external replacement racing commit"
    );
    assert!(matches!(
        &recover(&fixture.recovery)[..],
        [Recovery::Conflict {
            record: Some(_),
            ..
        }]
    ));
    assert!(finalize(&record).is_err());
    assert!(record.journal_path().exists());
    assert!(super::super::inspect(
        &fixture.location(),
        FileFingerprint::read(&fixture.media).unwrap(),
        &AtomicBool::new(false)
    )
    .is_ok());
}
#[test]
fn unknown_recovery_records_block_sidecar_fallback_and_keep_media_unchanged() {
    let fixture = Fixture::new("tone.mp3");
    let before = fs::read(&fixture.media).unwrap();
    root_directory(&fixture.recovery).unwrap();
    fs::write(fixture.recovery.join("unknown.json"), b"not a valid intent").unwrap();
    let failed = apply(
        &fixture.location(),
        &fixture.review(),
        &patch(),
        &fixture.recovery,
        &fixture.work(),
    )
    .unwrap_err();
    assert!(!failed.fallback_safe);
    assert!(failed.record.is_none());
    assert_eq!(fs::read(&fixture.media).unwrap(), before);
    assert!(matches!(
        &recover(&fixture.recovery)[..],
        [Recovery::Conflict { record: None, .. }]
    ));
}

#[test]
fn protection_orders_before_exchange_or_after_its_short_commit_claim() {
    for point in [0, 2, 1] {
        let fixture = Fixture::new("tone.mp3");
        let before = fs::read(&fixture.media).unwrap();
        let performance = Handle::default();
        let work = performance.optional_work().unwrap();
        let result = apply_with(
            &fixture.location(),
            &fixture.review(),
            &patch(),
            &fixture.recovery,
            &work,
            |at| {
                if at == point {
                    let enabled = performance.set_enabled(true);
                    if point == 2 {
                        assert_eq!(enabled, Err(crate::engine::performance::Error::Changing));
                    } else {
                        enabled.unwrap();
                    }
                }
                Ok(())
            },
        );
        if point == 0 {
            let failed = result.unwrap_err();
            assert_eq!(fs::read(&fixture.media).unwrap(), before);
            discard_staged(&failed.record.unwrap()).unwrap();
        } else {
            let installed = result.unwrap();
            assert_ne!(fs::read(&fixture.media).unwrap(), before);
            finalize(&installed.record).unwrap();
        }
    }
}

#[test]
fn only_uuid_qualified_recovery_can_accept_changed_kernel_device_numbers() {
    use crate::engine::media_source::LibSource;
    let fixture = Fixture::new("tone.mp3");
    let metadata = fs::metadata(&fixture.media).unwrap();
    let mut stored = Node::of(&metadata);
    stored.device ^= 1;
    assert!(!stored.matches(&metadata));
    assert!(!stored.matches_recovered(&metadata, &LibSource::File(fixture.media.clone())));
    let removable = LibSource::Removable {
        volume_id: "fake-volume-for-guard-test".into(),
        relative_path: "tone.mp3".into(),
    };
    assert!(stored.matches_recovered(&metadata, &removable));
    stored.inode ^= 1;
    assert!(!stored.matches_recovered(&metadata, &removable));
    // The production recovery path obtains this permission only AFTER resolving
    // a currently unambiguous UUID. A syntactically removable source alone is
    // insufficient: an unavailable fake volume leaves both files and journal.
    let applied = apply(
        &fixture.location(),
        &fixture.review(),
        &patch(),
        &fixture.recovery,
        &fixture.work(),
    )
    .unwrap();
    let mut record = applied.record;
    record.review.source = removable;
    fs::write(record.journal_path(), serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(matches!(
        &recover(&fixture.recovery)[..],
        [Recovery::Conflict {
            record: Some(_),
            ..
        }]
    ));
    assert!(record.journal_path().exists());
    assert!(fixture
        .root
        .join(record.stage_name())
        .join("media")
        .exists());
}

#[test]
fn concrete_flac_writer_keeps_custom_duplicate_comments_and_vendor() {
    use lofty::{file::AudioFile, flac::FlacFile, ogg::VorbisComments};
    let fixture = Fixture::new("tone.flac");
    let mut comments = VorbisComments::new();
    comments.set_vendor("custom encoder π".into());
    comments.push("X-PRODUCER-NOTES".into(), "take one 夜".into());
    comments.push("X-PRODUCER-NOTES".into(), "take two 🎹".into());
    comments.push("ARTIST".into(), "old artist".into());
    comments
        .save_to_path(
            &fixture.media,
            WriteOptions::new().lossy_text_encoding(false),
        )
        .unwrap();
    // Lofty deliberately retains an existing vendor string even when the
    // supplied comments request another one. Construct the distinct vendor in
    // the fixture's actual Vorbis block, then prove the writer preserves it.
    let mut bytes = fs::read(&fixture.media).unwrap();
    let mut offset = 4usize;
    loop {
        let kind = bytes[offset] & 127;
        let length =
            u32::from_be_bytes([0, bytes[offset + 1], bytes[offset + 2], bytes[offset + 3]])
                as usize;
        if kind == 4 {
            let start = offset + 4;
            let vendor_len =
                u32::from_le_bytes(bytes[start..start + 4].try_into().unwrap()) as usize;
            let vendor = "custom encoder π".as_bytes();
            let mut replacement = (vendor.len() as u32).to_le_bytes().to_vec();
            replacement.extend_from_slice(vendor);
            replacement.extend_from_slice(&bytes[start + 4 + vendor_len..start + length]);
            let size = (replacement.len() as u32).to_be_bytes();
            bytes[offset + 1..offset + 4].copy_from_slice(&size[1..]);
            bytes.splice(start..start + length, replacement);
            break;
        }
        assert_eq!(bytes[offset] & 128, 0, "fixture requires Vorbis comments");
        offset += 4 + length;
    }
    fs::write(&fixture.media, bytes).unwrap();
    let before = FlacFile::read_from(
        &mut File::open(&fixture.media).unwrap(),
        parse_options(true),
    )
    .unwrap();
    assert_eq!(
        before.vorbis_comments().unwrap().vendor(),
        "custom encoder π"
    );
    let applied = apply(
        &fixture.location(),
        &fixture.review(),
        &patch(),
        &fixture.recovery,
        &fixture.work(),
    )
    .unwrap();
    let parsed = FlacFile::read_from(
        &mut File::open(&fixture.media).unwrap(),
        parse_options(true),
    )
    .unwrap();
    let actual = parsed.vorbis_comments().unwrap();
    assert_eq!(actual.vendor(), "custom encoder π");
    assert_eq!(
        actual.get_all("X-PRODUCER-NOTES").collect::<Vec<_>>(),
        ["take one 夜", "take two 🎹"]
    );
    assert_eq!(actual.get("ARTIST"), Some("Björk / 作曲家"));
    finalize(&applied.record).unwrap();
}
