use super::*;
use crate::engine::{dsp::Sample, Engine, RtEngine};
use std::sync::Arc;
struct Files(PathBuf);
impl Files {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "omatainer-audio-delivery-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn capture(engine: &Engine, rt: &mut RtEngine) -> Captured {
    let h = engine.project.clone();
    let job = std::thread::spawn(move || h.capture(&AtomicBool::new(false)).unwrap());
    let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !job.is_finished() {
        rt.process(&mut []);
        assert!(std::time::Instant::now() < end);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    job.join().unwrap()
}
fn source(rate: u32) -> Arc<Sample> {
    Arc::new(Sample {
        spectrum: None,
        name: "Independent stereo source".into(),
        sr: rate,
        ch: 2,
        data: (0..rate * 2)
            .flat_map(|i| {
                [
                    (i as f32 * 0.036).sin() * 0.2,
                    (i as f32 * 0.059).cos() * 0.4,
                ]
            })
            .collect(),
        peaks: Vec::new().into(),
        bpm: 120.0,
        path: String::new(),
    })
}
fn fixture(rate: u32) -> (Engine, RtEngine) {
    let (engine, mut rt) = Engine::headless_for_test(rate, 256);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: source(rate),
    });
    rt.quantize = false;
    rt.master = 0.25;
    for deck in &mut rt.decks {
        deck.sync = false;
        deck.keylock = false;
    }
    (engine, rt)
}

#[test]
fn offline_export_matches_actual_stereo_playback_ranges_repeats_and_rates() {
    let files = Files::new();
    for rate in [44100, 48000, 96000] {
        let (engine, mut rt) = fixture(rate);
        let captured = capture(&engine, &mut rt);
        let request = Export {
            source: Source::Session,
            decks: true,
            start: 0.01,
            end: 0.04,
            repeats: 3,
            tail: 0.0,
            options: Options {
                rate,
                ..Options::default()
            },
            ..Export::default()
        };
        let bounds = request.frames().unwrap();
        rt.apply(Command::Play);
        rt.apply(Command::DeckPlay { deck: 0 });
        rt.apply(Command::DeckPlay { deck: 1 });
        let mut actual = vec![0.0; ((bounds.0 + bounds.1) * 2) as usize];
        rt.process(&mut actual);
        let folder = files.0.join(format!("range-{rate}"));
        let outcome = run(
            captured,
            &request,
            &folder,
            &engine.cmd.performance().optional_work().unwrap(),
            &crate::background::Reporter::default(),
        )
        .unwrap();
        let decoded = crate::engine::decode::decode_audio(&folder.join("master.wav")).unwrap();
        assert_eq!(outcome.frames, bounds.3);
        assert_eq!(decoded.sample.frames() as u64, bounds.3);
        assert_eq!(decoded.sample.sr, rate);
        assert_eq!(decoded.sample.ch, 2);
        let expected = &actual[(bounds.0 * 2) as usize..];
        for repeated in decoded.sample.data.chunks_exact(expected.len()) {
            assert_eq!(repeated, expected, "export/playback differs at {rate}");
        }
        assert!(decoded.sample.data.iter().any(|v| v.abs() > 0.01));
        assert_ne!(decoded.sample.data[0], decoded.sample.data[1]);
        assert_eq!(rt.decks[0].pos, (bounds.0 + bounds.1) as f64);
        let receipt: serde_json::Value =
            serde_json::from_slice(&std::fs::read(folder.join("export.json")).unwrap()).unwrap();
        assert_eq!(receipt["frames"].as_u64(), Some(bounds.3));
    }
}

#[test]
fn mono_normalization_integer_formats_and_lossless_compressed_delivery_decode_correctly() {
    let files = Files::new();
    let raw = files.0.join("source.f32");
    let samples: Vec<f32> = (0..4096).map(|i| (i as f32 * 0.05).sin() * 0.1).collect();
    std::fs::write(
        &raw,
        samples
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    for format in Format::ALL {
        let options = Options {
            format,
            channels: 1,
            normalize: true,
            dither: format.dither(),
            ..Options::default()
        };
        let path = files.0.join(format!("{:?}.{}", format, format.extension()));
        let (peak, gain) = encode(
            &raw,
            &path,
            4096,
            0,
            1,
            &options,
            &AtomicBool::new(false),
            &crate::background::Reporter::default(),
        )
        .unwrap();
        assert!((peak - 0.1).abs() < 0.00001);
        assert!((gain * f64::from(peak) - 10_f64.powf(-1.0 / 20.0)).abs() < 1e-12);
        let decoded = crate::engine::decode::decode_audio(&path).unwrap();
        assert_eq!(decoded.sample.ch, 1);
        assert_eq!(decoded.sample.sr, 48000);
        if format == Format::Mp3 {
            assert!(decoded.sample.frames() >= 4096 && decoded.sample.frames() <= 4096 + 2304);
            assert!(decoded.sample.data.iter().any(|v| v.abs() > 0.5));
        } else {
            assert_eq!(decoded.sample.frames(), 4096);
            let tolerance = if matches!(format, Format::Pcm16 | Format::Flac16) {
                0.00007
            } else {
                0.0000003
            };
            for (&actual, &expected) in decoded.sample.data.iter().zip(&samples) {
                assert!(
                    (f64::from(actual) - f64::from(expected) * gain).abs() < tolerance,
                    "{format:?} altered lossless integer delivery"
                );
            }
        }
    }
}

#[test]
fn source_release_renders_native_tails_and_export_cancel_or_conflict_never_overwrites() {
    let files = Files::new();
    let (engine, mut rt) = fixture(48000);
    rt.apply(Command::FxWet {
        slot: 0,
        value: 0.5,
    });
    let captured = capture(&engine, &mut rt);
    let request = Export {
        source: Source::Session,
        decks: true,
        start: 0.0,
        end: 0.3,
        tail: 0.5,
        ..Export::default()
    };
    let path = files.0.join("tail");
    let permit = engine.cmd.performance().optional_work().unwrap();
    let o = run(
        captured,
        &request,
        &path,
        &permit,
        &crate::background::Reporter::default(),
    )
    .unwrap();
    assert_eq!(o.frames, 38400);
    let decoded = crate::engine::decode::decode_audio(&path.join("master.wav")).unwrap();
    assert!(decoded.sample.data[32000..]
        .iter()
        .any(|v| v.abs() > 0.00001));
    let before = std::fs::read(path.join("master.wav")).unwrap();
    let captured = capture(&engine, &mut rt);
    assert!(run(
        captured,
        &request,
        &path,
        &permit,
        &crate::background::Reporter::default()
    )
    .is_err());
    assert_eq!(before, std::fs::read(path.join("master.wav")).unwrap());
    drop(permit);
    let permit = engine.cmd.performance().optional_work().unwrap();
    permit.cancel().store(true, Ordering::Release);
    let captured = capture(&engine, &mut rt);
    let cancelled = files.0.join("cancelled");
    assert!(run(
        captured,
        &request,
        &cancelled,
        &permit,
        &crate::background::Reporter::default()
    )
    .unwrap_err()
    .contains("cancelled"));
    assert!(!cancelled.exists());
    assert_eq!(std::fs::read_dir(&files.0).unwrap().count(), 1);
}

#[test]
fn exact_routed_program_alias_preserves_channel_order_and_physical_input_cannot_disappear() {
    use crate::engine::audio::routing::model::{
        ChannelMap, Connection, Direction, Group, Model, Port, Source as RouteSource, Tap,
    };
    let files = Files::new();
    let (engine, mut rt) = fixture(48000);
    let mut captured = capture(&engine, &mut rt);
    let mut model = Model::for_output_channels(4);
    let output = model
        .ports
        .iter()
        .find(|p| p.direction == Direction::Output)
        .unwrap()
        .id;
    model
        .ports
        .iter_mut()
        .find(|p| p.id == output)
        .unwrap()
        .channels = vec![3, 2];
    captured.state.routing = Some(Arc::new(model.clone()));
    let mut reference = Prepared::from_state(captured.state.clone(), captured.media.clone(), 48000)
        .unwrap()
        .into_offline();
    reference.apply(Command::Play);
    reference.apply(Command::DeckPlay { deck: 0 });
    reference.apply(Command::DeckPlay { deck: 1 });
    let mut physical = vec![0.0; 480 * 4];
    reference.process_interleaved(&mut physical, 4);
    let request = Export {
        source: Source::Session,
        decks: true,
        output_alias: Some(output),
        end: 0.01,
        tail: 0.0,
        ..Export::default()
    };
    let folder = files.0.join("routed");
    run(
        captured,
        &request,
        &folder,
        &engine.cmd.performance().optional_work().unwrap(),
        &crate::background::Reporter::default(),
    )
    .unwrap();
    let decoded = crate::engine::decode::decode_audio(&folder.join("master.wav")).unwrap();
    assert_eq!(
        decoded.sample.data,
        physical
            .chunks_exact(4)
            .flat_map(|frame| [frame[3], frame[2]])
            .collect::<Vec<_>>()
    );
    assert!(decoded.sample.data.chunks_exact(2).any(|p| p[0] != p[1]));
    let mut captured = capture(&engine, &mut rt);
    let id = model.next_id;
    model.next_id += 1;
    model.ports.push(Port {
        id,
        alias: "Required physical return".into(),
        direction: Direction::Input,
        channels: vec![0, 1],
    });
    model.connections.push(Connection {
        source: RouteSource {
            group: Group::Input(id),
            tap: Tap::PostMixer,
        },
        destination: Group::Main,
        map: vec![ChannelMap {
            source: 0,
            destination: 0,
            gain: 1.0,
        }],
    });
    captured.state.routing = Some(Arc::new(model));
    let input = files.0.join("input");
    assert!(run(
        captured,
        &request,
        &input,
        &engine.cmd.performance().optional_work().unwrap(),
        &crate::background::Reporter::default()
    )
    .unwrap_err()
    .contains("physical input"));
    assert!(!input.exists());
    assert_eq!(std::fs::read_dir(&files.0).unwrap().count(), 1);
}

#[test]
fn invalid_delivery_choices_refuse_before_file_work() {
    let mut request = Export::default();
    request.start = request.end;
    assert!(request.frames().is_err());
    request = Export::default();
    request.repeats = 0;
    assert!(request.frames().is_err());
    request = Export::default();
    request.tail = f64::NAN;
    assert!(request.frames().is_err());
    request = Export::default();
    request.end = 28800.0;
    request.repeats = 64;
    assert!(request.frames().is_err());
    for options in [
        Options {
            format: Format::Mp3,
            channels: 4,
            ..Options::default()
        },
        Options {
            format: Format::Mp3,
            rate: 96000,
            ..Options::default()
        },
        Options {
            dither: true,
            ..Options::default()
        },
        Options {
            format: Format::Flac24,
            channels: 26,
            ..Options::default()
        },
    ] {
        assert!(options.validate().is_err());
    }
}
