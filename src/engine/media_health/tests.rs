use super::*;
use crate::engine::media_analysis::tests::{wav, Files};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{atomic::AtomicU64, Arc},
};

fn check(source: LibSource, fingerprint: Option<FileFingerprint>) -> Observation {
    let current = Arc::new(AtomicU64::new(1));
    let token = media_analysis::Token::new(1, current);
    let performance = performance::Handle::default();
    let work = performance.optional_work().unwrap();
    run(
        Request {
            source,
            fingerprint,
        },
        &token,
        &work,
    )
    .unwrap()
}

#[test]
fn real_media_conditions_preserve_read_only_playback_and_recover_after_repairs() {
    let files = Files::new();
    let reference = files.source("healthy.wav", &wav(12000, 48000, 2, false));
    let LibSource::File(path) = &reference.source else {
        panic!()
    };
    assert_eq!(
        check(reference.source.clone(), Some(reference.fingerprint)).condition,
        Condition::Ready
    );
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o444)).unwrap();
    let read_only = check(reference.source.clone(), FileFingerprint::read(path));
    assert_eq!(read_only.condition, Condition::Ready);
    assert!(read_only.read_only && read_only.description().contains("read-only"));
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
    assert_ne!(
        unsafe { libc::geteuid() },
        0,
        "permission fixture must run without root privileges"
    );
    assert_eq!(
        check(reference.source.clone(), None).condition,
        Condition::Unreadable
    );
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        check(reference.source.clone(), Some(reference.fingerprint)).condition,
        Condition::Changed
    );
    let bytes = wav(12000, 48000, 2, false);
    std::fs::write(path, &bytes[..bytes.len() - 4000]).unwrap();
    let corrupt = check(reference.source.clone(), FileFingerprint::read(path));
    assert_eq!(
        corrupt.condition,
        Condition::Corrupt,
        "{}",
        corrupt.description()
    );
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        check(reference.source.clone(), Some(reference.fingerprint)).condition,
        Condition::Missing
    );
    std::fs::write(path, bytes).unwrap();
    assert_eq!(
        check(reference.source.clone(), FileFingerprint::read(path)).condition,
        Condition::Ready
    );
    let unsupported = files.source("unsupported.wav", b"This is not an audio container.");
    assert_eq!(
        check(unsupported.source, Some(unsupported.fingerprint)).condition,
        Condition::Unsupported
    );
}

#[test]
fn streaming_validation_does_not_retain_the_whole_audio_payload_and_cancels_between_packets() {
    let files = Files::new();
    let frames = 9_000_000;
    let reference = files.source("long.wav", &wav(frames, 48000, 2, false));
    let LibSource::File(path) = reference.source else {
        panic!()
    };
    let file = std::fs::File::open(&path).unwrap();
    let result = decode::validate_audio_file(&path, file, || false, |_, _| {}).unwrap();
    assert_eq!(result.0.decoded_frames, frames as u64);
    let retained = decode::decode_sampler_file(
        &path,
        std::fs::File::open(&path).unwrap(),
        64 * 1024 * 1024,
        || false,
    )
    .unwrap_err();
    assert_eq!(
        retained.kind,
        decode::DecodeFailureKind::Capacity,
        "whole decoded PCM exceeds the same 64 MiB credit that permits streaming validation"
    );
    let cancelled = std::cell::Cell::new(false);
    let error = decode::validate_audio_file(
        &path,
        std::fs::File::open(&path).unwrap(),
        || cancelled.get(),
        |_, _| cancelled.set(true),
    )
    .unwrap_err();
    assert_eq!(error.kind, decode::DecodeFailureKind::Cancelled);
    assert!(
        error.diagnostics.decoded_frames > 0 && error.diagnostics.decoded_frames < frames as u64
    );
}

#[test]
fn request_cancellation_and_optional_work_protection_never_label_media_as_corrupt() {
    let files = Files::new();
    let reference = files.source(
        "healthy.flac",
        include_bytes!("../../../tests/fixtures/audio/tone.flac"),
    );
    let current = Arc::new(AtomicU64::new(1));
    let token = media_analysis::Token::new(1, current.clone());
    let performance = performance::Handle::default();
    let work = performance.optional_work().unwrap();
    token.cancel();
    assert!(matches!(
        run(
            Request {
                source: reference.source.clone(),
                fingerprint: Some(reference.fingerprint)
            },
            &token,
            &work
        ),
        Err(media_analysis::Failure::Cancelled)
    ));
    let token = media_analysis::Token::new(1, current);
    work.cancel()
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(matches!(
        run(
            Request {
                source: reference.source,
                fingerprint: Some(reference.fingerprint)
            },
            &token,
            &work
        ),
        Err(media_analysis::Failure::Protected)
    ));
}

#[test]
fn malformed_embedded_tags_are_disclosed_separately_when_the_audio_decodes() {
    use lofty::{
        config::WriteOptions,
        tag::{ItemKey, Tag, TagExt, TagType},
    };
    let files = Files::new();
    let reference = files.source(
        "bad-tags.mp3",
        include_bytes!("../../../tests/fixtures/audio/tone.mp3"),
    );
    let LibSource::File(path) = &reference.source else {
        panic!()
    };
    let mut tag = Tag::new(TagType::Id3v2);
    tag.insert_text(ItemKey::TrackTitle, "Retained title".into());
    tag.save_to_path(path, WriteOptions::new()).unwrap();
    let mut bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"ID3\x04");
    let length = bytes[6..10]
        .iter()
        .fold(0usize, |n, byte| (n << 7) | *byte as usize)
        + 10;
    for (index, byte) in bytes[6..10].iter_mut().enumerate() {
        *byte = ((length >> ((3 - index) * 7)) & 127) as u8;
    }
    bytes.splice(10..10, *b"TALB\0\0\0\0\0\0");
    std::fs::write(path, &bytes).unwrap();
    let observed = check(reference.source.clone(), FileFingerprint::read(path));
    assert_eq!(
        observed.condition,
        Condition::Unverified,
        "{}",
        observed.description()
    );
    assert!(
        observed.detail.contains("Tags need inspection"),
        "{}",
        observed.detail
    );
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    let estimated = files.source(
        "estimated.mp3",
        include_bytes!("../../../tests/fixtures/audio/tone-estimated.mp3"),
    );
    let uncertain = check(estimated.source, Some(estimated.fingerprint));
    assert_eq!(uncertain.condition, Condition::Unverified);
    assert!(uncertain
        .detail
        .contains("incomplete media cannot be ruled out"));
    let offline = check(
        LibSource::Removable {
            volume_id: "1234-ABCD".into(),
            relative_path: "lost.wav".into(),
        },
        None,
    );
    assert_eq!(offline.condition, Condition::Missing);
    assert!(offline.description().contains("Reconnect the drive"));
}
