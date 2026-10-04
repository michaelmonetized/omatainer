use super::*;
use crate::track_gain::{Analysis, Policy, Resolved};
use std::time::{Duration, Instant};

#[test]
fn paused_preview_small_fades_protection_and_wrong_measurements_refuse_trim() {
    for condition in 0..5 {
        let (_engine, mut rt) = Engine::headless_for_test(48_000, 80);
        let receipt = rt.decks[0].load_receipt.clone().unwrap();
        let mut level = receipt.source_level();
        match condition {
            0 => rt.decks[0].preview_position = Some(0.0),
            1 => rt.decks[0].last_output = [f32::MIN_POSITIVE, 0.0],
            2 => {
                rt.decks[0].transition_remaining = 1;
                rt.decks[0].transition_from = [f32::MIN_POSITIVE, 0.0];
            }
            3 => rt.apply(Command::PerformanceMode(true)),
            _ => level.as_mut().unwrap().samples += 1,
        }
        let gain = Resolved::prepare(Policy::Manual { db: -6.0 }, level).unwrap();
        let original = rt.decks[0].source_gain;
        let ack = beatgrid::GridEditAck::new();
        let counts = test_alloc::measure(|| {
            rt.apply(Command::DeckSourceGain {
                deck: 0,
                gain,
                receipt,
                ack: ack.clone(),
            })
        });
        assert_eq!(counts, test_alloc::Counts::default());
        assert_eq!(
            ack.state(),
            beatgrid::GridEditState::Rejected,
            "condition {condition}"
        );
        assert_eq!(rt.decks[0].source_gain, original);
    }
}

fn wav(amplitude: f32) -> Vec<u8> {
    let frames = 12_000u32;
    let mut bytes = Vec::with_capacity(44 + frames as usize * 8);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + frames * 8).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&3u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&48_000u32.to_le_bytes());
    bytes.extend_from_slice(&(48_000u32 * 8).to_le_bytes());
    bytes.extend_from_slice(&8u16.to_le_bytes());
    bytes.extend_from_slice(&32u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(frames * 8).to_le_bytes());
    for frame in 0..frames {
        let value = (amplitude * (frame as f32 * std::f32::consts::TAU * 440.0 / 48_000.0).sin())
            .clamp(-1.0, 1.0);
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[test]
fn actual_decode_analysis_gain_recall_and_prefader_output_preserve_pcm_and_fader() {
    for (name, amplitude) in [
        ("quiet", 0.001),
        ("loud", 0.5),
        ("clipped", 2.0),
        ("silence", 0.0),
    ] {
        let files = media_analysis::tests::Files::new();
        let mut reference = files.source(&format!("{name}.wav"), &wav(amplitude));
        let loader = media_load::Loader::start().unwrap();
        loader
            .request_source_expected(0, reference.source.clone(), Some(reference.fingerprint))
            .unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        let done = loop {
            if let Some(done) = loader.take_ready()[0].take() {
                break done;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        };
        let decoded = done.result.unwrap();
        let original = Arc::new(decoded.sample);
        let actual =
            crate::track_gain::measure_channels(&original.data, original.ch, || false).unwrap();
        assert_eq!(done.level, Some(actual));
        let mut store = crate::library::Store::open(files.0.join("catalog.json")).unwrap();
        store
            .catalog
            .upsert(
                reference.source.clone(),
                Some(reference.fingerprint),
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
        reference.track = store.catalog.track(&reference.source).unwrap().id.clone();
        let fields = crate::track_analysis::Fields {
            bpm: false,
            duration: false,
            waveform: false,
            level: true,
        };
        loader
            .request_analysis(media_analysis::Request {
                reference: reference.clone(),
                fields,
            })
            .unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        let prepared = loop {
            if let Some(done) = loader.take_analysis_ready() {
                break done.result.unwrap();
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(prepared.level, Some(Analysis::new(actual)));
        store
            .catalog
            .apply_analysis(&crate::track_analysis::Patch {
                reference: prepared.reference,
                fields,
                at_unix_ms: prepared.at_unix_ms,
                bpm: None,
                duration: prepared.duration,
                waveform: None,
                level: prepared.level,
            })
            .unwrap();
        assert_eq!(
            store
                .catalog
                .version(&reference.source, Some(reference.fingerprint))
                .unwrap()
                .preparation
                .source_gain,
            Policy::Off
        );
        let policy = Policy::Auto {
            target_dbfs: -18.0,
            peak_dbfs: -3.0,
        };
        let receipt = load_receipt::Receipt::with_preparation(Some(preparation::Preparation {
            source_gain: policy,
            ..Default::default()
        }))
        .with_source_level(done.level);
        if amplitude == 0.0 {
            assert!(receipt.is_err());
            assert!(prepared.level.unwrap().recommended_db.is_none());
            continue;
        }
        let receipt = receipt.unwrap();
        let (engine, mut rt) = Engine::headless_for_test(48_000, 80);
        let fader = rt.decks[0].gain;
        rt.apply(Command::DeckLoadRequested {
            deck: 0,
            media: load_receipt::Media::Decoded {
                token: done.token,
                audio: original.clone(),
            },
            receipt: receipt.clone(),
        });
        assert_eq!(receipt.state(), load_receipt::State::Current);
        assert_eq!(rt.decks[0].gain, fader);
        assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &original));
        let expected = Resolved::prepare(policy, Some(actual)).unwrap();
        assert_eq!(rt.decks[0].source_gain, expected);
        rt.decks[0].playing = true;
        let mut output = vec![0.0; original.data.len()];
        let counts = test_alloc::measure(|| {
            for frame in output.chunks_exact_mut(2) {
                rt.render_deck(0);
                frame.copy_from_slice(&rt.routing_deck_taps[0]);
            }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        let rendered = crate::track_gain::measure_channels(&output, 2, || false).unwrap();
        assert!(rendered.peak_dbfs.unwrap() <= -3.0 + 1e-5);
        assert!(
            (rendered.rms_dbfs.unwrap() - (actual.rms_dbfs.unwrap() + f64::from(expected.db())))
                .abs()
                < 0.01
        );
        assert_eq!(rt.decks[0].gain, fader);
        store.catalog.tracks[0].versions[0].preparation = receipt.preparation().unwrap().1;
        store.save().unwrap();
        drop(store);
        let reopened = crate::library::Store::open(files.0.join("catalog.json")).unwrap();
        let version = reopened
            .catalog
            .version(&reference.source, Some(reference.fingerprint))
            .unwrap();
        assert_eq!(version.preparation.source_gain, policy);
        assert_eq!(
            version
                .analysis
                .as_ref()
                .unwrap()
                .level
                .as_ref()
                .unwrap()
                .value,
            Analysis::new(actual)
        );
        assert_eq!(
            Resolved::prepare(version.preparation.source_gain, Some(actual)).unwrap(),
            expected
        );
        drop(engine);
    }
}
