use super::*;
use lofty::{config::WriteOptions, prelude::*, tag::Tag};
use std::path::PathBuf;
struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "omat-tags-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/audio")
            .join(name);
        let path = root.join(name);
        std::fs::copy(source, &path).unwrap();
        Self(path)
    }
    fn inspect(&self) -> Observation {
        let location = Snapshot::discover().unwrap().identify(&self.0).unwrap();
        inspect(
            &location,
            FileFingerprint::read(&self.0).unwrap(),
            &AtomicBool::new(false),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}
#[test]
fn real_unicode_tags_are_read_from_mp3_flac_wav_aiff_without_changing_files() {
    for name in ["tone.mp3", "tone.flac", "tone-tags.wav", "tone-tags.aiff"] {
        let file = Fixture::new(name);
        let original_identity = payload::measure(
            File::open(&file.0).unwrap(),
            std::sync::Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let original_audio = crate::engine::decode::decode_audio(&file.0).unwrap().sample;
        let parsed = Probe::open(&file.0)
            .unwrap()
            .guess_file_type()
            .unwrap()
            .read()
            .unwrap();
        let tag_type = parsed.primary_tag_type();
        let mut tag = Tag::new(tag_type);
        assert!(tag.insert_text(ItemKey::TrackTitle, "夜明け · Café 🎹".into()));
        assert!(tag.insert_text(ItemKey::TrackArtist, "Björk / 作曲家".into()));
        let bpm_key = if ItemKey::Bpm.map_key(tag_type).is_some() {
            ItemKey::Bpm
        } else {
            ItemKey::IntegerBpm
        };
        assert!(
            tag.insert_text(bpm_key, "128".into()),
            "{name}: BPM mapping"
        );
        assert!(tag.insert_text(ItemKey::InitialKey, "F#m".into()));
        tag.save_to_path(&file.0, WriteOptions::new().lossy_text_encoding(false))
            .unwrap();
        let before = std::fs::read(&file.0).unwrap();
        let observed = file.inspect();
        assert!(observed.supports_write());
        assert_eq!(
            observed.fields.title.as_ref().unwrap().value,
            "夜明け · Café 🎹"
        );
        assert_eq!(
            observed.fields.artist.as_ref().unwrap().value,
            "Björk / 作曲家"
        );
        assert_eq!(observed.fields.bpm.as_ref().unwrap().value, "128");
        assert_eq!(observed.fields.key.as_ref().unwrap().value, "F#m");
        assert_eq!(
            observed.fields.title.unwrap().source,
            TagSource::from_tag(tag_type)
        );
        assert_eq!(
            std::fs::read(&file.0).unwrap(),
            before,
            "reader wrote to media"
        );
        let after_identity = payload::measure(
            File::open(&file.0).unwrap(),
            std::sync::Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(
            original_identity, after_identity,
            "{name}: changed audio packet identity"
        );
        let after_audio = crate::engine::decode::decode_audio(&file.0).unwrap().sample;
        assert_eq!(original_audio.data, after_audio.data);
        assert_eq!(
            (original_audio.sr, original_audio.ch),
            (after_audio.sr, after_audio.ch)
        );
    }
}
#[test]
fn tagless_metadata_never_manufactures_filename_values_and_stale_cancelled_reads_refuse() {
    let file = Fixture::new("tone-tags.wav");
    let observed = file.inspect();
    assert_eq!(observed.fields, Fields::default());
    let location = Snapshot::discover().unwrap().identify(&file.0).unwrap();
    let fingerprint = FileFingerprint::read(&file.0).unwrap();
    assert!(inspect(&location, fingerprint, &AtomicBool::new(true))
        .unwrap_err()
        .contains("cancelled"));
    std::fs::write(&file.0, b"replacement").unwrap();
    assert!(inspect(&location, fingerprint, &AtomicBool::new(false))
        .unwrap_err()
        .contains("changed"));
}
#[test]
fn bounded_parser_rejects_reads_seeks_and_operations_past_its_capture() {
    let file = Fixture::new("tone.mp3");
    let mut input = File::open(&file.0).unwrap();
    let cancel = AtomicBool::new(false);
    let length = input.metadata().unwrap().len();
    let mut reader = Budget {
        file: &mut input,
        cancel: &cancel,
        length,
        bytes: 4,
        operations: 8,
    };
    let mut bytes = [0u8; 8];
    assert_eq!(reader.read(&mut bytes).unwrap(), 4);
    assert!(reader
        .read(&mut bytes)
        .unwrap_err()
        .to_string()
        .contains("budget"));
    assert!(reader.seek(SeekFrom::End(1)).is_err());
    assert!(reader.seek(SeekFrom::Start(length + 1)).is_err());
    cancel.store(true, Ordering::Release);
    assert!(reader
        .seek(SeekFrom::Start(0))
        .unwrap_err()
        .to_string()
        .contains("cancelled"));
}

#[test]
fn aiff_rejects_inconsistent_common_frames_and_short_sound_headers() {
    let file = Fixture::new("tone-tags.aiff");
    let original = std::fs::read(&file.0).unwrap();
    let decoded = crate::engine::decode::decode_audio(&file.0).unwrap();
    assert_eq!(decoded.sample.frames(), 12_000);
    assert_eq!(decoded.diagnostics.expected_frames, Some(12_000));
    let common = original.windows(4).position(|v| v == b"COMM").unwrap();
    let sound = original.windows(4).position(|v| v == b"SSND").unwrap();
    let mut bad_frames = original.clone();
    bad_frames[common + 10..common + 14].copy_from_slice(&12_001u32.to_be_bytes());
    std::fs::write(&file.0, bad_frames).unwrap();
    assert!(crate::engine::decode::decode_audio(&file.0)
        .unwrap_err()
        .to_string()
        .contains("sound size disagrees"));
    let mut short = original;
    short[sound + 4..sound + 8].copy_from_slice(&7u32.to_be_bytes());
    std::fs::write(&file.0, short).unwrap();
    assert!(crate::engine::decode::decode_audio(&file.0)
        .unwrap_err()
        .to_string()
        .contains("shorter than its header"));
}

#[test]
fn payload_identity_changes_for_actual_pcm_changes_and_refuses_cancellation() {
    let file = Fixture::new("tone-tags.wav");
    let identity = payload::measure(
        File::open(&file.0).unwrap(),
        std::sync::Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let mut bytes = std::fs::read(&file.0).unwrap();
    let data = bytes.windows(4).position(|v| v == b"data").unwrap();
    bytes[data + 12] ^= 1;
    std::fs::write(&file.0, bytes).unwrap();
    let changed = payload::measure(
        File::open(&file.0).unwrap(),
        std::sync::Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_ne!(identity.digest, changed.digest);
    assert_eq!(
        (identity.packets, identity.bytes),
        (changed.packets, changed.bytes)
    );
    assert!(payload::measure(
        File::open(&file.0).unwrap(),
        std::sync::Arc::new(AtomicBool::new(true))
    )
    .unwrap_err()
    .contains("cancelled"));
}

#[test]
fn conflicting_embedded_groups_keep_primary_provenance_and_show_the_conflict() {
    let file = Fixture::new("tone.mp3");
    for (kind, title) in [
        (TagType::Id3v1, "Legacy title"),
        (TagType::Id3v2, "Current title"),
    ] {
        let mut tag = Tag::new(kind);
        if kind == TagType::Id3v1 {
            assert!(tag.insert_text(ItemKey::Year, "2026".into()));
        }
        assert!(tag.insert_text(ItemKey::TrackTitle, title.into()));
        tag.save_to_path(&file.0, WriteOptions::new().lossy_text_encoding(false))
            .unwrap();
    }
    let observed = file.inspect();
    assert_eq!(
        observed.fields.title,
        Some(Field {
            value: "Current title".into(),
            source: TagSource::Id3v2
        })
    );
    assert!(observed
        .notices
        .iter()
        .any(|n| n.contains("Title: embedded tag values disagree")));
}

#[test]
fn blank_legacy_year_keeps_primary_values_visible_but_never_authorizes_rewriting() {
    let file = Fixture::new("tone.mp3");
    for (kind, title) in [
        (TagType::Id3v1, "Legacy title"),
        (TagType::Id3v2, "Current title · 夜明け"),
    ] {
        let mut tag = Tag::new(kind);
        assert!(tag.insert_text(ItemKey::TrackTitle, title.into()));
        tag.save_to_path(&file.0, WriteOptions::new().lossy_text_encoding(false))
            .unwrap();
    }
    let before = std::fs::read(&file.0).unwrap();
    let observed = file.inspect();
    assert_eq!(
        observed.fields.title.as_ref().unwrap().value,
        "Current title · 夜明け"
    );
    assert!(!observed.rewrite_eligible);
    assert!(!observed.supports_write());
    assert!(observed
        .notices
        .iter()
        .any(|n| n.contains("strict validation failed")));
    assert!(observed.notices.iter().all(|n| n.len() <= 1024));
    let mut input = File::open(&file.0).unwrap();
    assert!(read_tagged(&mut input, &AtomicBool::new(false), true)
        .err()
        .expect("strict rewriting must reject an invalid legacy year")
        .contains("year"));
    assert_eq!(std::fs::read(&file.0).unwrap(), before);
}

#[test]
fn tolerant_observation_discloses_a_skipped_malformed_frame_and_refuses_rewrite() {
    let file = Fixture::new("tone.mp3");
    let mut tag = Tag::new(TagType::Id3v2);
    assert!(tag.insert_text(ItemKey::TrackTitle, "Retained title".into()));
    tag.save_to_path(&file.0, WriteOptions::new()).unwrap();
    let mut bytes = std::fs::read(&file.0).unwrap();
    assert_eq!(&bytes[..4], b"ID3\x04");
    let length = bytes[6..10]
        .iter()
        .fold(0usize, |n, byte| (n << 7) | *byte as usize);
    let next_length = length + 10;
    for (index, byte) in bytes[6..10].iter_mut().enumerate() {
        *byte = ((next_length >> ((3 - index) * 7)) & 127) as u8;
    }
    bytes.splice(10..10, *b"TALB\0\0\0\0\0\0");
    std::fs::write(&file.0, &bytes).unwrap();
    let observed = file.inspect();
    assert_eq!(observed.fields.title.unwrap().value, "Retained title");
    assert!(!observed.rewrite_eligible);
    assert!(observed
        .notices
        .iter()
        .any(|n| n.contains("Read-only tag observation")));
    assert!(read_tagged(
        &mut File::open(&file.0).unwrap(),
        &AtomicBool::new(false),
        true
    )
    .is_err());
    assert_eq!(std::fs::read(&file.0).unwrap(), bytes);
}

#[test]
fn same_tag_empty_bpm_does_not_hide_integer_bpm_and_duplicate_values_are_disclosed() {
    let fixture = Fixture::new("tone.m4a");
    let mut tagged = Probe::open(&fixture.0).unwrap().read().unwrap();
    let mut tag = Tag::new(TagType::Mp4Ilst);
    assert!(tag.insert_text(ItemKey::Bpm, " ".into()));
    assert!(tag.insert_text(ItemKey::IntegerBpm, "128".into()));
    tagged.insert_tag(tag);
    assert_eq!(fields(&tagged).0.bpm.unwrap().value, "128");
    let tag = tagged.primary_tag_mut().unwrap();
    assert!(tag.insert_text(ItemKey::Bpm, "128.5".into()));
    let (selected, notices) = fields(&tagged);
    assert_eq!(selected.bpm.unwrap().value, "128.5");
    assert!(notices
        .iter()
        .any(|n| n.contains("BPM: embedded tag values disagree")));
}

#[test]
fn tag_reader_cannot_consume_bytes_appended_beyond_its_captured_length() {
    use std::io::Write;
    let fixture = Fixture::new("tone.mp3");
    let mut input = File::open(&fixture.0).unwrap();
    let captured = input.metadata().unwrap().len();
    input.seek(SeekFrom::Start(captured - 2)).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&fixture.0)
        .unwrap()
        .write_all(b"appended")
        .unwrap();
    let cancel = AtomicBool::new(false);
    let mut reader = Budget {
        file: &mut input,
        cancel: &cancel,
        length: captured,
        bytes: 100,
        operations: 100,
    };
    assert_eq!(reader.read(&mut [0; 32]).unwrap(), 2);
    assert_eq!(reader.read(&mut [0; 32]).unwrap(), 0);
}

#[test]
fn truncated_pcm_payloads_refuse_incomplete_audio_identity() {
    for name in ["tone-tags.wav", "tone-tags.aiff"] {
        let fixture = Fixture::new(name);
        let original = std::fs::read(&fixture.0).unwrap();
        std::fs::write(&fixture.0, &original[..original.len() - 8192]).unwrap();
        let error = payload::measure(
            File::open(&fixture.0).unwrap(),
            std::sync::Arc::new(AtomicBool::new(false)),
        )
        .unwrap_err();
        assert!(error.contains("declared audio"), "{name}: {error}");
    }
}

#[test]
fn encoded_audio_with_a_failed_stream_checksum_cannot_qualify_a_tag_rewrite() {
    let fixture = Fixture::new("tone.flac");
    let mut bytes = std::fs::read(&fixture.0).unwrap();
    assert_eq!(&bytes[..4], b"fLaC");
    assert_eq!(bytes[4] & 127, 0);
    assert_eq!(&bytes[5..8], &[0, 0, 34]);
    // Alter only STREAMINFO's MD5 declaration, leaving encoded packets intact.
    bytes[26] ^= 1;
    std::fs::write(&fixture.0, &bytes).unwrap();
    assert!(crate::engine::decode::decode_audio(&fixture.0).is_err());
    let error = payload::measure(
        File::open(&fixture.0).unwrap(),
        std::sync::Arc::new(AtomicBool::new(false)),
    )
    .unwrap_err();
    assert!(error.contains("checksum"), "{error}");
}

#[test]
fn aiff_rejects_partial_audio_frames_but_accepts_external_odd_byte_padding() {
    let fixture = Fixture::new("tone-tags.aiff");
    let original = std::fs::read(&fixture.0).unwrap();
    let sound = original
        .windows(4)
        .position(|bytes| bytes == b"SSND")
        .unwrap();
    let length = u32::from_be_bytes(original[sound + 4..sound + 8].try_into().unwrap());
    let mut partial = original.clone();
    partial[sound + 4..sound + 8].copy_from_slice(&(length + 1).to_be_bytes());
    let end = sound + 8 + length as usize;
    partial.splice(end..end, [42, 0]);
    let form_length = (partial.len() - 8) as u32;
    partial[4..8].copy_from_slice(&form_length.to_be_bytes());
    std::fs::write(&fixture.0, partial).unwrap();
    assert!(crate::engine::decode::decode_audio(&fixture.0)
        .unwrap_err()
        .to_string()
        .contains("partial audio frame"));

    // Three valid signed 8-bit mono frames require one external chunk pad.
    // Keep the fixture's actual 80-bit sample-rate declaration unchanged.
    let common = original
        .windows(4)
        .position(|bytes| bytes == b"COMM")
        .unwrap();
    let mut common_body = original[common + 8..common + 26].to_vec();
    common_body[..2].copy_from_slice(&1u16.to_be_bytes());
    common_body[2..6].copy_from_slice(&3u32.to_be_bytes());
    common_body[6..8].copy_from_slice(&8u16.to_be_bytes());
    let mut valid = b"FORM\0\0\0\0AIFFCOMM".to_vec();
    valid.extend_from_slice(&18u32.to_be_bytes());
    valid.extend_from_slice(&common_body);
    valid.extend_from_slice(b"SSND");
    valid.extend_from_slice(&11u32.to_be_bytes());
    valid.extend_from_slice(&[0; 8]);
    valid.extend_from_slice(&[0, 64, 192, 0]);
    let form_length = (valid.len() - 8) as u32;
    valid[4..8].copy_from_slice(&form_length.to_be_bytes());
    std::fs::write(&fixture.0, valid).unwrap();
    let audio = crate::engine::decode::decode_audio(&fixture.0)
        .unwrap()
        .sample;
    assert_eq!(audio.ch, 1);
    assert_eq!(audio.frames(), 3);
    assert_eq!(audio.data, [0.0, 0.5, -0.5]);
    let identity = payload::measure(
        File::open(&fixture.0).unwrap(),
        std::sync::Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(identity.bytes, 3);
}
