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
fn arrangement_song_export_matches_realtime_overlaps_midi_and_five_minute_clock(){
    use crate::engine::{arrangement::{Model,Instance,Source as SongSource},audio_clip::Region,midi_edit::NoteId,MidiNote,ClipKind};
    let files=Files::new();
    for fading in [false,true] { for rate in [12000,44100,48000,96000]{
        let(engine,rt)=fixture(rate);let mut rt=Box::new(rt);rt.apply(Command::Stop);rt.bpm=120.0;
        if rate==48000 {use crate::engine::midi_data::{Conductor,Meter,Tempo,TimingSettings};rt.conductor=Some(Conductor::native(960,vec![Tempo::new(0,120.0,true).unwrap(),Tempo::new(1920,180.0,false).unwrap()],vec![Meter{tick:0,numerator:4,denominator_power:2,clocks:24,thirty_seconds:8}],TimingSettings::default()).unwrap());}
        let mut captured=capture(&engine,&mut rt);let audio=source(rate);let index=captured.media.len();captured.media.push(audio.clone());
        let mut clip=captured.state.tracks[0].clips[7].clone();clip.kind=ClipKind::Audio;clip.notes.clear();clip.region=None;clip.lanes=None;clip.audio=Some(index);let region=Region{start:64,end:u64::from(rate),loop_start:128,loop_end:u64::from(rate)-64,loop_enabled:true,reverse:true,transpose:3.0,tempo:120.0,fades:if fading {crate::engine::audio_clip::Fades{fade_in:0.1,fade_out:0.1,in_curve:-0.5,out_curve:0.5,automatic:true}}else{Default::default()}};clip.audio_region=Some(region);clip.bars=(region.prepare(&audio).unwrap().duration_beats/4.0)as f32;clip.gain=0.7;
        let mut midi=captured.state.tracks[1].clips[7].clone();midi.kind=ClipKind::Midi;midi.bars=1.0;midi.notes=vec![MidiNote{id:NoteId::new(),muted:false,pitch:64,vel:90,channel:3,release_vel:9,source_timing:None,start:0.0,len:1.75}];
        let layout=captured.state.session.as_ref().unwrap();let t0=layout.reference(crate::engine::session::Axis::Track,0).unwrap();let t1=layout.reference(crate::engine::session::Axis::Track,1).unwrap();let duration=if rate==12000{600.0}else{2.0};
        let model=Model{enabled:true,next_id:6,sources:vec![SongSource{id:1,clip},SongSource{id:2,clip:midi}],instances:vec![Instance{id:3,source:1,track:t0,start:0.0,offset:0.0,duration,repeating:true,gain:0.8,fades:None,fade_link:0,crossfade:None},Instance{id:4,source:1,track:t0,start:0.25,offset:0.75,duration:duration-0.25,repeating:true,gain:0.4,fades:if fading {Some(crate::engine::audio_clip::Fades{fade_in:0.2,fade_out:0.2,in_curve:0.5,out_curve:-0.5,automatic:true})}else{None},fade_link:0,crossfade:None},Instance{id:5,source:2,track:t1,start:0.0,offset:0.0,duration,repeating:true,gain:0.35,fades:None,fade_link:0,crossfade:None}]};
        let(request,ack)=crate::engine::arrangement::edit::Request::prepare(captured,model,&AtomicBool::new(false)).unwrap();assert_eq!(crate::engine::test_alloc::measure(||rt.apply(crate::engine::Command::ArrangementEdit(request))),Default::default());assert_eq!(ack.state(),crate::engine::midi_edit::Outcome::Applied);
        let captured=capture(&engine,&mut rt);let request=Export{source:Source::Arrangement,decks:false,start:0.0,end:duration/2.0,repeats:1,tail:0.0,options:Options{rate,..Options::default()},..Export::default()};let bounds=request.frames().unwrap();let mut actual=vec![0.0;(bounds.1*2)as usize];rt.apply(Command::Play);assert_eq!(crate::engine::test_alloc::measure(||rt.process(&mut actual)),Default::default());assert!((rt.timeline_seconds()-duration/2.0).abs()<1e-9);
        let folder=files.0.join(format!("arrangement-{rate}-{fading}"));let outcome=run(captured,&request,&folder,&engine.cmd.performance().optional_work().unwrap(),&crate::background::Reporter::default()).unwrap();let decoded=crate::engine::decode::decode_audio(&folder.join("master.wav")).unwrap();assert_eq!(outcome.frames,bounds.1);assert_eq!(decoded.sample.data,actual,"Arrangement realtime/export differs at {rate}");assert!(actual.iter().any(|v|v.abs()>0.001));
    } }
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
