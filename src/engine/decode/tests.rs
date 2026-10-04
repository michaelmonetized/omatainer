use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
use symphonia::core::io::Monitor;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new(extension: &str, bytes: &[u8]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "omatainer-decode-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join(format!("fixture.{extension}"));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }
    fn decode(&self) -> Result<DecodedAudio, DecodeFailure> {
        decode_audio(&self.0)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
    }
}

fn wav(frames: usize, sr: u32, ch: u16) -> Vec<u8> {
    let mut bytes = Vec::new();
    let data_len = frames as u32 * ch as u32 * 2;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&ch.to_le_bytes());
    bytes.extend_from_slice(&sr.to_le_bytes());
    bytes.extend_from_slice(&(sr * ch as u32 * 2).to_le_bytes());
    bytes.extend_from_slice(&(ch * 2).to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for frame in 0..frames {
        for channel in 0..ch {
            bytes.extend_from_slice(
                &(((frame % 257) as i16 - 128) * (channel as i16 + 1) * 64).to_le_bytes(),
            );
        }
    }
    bytes
}

#[test]
fn deck_decode_refuses_declared_pcm_over_its_budget_before_decoding_or_allocating_samples() {
    let mut bytes=wav(1,48000,2);
    let declared=512u32*1024*1024;
    bytes[4..8].copy_from_slice(&(36+declared).to_le_bytes());
    bytes[40..44].copy_from_slice(&declared.to_le_bytes());
    let fixture=Fixture::new("wav",&bytes);
    let error=decode_deck_file_progress(&fixture.0,std::fs::File::open(&fixture.0).unwrap(),||false,&super::super::performance::Handle::default(),|_,_|{}).unwrap_err();
    assert_eq!(error.kind,DecodeFailureKind::Capacity,"{error}");
    assert_eq!(error.stage,DecodeStage::CreateDecoder);
    assert_eq!(error.diagnostics.decoded_frames,0);
    assert_eq!(error.diagnostics.decoded_packets,0);
    assert_eq!(error.diagnostics.expected_frames,Some(u64::from(declared)/4));
}

#[test]
fn pcm_eof_preserves_exact_duration_channels_and_samples() {
    for (sr, ch) in [(8000, 1), (44100, 2), (96000, 2)] {
        let frames = sr as usize / 4;
        let report = Fixture::new("wav", &wav(frames, sr, ch)).decode().unwrap();
        assert_eq!(report.sample.frames(), frames);
        assert_eq!(report.sample.sr, sr);
        assert_eq!(report.sample.ch, ch);
        assert_eq!(report.diagnostics.expected_frames, Some(frames as u64));
        assert_eq!(report.diagnostics.length_evidence, LengthEvidence::Declared);
        assert!(report.diagnostics.reached_eof);
        assert_eq!(report.diagnostics.warning(), None);
        for (i, value) in report.sample.data.iter().enumerate() {
            let frame = i / ch as usize;
            let channel = i % ch as usize;
            let expected = ((frame % 257) as i16 - 128) * (channel as i16 + 1) * 64;
            assert_eq!(*value, expected as f32 / 32768.0);
        }
    }
}

#[test]
fn truncated_pcm_never_returns_a_decoded_prefix_as_success() {
    let bytes = wav(12000, 48000, 2);
    for missing in [1, 4, 2000, 44000, 47999, 48000] {
        let error = Fixture::new("wav", &bytes[..bytes.len() - missing])
            .decode()
            .unwrap_err();
        assert_eq!(error.kind, DecodeFailureKind::Incomplete, "{error}");
        assert_eq!(error.diagnostics.expected_frames, Some(12000));
        assert!(error.diagnostics.decoded_frames < 12000);
        assert!(error.to_string().contains("media was not loaded"));
    }
}

const FLAC: &[u8] = include_bytes!("../../../tests/fixtures/audio/tone.flac");
const OGG: &[u8] = include_bytes!("../../../tests/fixtures/audio/tone.ogg");
const NEXT_OGG: &[u8] = include_bytes!("../../../tests/fixtures/audio/tone-next.ogg");
const LONG_OGG: &[u8] = include_bytes!("../../../tests/fixtures/audio/tone-long.ogg");

fn damage_last_ogg_page(header_packet: bool) -> Vec<u8> {
    let mut bytes = LONG_OGG.to_vec();
    let mut page = 0;
    let (start, body) = loop {
        assert_eq!(&bytes[page..page + 4], b"OggS");
        let segments = bytes[page + 26] as usize;
        let body = page + 27 + segments;
        let length: usize = bytes[page + 27..body].iter().map(|&n| n as usize).sum();
        if body + length == bytes.len() {
            break (page, body);
        }
        page = body + length;
    };
    // Keep the container valid (including CRC) so the real Vorbis reader or
    // decoder sees malformed packet content after earlier audio was decoded.
    bytes[body..].fill(0xff);
    bytes[body] = if header_packet { 1 } else { 0 };
    bytes[start + 22..start + 26].fill(0);
    let mut crc = 0u32;
    for byte in &bytes[start..] {
        crc ^= (*byte as u32) << 24;
        for _ in 0..8 {
            crc = (crc << 1)
                ^ if crc & 0x8000_0000 != 0 {
                    0x04c1_1db7
                } else {
                    0
                };
        }
    }
    bytes[start + 22..start + 26].copy_from_slice(&crc.to_le_bytes());
    bytes
}

#[test]
fn real_vorbis_packet_damage_cannot_hide_shortened_content() {
    for (header_packet, expected) in [
        (false, DecodeFailureKind::Incomplete),
        (true, DecodeFailureKind::Incomplete),
    ] {
        let error = Fixture::new("ogg", &damage_last_ogg_page(header_packet))
            .decode()
            .unwrap_err();
        assert_eq!(error.kind, expected, "{error}");
        assert!(error.diagnostics.decoded_frames > 0, "{error}");
    }
}

#[test]
fn recoverable_flac_packet_error_is_refused_instead_of_skipped() {
    let mut bytes = FLAC.to_vec();
    // The fixed fixture's second frame has a 6-byte header; mark its first
    // subframe as reserved. Recompute frame CRC so demuxing succeeds and the
    // actual codec sees the invalid subframe after 4096 valid PCM frames.
    let start = 10540;
    let end = 12870;
    bytes[start + 6] = 0x04;
    let mut crc = symphonia::core::checksum::Crc16Ansi::new(0);
    crc.process_buf_bytes(&bytes[start..end - 2]);
    bytes[end - 2..end].copy_from_slice(&crc.crc().to_be_bytes());
    let error = Fixture::new("flac", &bytes).decode().unwrap_err();
    assert_eq!(
        error.kind,
        DecodeFailureKind::RecoverablePacketDamage,
        "{error}"
    );
    assert_eq!(error.stage, DecodeStage::DecodePacket);
    assert_eq!(error.diagnostics.decoded_frames, 4096);
}

#[test]
fn real_compressed_formats_decode_content_and_report_length_evidence() {
    for (extension, bytes) in [
        ("flac", FLAC),
        ("ogg", OGG),
        (
            "mp3",
            include_bytes!("../../../tests/fixtures/audio/tone.mp3").as_slice(),
        ),
        (
            "m4a",
            include_bytes!("../../../tests/fixtures/audio/tone.m4a").as_slice(),
        ),
        (
            "mp3",
            include_bytes!("../../../tests/fixtures/audio/tone-estimated.mp3").as_slice(),
        ),
    ] {
        let report = Fixture::new(extension, bytes).decode().unwrap();
        assert_eq!(report.sample.sr, 48000);
        assert_eq!(report.sample.ch, 2);
        assert!((12000..=14304).contains(&report.sample.frames()));
        assert!(report.sample.data.iter().any(|x| x.abs() > 0.2));
        // The synthetic fixture is 400 Hz on L and 800 Hz on R. Check actual
        // decoded content with a phase-insensitive projection, including lossy
        // codec priming/padding, instead of only trusting metadata and length.
        let power = |channel: usize, hz: f64| {
            let (mut real, mut imaginary) = (0.0, 0.0);
            for (frame, pair) in report.sample.data.chunks_exact(2).enumerate() {
                let angle = std::f64::consts::TAU * hz * frame as f64 / 48000.0;
                real += pair[channel] as f64 * angle.cos();
                imaginary += pair[channel] as f64 * angle.sin();
            }
            real * real + imaginary * imaginary
        };
        assert!(power(0, 400.0) > 100.0 * power(0, 800.0));
        assert!(power(1, 800.0) > 100.0 * power(1, 400.0));
        assert!(report.diagnostics.reached_eof);
        if extension == "mp3" {
            assert!(matches!(
                report.diagnostics.length_evidence,
                LengthEvidence::Estimated | LengthEvidence::Unavailable
            ));
            assert!(report.diagnostics.warning().is_some());
        } else {
            assert!(report.diagnostics.warning().is_none());
        }
        if extension == "flac" {
            assert_eq!(report.sample.frames(), 12000);
            assert_eq!(report.diagnostics.verification, Some(true));
        }
    }
}

#[test]
fn chained_ogg_reports_reset_instead_of_loading_only_the_first_stream() {
    let chained: Vec<_> = LONG_OGG.iter().chain(NEXT_OGG).copied().collect();
    let error = Fixture::new("ogg", &chained).decode().unwrap_err();
    assert_eq!(error.kind, DecodeFailureKind::ResetRequired, "{error}");
    assert_eq!(error.stage, DecodeStage::ReadPacket);
    assert!(error.diagnostics.decoded_frames > 48000, "{error}");
    assert!(error
        .to_string()
        .contains("chained streams are unsupported"));
}

#[test]
fn flac_truncation_and_checksum_damage_are_explicit_failures() {
    let error = Fixture::new("flac", &FLAC[..FLAC.len() - 200])
        .decode()
        .unwrap_err();
    assert!(
        matches!(
            error.kind,
            DecodeFailureKind::Incomplete | DecodeFailureKind::RecoverablePacketDamage
        ),
        "{error}"
    );
    assert!(error.diagnostics.decoded_frames < 12000);
    // Native FLAC STREAMINFO starts after the 4-byte marker + block header;
    // its final 16 bytes are the decoded-PCM MD5. Corrupt only this checksum.
    let mut corrupted = FLAC.to_vec();
    corrupted[26] ^= 1;
    let error = Fixture::new("flac", &corrupted).decode().unwrap_err();
    assert_eq!(error.kind, DecodeFailureKind::VerificationFailed, "{error}");
    assert_eq!(error.diagnostics.decoded_frames, 12000);
    assert_eq!(error.diagnostics.verification, Some(false));
}

#[test]
fn malformed_empty_and_missing_files_surface_errors() {
    let garbage = Fixture::new("wav", b"not an audio file")
        .decode()
        .unwrap_err();
    assert_eq!(garbage.stage, DecodeStage::Probe);
    let empty = Fixture::new("wav", &wav(0, 48000, 2)).decode().unwrap_err();
    assert_eq!(empty.kind, DecodeFailureKind::Empty);
    let missing = Fixture::new("wav", &wav(1, 48000, 2));
    std::fs::remove_file(&missing.0).unwrap();
    let error = missing.decode().unwrap_err();
    assert_eq!(error.kind, DecodeFailureKind::Io);
    assert_eq!(error.stage, DecodeStage::Open);
}

#[test]
fn cancellation_discards_audio_at_every_cooperative_boundary() {
    let fixture = Fixture::new("wav", &wav(12000, 48000, 2));
    let calls = std::cell::Cell::new(0);
    decode_audio_with_cancel(&fixture.0, || {
        calls.set(calls.get() + 1);
        false
    })
    .unwrap();
    let total = calls.get();
    assert!(total > 15);
    for stop_at in 1..=total {
        calls.set(0);
        let error = decode_audio_with_cancel(&fixture.0, || {
            calls.set(calls.get() + 1);
            calls.get() == stop_at
        })
        .unwrap_err();
        assert_eq!(error.kind, DecodeFailureKind::Cancelled);
        assert_eq!(calls.get(), stop_at);
    }
}

#[test]
fn truncated_streams_without_reliable_duration_are_rejected_or_explicitly_labelled() {
    for (extension, bytes) in [
        ("ogg", LONG_OGG),
        (
            "mp3",
            include_bytes!("../../../tests/fixtures/audio/tone-estimated.mp3").as_slice(),
        ),
    ] {
        let complete = Fixture::new(extension, bytes).decode().unwrap();
        match Fixture::new(extension, &bytes[..bytes.len() - 200]).decode() {
            Ok(report) => {
                assert!(report.sample.frames() < complete.sample.frames());
                assert!(
                    report.diagnostics.warning().is_some(),
                    "quiet partial {extension}: {:?}",
                    report.diagnostics
                );
            }
            Err(error) => assert!(
                matches!(
                    error.kind,
                    DecodeFailureKind::Incomplete
                        | DecodeFailureKind::RecoverablePacketDamage
                        | DecodeFailureKind::Empty
                        | DecodeFailureKind::Fatal
                ),
                "{error}"
            ),
        }
    }
}

#[test]
fn performance_decode_keeps_actual_pcm_but_defers_optional_bpm_analysis() {
    let file = Fixture::new("wav", &wav(96_000, 48_000, 2));
    let normal = file.decode().unwrap();
    let performance = super::super::performance::Handle::default();
    performance.set_enabled(true).unwrap();
    let protected = decode_audio_for_show(&file.0, || false, &performance).unwrap();
    assert_eq!(protected.sample.data, normal.sample.data);
    assert_eq!(protected.sample.peaks, normal.sample.peaks);
    assert_eq!(protected.sample.bpm, 0.0);
    assert!(protected.diagnostics.analysis_deferred);
    assert!(protected.diagnostics.warning().unwrap().contains("deferred"));
    performance.set_enabled(false).unwrap();
    // The analysis loop cooperates during its bounded sample hops, not only at
    // entry or after a whole long file's correlation pass.
    let seen = std::cell::Cell::new(0);
    let result = super::super::dsp::detect_bpm_with_cancel(&normal.sample.data, 2, 48_000, || {
        let n = seen.get() + 1; seen.set(n); n > 5
    });
    assert_eq!(result, None);
    assert_eq!(seen.get(), 6);
}
