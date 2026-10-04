use super::*;
use crate::{engine::media_source::LibSource, library::Catalog, sampler_bank::BankId};
use std::path::PathBuf;

pub(crate) struct Files(pub PathBuf);
impl Files {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!("omat-analysis-{}", BankId::new().unwrap()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub fn source(&self, name: &str, bytes: &[u8]) -> SourceRef {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        let source = LibSource::File(path.clone());
        let fingerprint = FileFingerprint::read(&path).unwrap();
        let mut catalog = Catalog::default();
        catalog
            .upsert(
                source.clone(),
                Some(fingerprint),
                crate::library::Metadata {
                    title: name.into(),
                    artist: String::new(),
                    bpm: crate::ui::bpm::Bpm::UNKNOWN,
                    key: String::new(),
                    duration: None,
                    last_play: None,
                },
            )
            .unwrap();
        SourceRef {
            track: catalog.track(&source).unwrap().id.clone(),
            source,
            fingerprint,
            content_hash: None,
        }
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub(crate) fn wav(frames: usize, sr: u32, channels: u16, silent: bool) -> Vec<u8> {
    let size = frames as u32 * u32::from(channels) * 2;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + size).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&sr.to_le_bytes());
    bytes.extend_from_slice(&(sr * u32::from(channels) * 2).to_le_bytes());
    bytes.extend_from_slice(&(channels * 2).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&size.to_le_bytes());
    for i in 0..frames * channels as usize {
        bytes.extend_from_slice(&(if silent { 0 } else { (i as i16 % 512) * 32 }).to_le_bytes());
    }
    bytes
}
fn token() -> Token {
    Token::new(1, Arc::new(AtomicU64::new(1)))
}
fn analyze(reference: SourceRef, fields: Fields) -> Prepared {
    run(
        Request { reference, fields },
        &token(),
        &performance::Handle::default().optional_work().unwrap(),
    )
    .unwrap()
}

#[test]
fn actual_mixed_formats_produce_only_selected_bounded_source_qualified_results() {
    let files = Files::new();
    for (name, bytes) in [
        ("mono.wav", wav(16000, 16000, 1, false)),
        (
            "stereo.flac",
            include_bytes!("../../../tests/fixtures/audio/tone.flac").to_vec(),
        ),
        (
            "vorbis.ogg",
            include_bytes!("../../../tests/fixtures/audio/tone.ogg").to_vec(),
        ),
    ] {
        let reference = files.source(name, &bytes);
        let decoded = decode::decode_audio(reference.path().unwrap())
            .unwrap()
            .sample;
        let result = analyze(reference.clone(), Fields::ALL);
        assert_eq!(result.reference.track, reference.track);
        assert_eq!(result.reference.fingerprint, reference.fingerprint);
        assert_eq!(
            result.reference.content_hash,
            Some(Sha256::digest(&bytes).into())
        );
        assert_eq!(
            result.duration,
            decoded.frames() as f64 / f64::from(decoded.sr)
        );
        let wave = result.waveform.unwrap();
        assert_eq!(wave.frames, decoded.frames() as u64);
        assert_eq!(wave.sample_rate, decoded.sr);
        assert_eq!(wave.channels, decoded.ch);
        assert_eq!(wave.bands.as_slice(), decoded.peaks.as_slice());
        wave.validate().unwrap();
        assert_eq!(wave.bands.len(), 2048);
        let only_duration = analyze(
            reference,
            Fields {
                bpm: false,
                duration: true,
                waveform: false, level: false },
        );
        assert!(only_duration.waveform.is_none() && only_duration.bpm.is_none());
        assert_eq!(only_duration.duration, result.duration);
    }
}

#[test]
fn silence_short_and_no_score_are_unknown_and_legacy_deck_fallback_is_retained() {
    let files = Files::new();
    for (frames, silent) in [(1, false), (256, false), (48_000, true)] {
        let reference = files.source(&format!("{frames}.wav"), &wav(frames, 48000, 1, silent));
        let result = analyze(reference, Fields::ALL);
        assert_eq!(result.bpm, None);
    }
    assert_eq!(
        dsp::estimate_bpm_with_cancel(&[0.0; 48_000], 1, 48000, || false),
        Ok(None)
    );
    assert_eq!(dsp::detect_bpm(&[0.0; 48_000], 1, 48000), 120.0);
    assert_eq!(
        dsp::estimate_bpm_with_cancel(&[0.5; 48_000], 1, 48000, || false),
        Ok(None)
    );
    assert_eq!(
        dsp::estimate_bpm_with_cancel(&[0.0; 48_000], 1, 48000, || true),
        Err(())
    );
}

#[test]
fn changed_bytes_and_path_identity_never_receive_an_analysis_proof() {
    let files = Files::new();
    let mut reference = files.source("track.wav", &wav(1024, 8000, 1, false));
    reference.content_hash = Some([0xff; 32]);
    let work = performance::Handle::default().optional_work().unwrap();
    let result = run(
        Request {
            reference: reference.clone(),
            fields: Fields::ALL,
        },
        &token(),
        &work,
    );
    assert!(result.unwrap_err().to_string().contains("bytes differ"));
    reference.content_hash = None;
    std::fs::write(reference.path().unwrap(), wav(2048, 8000, 1, false)).unwrap();
    assert!(run(
        Request {
            reference: reference.clone(),
            fields: Fields::ALL
        },
        &token(),
        &work
    )
    .is_err());
    let original = files.0.join("original.wav");
    std::fs::rename(reference.path().unwrap(), &original).unwrap();
    std::os::unix::fs::symlink(original, reference.path().unwrap()).unwrap();
    assert!(run(
        Request {
            reference,
            fields: Fields::ALL
        },
        &token(),
        &work
    )
    .is_err());
}

#[test]
fn prepared_token_and_permit_remain_cancellable_until_publication_claim() {
    let performance = performance::Handle::default();
    let work = performance.optional_work().unwrap();
    let token = token();
    token.set_progress(Stage::Ready, 1, Some(1));
    assert!(token.cancel());
    let _guard = work.commit().unwrap();
    assert_eq!(token.claim_publication(), Err(Failure::Cancelled));
    drop(_guard);
    let token = self::token();
    token.set_progress(Stage::Ready, 1, Some(1));
    let guard = work.commit().unwrap();
    token.claim_publication().unwrap();
    assert!(!token.cancel());
    assert!(performance.set_enabled(true).is_err());
    drop(guard);
    performance.set_enabled(true).unwrap();
    performance.set_enabled(false).unwrap();
    assert!(
        work.commit().is_err(),
        "quick protection cycle must invalidate old result"
    );
}

#[test]
fn optional_algorithms_observe_bounded_cancellation_and_match_existing_wave_geometry() {
    let data: Vec<_> = (0..150_000)
        .map(|n| (n as f32 * 0.07).sin() * 0.5)
        .collect();
    assert_eq!(
        dsp::peaks_3band_with_cancel(&data, 1, 2048, || false).unwrap(),
        dsp::peaks_3band(&data, 1, 2048)
    );
    let checks = std::cell::Cell::new(0);
    assert!(dsp::peaks_3band_with_cancel(&data, 1, 1, || {
        checks.set(checks.get() + 1);
        checks.get() >= 3
    })
    .is_none());
    assert_eq!(checks.get(), 3);
    let checks = std::cell::Cell::new(0);
    assert_eq!(
        dsp::estimate_bpm_with_cancel(&data, 1, 48000, || {
            checks.set(checks.get() + 1);
            checks.get() >= 3
        }),
        Err(())
    );
    assert_eq!(checks.get(), 3);
}

#[test]
fn replacement_during_hash_or_decode_is_rejected_and_packet_cancel_discards_pcm() {
    for boundary in [Stage::Hashing, Stage::Decoding] {
        let files = Files::new();
        let reference = files.source("track.wav", &wav(96_000, 48000, 2, false));
        let path = reference.path().unwrap().to_path_buf();
        let work = performance::Handle::default().optional_work().unwrap();
        let result = run_with(
            Request {
                reference,
                fields: Fields::ALL,
            },
            &token(),
            &work,
            |stage| {
                if stage == boundary {
                    std::fs::rename(&path, path.with_extension("retained-original")).unwrap();
                    std::fs::write(&path, wav(96_000, 48000, 2, false)).unwrap();
                }
            },
        );
        assert!(result.unwrap_err().to_string().contains("changed"));
    }
    let files = Files::new();
    let reference = files.source("long.wav", &wav(192_000, 48000, 2, false));
    let stop = std::cell::Cell::new(false);
    let decoded = std::cell::Cell::new(0);
    let failure = decode::decode_analysis_file(
        reference.path().unwrap(),
        std::fs::File::open(reference.path().unwrap()).unwrap(),
        || stop.get(),
        |frames, _| {
            decoded.set(frames);
            stop.set(true);
        },
    )
    .unwrap_err();
    assert_eq!(failure.kind, decode::DecodeFailureKind::Cancelled);
    assert!(decoded.get() > 0 && decoded.get() < 192_000);
}

#[test]
#[ignore = "local block filesystem UUID through production analysis resolver"]
fn local_block_volume_analysis_keeps_typed_source_and_same_descriptor_digest() {
    let home=PathBuf::from(std::env::var_os("HOME").unwrap());let files=Files(home.join(format!(".cache/omat-analysis-volume-{}",BankId::new().unwrap())));std::fs::create_dir(&files.0).unwrap();
    let bytes=wav(4096,16000,1,false);let mut reference=files.source("volume.wav",&bytes);let path=reference.path().unwrap().to_path_buf();
    let inventory=crate::media_location::Snapshot::fixture_local_volume(&path).unwrap();reference.source=inventory.identify(&path).unwrap().source;
    assert!(matches!(reference.source,LibSource::Removable {..}));let result=analyze(reference.clone(),Fields::ALL);
    assert_eq!(result.reference.source,reference.source);assert_eq!(result.reference.content_hash,Some(Sha256::digest(&bytes).into()));assert_eq!(result.reference.fingerprint,reference.fingerprint);
    assert_eq!(result.duration,4096.0/16000.0);assert!(result.waveform.is_some());
    std::fs::remove_file(path).unwrap();let result=run(Request {reference,fields:Fields::ALL},&token(),&performance::Handle::default().optional_work().unwrap());
    assert!(result.unwrap_err().to_string().contains("missing on the mounted volume"));
}
