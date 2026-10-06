use super::*;
use crate::engine::{
    audio, session, test_alloc, ClipKind, Command, Engine, PlayingClip, RtEngine, Sample,
};
use std::sync::Arc;

fn engine() -> RtEngine {
    let (_, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::Stop);
    rt.fx_wet.fill(0.0);
    for track in &mut rt.tracks {
        track.fx.slots.clear();
    }
    for chain in &mut rt.scene_fx {
        chain.slots.clear();
    }
    rt.tracks[0].kind = 4;
    rt.tracks[0].gain = 1.0;
    rt.tracks[0].clips[7].kind = ClipKind::Audio;
    rt.tracks[0].clips[7].audio = Some(Arc::new(Sample {
        spectrum: None,
        name: "Monitoring reference".into(),
        sr: 48000,
        ch: 2,
        data: vec![0.05; 48000],
        peaks: vec![].into(),
        bpm: 120.0,
        path: String::new(),
    }));
    rt.beat = 1.0;
    rt
}
fn clip(rt: &mut RtEngine, start: f64) {
    rt.tracks[0].playing = Some(PlayingClip {
        scene: 7,
        start_beat: start,
        midi_start_beat: start,
        last_beat: -1.0,
        looping: true,
    });
}
fn raw(rt: &mut RtEngine) -> [f32; 2] {
    rt.routing_track_input = Some([0.25; 2]);
    rt.render_track_cached(0, false);
    rt.routing_track_taps[0]
}

#[test]
fn actual_audio_clip_and_input_follow_arm_recording_pending_and_legacy_precedence() {
    for (mode, armed, recording, start, expected) in [
        (None, false, false, Some(0.0), 0.30),
        (Some(Mode::In), false, false, Some(0.0), 0.25),
        (Some(Mode::Off), true, true, Some(0.0), 0.05),
        (Some(Mode::Auto), true, false, Some(0.0), 0.05),
        (Some(Mode::Auto), true, false, None, 0.25),
        (Some(Mode::Auto), false, false, None, 0.0),
        (Some(Mode::Auto), true, true, Some(0.0), 0.25),
        (Some(Mode::Auto), true, false, Some(2.0), 0.25),
        (Some(Mode::Auto), false, true, Some(0.0), 0.05),
    ] {
        let mut rt = engine();
        rt.tracks[0].input_monitor = mode;
        rt.tracks[0].armed = armed;
        rt.recording = recording;
        if let Some(start) = start {
            clip(&mut rt, start);
        }
        for _ in 0..512 {
            raw(&mut rt);
        }
        for sample in raw(&mut rt) {
            assert!(
                (sample - expected).abs() < 0.000001,
                "{mode:?} {armed} {recording} {start:?}: {sample}"
            );
        }
    }
}

#[test]
fn input_mode_transition_has_a_finite_five_millisecond_ramp_without_callback_heap_work() {
    let mut rt = engine();
    clip(&mut rt, 0.0);
    assert!((raw(&mut rt)[0] - 0.30).abs() < 0.000001);
    rt.apply(Command::TrackMonitor {
        track: 0,
        mode: Mode::In,
    });
    let mut samples = [0.0; 240];
    assert_eq!(
        test_alloc::measure(|| {
            for sample in &mut samples {
                *sample = raw(&mut rt)[0];
            }
        }),
        test_alloc::Counts::default()
    );
    for (index, sample) in samples.into_iter().enumerate() {
        let expected = 0.25 + 0.05 * (1.0 - (index + 1) as f32 / 240.0);
        assert!(
            (sample - expected).abs() < 0.000002,
            "{index}: {sample} versus {expected}"
        );
    }
    assert_eq!(raw(&mut rt), [0.25; 2]);
}

#[test]
fn track_cue_is_prefader_additive_and_never_changes_program_in_either_renderer() {
    for graph in [false, true] {
        let setup = || {
            let mut rt = engine();
            clip(&mut rt, 0.0);
            rt.playing = true;
            rt.tracks[0].input_monitor = Some(Mode::Off);
            rt.tracks[0].gain = 0.0;
            rt.tracks[0].mute = true;
            rt.tracks[1].kind = 4;
            rt.tracks[1].clips[7] = rt.tracks[0].clips[7].clone();
            rt.tracks[1].playing = rt.tracks[0].playing;
            rt.tracks[1].input_monitor = Some(Mode::Off);
            rt.tracks[1].gain = 0.0;
            rt.tracks[1].mute = true;
            rt.monitor.output(&audio::config::Plan {
                backend: "ALSA".into(),
                device: "hw:CARD=NS7,DEV=0".into(),
                channels: 4,
                rate: 48000,
                format: cpal::SampleFormat::F32,
                buffer: Some(256),
                warning: None,
                graph: Default::default(),
            });
            if graph {
                let mut model = audio::routing::model::Model::default();
                model.version = 2;
                model.next_id = 3;
                model.ports.push(audio::routing::model::Port {
                    id: 2,
                    alias: "Cue".into(),
                    direction: audio::routing::model::Direction::Output,
                    channels: vec![2, 3],
                });
                model.monitor_output = Some(2);
                rt.routing = Some(Box::new(
                    audio::routing::prepared::Prepared::new(Arc::new(model), &rt.session).unwrap(),
                ));
            }
            rt.apply(Command::Monitor(crate::engine::monitor::Control::Volume(
                1.0,
            )));
            rt.apply(Command::Monitor(crate::engine::monitor::Control::Source(
                crate::engine::monitor::Source::Pfl,
            )));
            rt
        };
        let mut actual = setup();
        let mut reference = setup();
        actual.apply(Command::TrackPfl {
            track: 0,
            value: true,
        });
        actual.apply(Command::TrackPfl {
            track: 1,
            value: true,
        });
        let mut a = vec![0.0; 2048 * 4];
        let mut b = a.clone();
        actual.process_interleaved(&mut a, 4);
        reference.process_interleaved(&mut b, 4);
        for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
            assert_eq!(a[..2], b[..2], "graph {graph}");
        }
        for sample in &a[a.len() - 2..] {
            assert!(
                (*sample - crate::engine::limiter(0.1)).abs() < 0.00001,
                "{sample}, graph {graph}"
            );
        }
        assert_eq!(
            test_alloc::measure(|| actual.process_interleaved(&mut a, 4)),
            test_alloc::Counts::default()
        );
    }
}

#[test]
fn queued_input_controls_retain_track_identity_and_reject_replacement_without_heap_work() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    for command in [
        Command::TrackMonitor {
            track: 2,
            mode: Mode::In,
        },
        Command::TrackArm {
            track: 2,
            value: true,
        },
        Command::TrackPfl {
            track: 2,
            value: true,
        },
    ] {
        engine.send(command).unwrap();
    }
    let id = rt.session.tracks[2].id;
    let (request, _) = session::Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        session::Action::Move {
            axis: session::Axis::Track,
            id,
            position: 0,
        },
    )
    .unwrap();
    rt.apply(Command::SessionEdit(request));
    rt.process(&mut []);
    assert_eq!(rt.session.track_order[0], 2);
    assert_eq!(rt.tracks[2].input_monitor, Some(Mode::In));
    assert!(rt.tracks[2].armed && rt.tracks[2].pfl);
    assert_eq!(rt.tracks[0].input_monitor, None);
    assert!(!rt.tracks[0].armed && !rt.tracks[0].pfl);
    for command in [
        Command::TrackMonitor {
            track: 2,
            mode: Mode::Off,
        },
        Command::TrackArm {
            track: 2,
            value: true,
        },
        Command::TrackPfl {
            track: 2,
            value: true,
        },
    ] {
        engine.send(command).unwrap();
    }
    crate::engine::project::Prepared::empty(48000)
        .unwrap()
        .swap_into(&mut rt);
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(rt.tracks[2].input_monitor, None);
    assert!(!rt.tracks[2].armed && !rt.tracks[2].pfl);
}

#[test]
fn final_output_recording_keeps_armed_auto_audio_clip_audible(){
    use crate::engine::audio::routing::record::delivery::{Request,STANDARD_OUTPUT};
    let mut rt=engine();clip(&mut rt,0.0);rt.tracks[0].input_monitor=Some(Mode::Auto);rt.tracks[0].armed=true;for _ in 0..512{raw(&mut rt);}let expected=raw(&mut rt);assert_eq!(expected,[0.05;2]);let r=rt.routing_pipe.recorder.clone();let destination=std::env::temp_dir().join(format!("omatainer-output-monitor-{}",crate::sampler_bank::BankId::new().unwrap()));let path=destination.clone();let request=Request{alias:STANDARD_OUTPUT,output_channels:Some(vec![0,1]),options:crate::audio_delivery::Options::default(),seconds:1,epoch:r.epoch()};let worker=r.clone();let work=std::thread::spawn(move||worker.write_delivery(&request,&path,&std::sync::atomic::AtomicBool::new(false)));let end=std::time::Instant::now()+std::time::Duration::from_secs(15);while r.alias()==0{r.begin_delivery(48000);assert!(std::time::Instant::now()<end);std::thread::sleep(std::time::Duration::from_millis(1));}assert_eq!(raw(&mut rt),expected);assert!(!r.monitoring_inputs());r.converted(&expected,2);r.stop();assert_eq!(work.join().unwrap().unwrap().frames,1);assert_eq!(raw(&mut rt),expected);std::fs::remove_dir_all(destination).unwrap();
}
