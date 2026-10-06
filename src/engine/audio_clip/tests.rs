use super::*;
fn source() -> Sample {
    Sample {
        name: "region".into(),
        sr: 100,
        ch: 1,
        data: vec![90.0, 1.0, 2.0, 3.0, 4.0, 5.0, 90.0],
        peaks: Default::default(),
        spectrum: None,
        bpm: 60.0,
        path: String::new(),
    }
}
fn trim() -> Region {
    Region {
        start: 1,
        end: 6,
        loop_start: 2,
        loop_end: 4,
        loop_enabled: false,
        reverse: false,
        transpose: 0.0,
        tempo: 60.0,
    }
}
#[test]
fn half_open_trim_and_reverse_never_interpolate_adjacent_material() {
    let source = source();
    for reverse in [false, true] {
        let plan = Region { reverse, ..trim() }.prepare(&source).unwrap();
        assert_eq!(
            plan.sample(&source, 0.0, false),
            [if reverse { 5.0 } else { 1.0 }; 2]
        );
        assert_eq!(
            plan.sample(&source, 0.045, false),
            [if reverse { 1.0 } else { 5.0 }; 2]
        );
        assert_eq!(plan.sample(&source, 0.05, false), [0.0; 2]);
        assert_eq!(plan.progress(0.05, false), if reverse { 0.0 } else { 1.0 });
        assert_eq!(plan.sample(&source, f64::NAN, true), [0.0; 2]);
        assert_eq!(plan.sample(&source, -1.0, true), [0.0; 2]);
    }
}
#[test]
fn forward_reverse_intro_and_loops_keep_their_own_interpolation_bounds() {
    let source = source();
    let forward = Region {
        loop_enabled: true,
        ..trim()
    }
    .prepare(&source)
    .unwrap();
    assert_eq!(forward.sample(&source, 0.025, true), [3.0; 2]);
    assert_eq!(forward.sample(&source, 0.03, true), [2.0; 2]);
    assert_eq!(forward.sample(&source, 0.045, true), [3.0; 2]);
    assert_eq!(forward.sample(&source, 0.05, true), [2.0; 2]);
    let reverse = Region {
        loop_enabled: true,
        reverse: true,
        ..trim()
    }
    .prepare(&source)
    .unwrap();
    assert_eq!(reverse.sample(&source, 0.035, true), [2.0; 2]);
    assert_eq!(reverse.sample(&source, 0.04, true), [3.0; 2]);
    assert_eq!(reverse.sample(&source, 0.045, true), [2.5; 2]);
    assert_eq!(reverse.sample(&source, 0.055, true), [2.0; 2]);
    assert_eq!(reverse.sample(&source, 0.06, true), [3.0; 2]);
}

use crate::engine::{
    project::{Captured, Prepared, State},
    test_alloc, Command, Engine, RtEngine,
};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::{Duration, Instant};
fn capture(engine: &Engine, rt: &mut RtEngine) -> Captured {
    let handle = engine.project.clone();
    let job = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(15);
    while !job.is_finished() {
        rt.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    job.join().unwrap()
}
fn pcm(rate: u32) -> Arc<Sample> {
    Arc::new(Sample {
        name: "Shared stereo".into(),
        sr: rate,
        ch: 2,
        data: (0..rate)
            .flat_map(|i| {
                [
                    0.1 + (i as f32 * 0.041).sin() * 0.05,
                    -0.2 + (i as f32 * 0.053).cos() * 0.05,
                ]
            })
            .collect(),
        peaks: Default::default(),
        spectrum: None,
        bpm: 120.0,
        path: String::new(),
    })
}
fn edit(
    engine: &Engine,
    rt: &mut RtEngine,
    track: u8,
    source: Arc<Sample>,
    region: Region,
) -> (Box<edit::Request>, crate::engine::midi_edit::Ack) {
    let captured = capture(engine, rt);
    let document = edit::Document::capture(&captured, track, 7).unwrap();
    edit::Request::prepare(
        captured,
        document,
        source,
        region,
        "Edited source".into(),
        0.75,
        &AtomicBool::new(false),
    )
    .unwrap()
}
#[test]
fn prepared_audio_source_edits_and_undo_redo_are_atomic_without_renderer_heap_work() {
    let (engine, live) = Engine::headless_for_test(48000, 256);
    let mut live = Box::new(live);
    let source = pcm(48000);
    let region = Region {
        start: 1200,
        end: 24000,
        loop_start: 2400,
        loop_end: 12000,
        loop_enabled: true,
        reverse: true,
        transpose: 7.0,
        tempo: 120.0,
    };
    let (request, ack) = edit(&engine, &mut live, 0, source.clone(), region);
    let before = engine.undo.view().cursor;
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::AudioClipEdit(request))),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
    assert!(Arc::ptr_eq(
        live.tracks[0].clips[7].audio.as_ref().unwrap(),
        &source
    ));
    assert_eq!(live.tracks[0].clips[7].audio_region.unwrap().region, region);
    live.process(&mut []);
    assert_eq!(engine.undo.view().cursor, before + 1);
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::Undo)),
        test_alloc::Counts::default()
    );
    assert_eq!(live.tracks[0].clips[7].kind, crate::engine::ClipKind::Empty);
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::Redo)),
        test_alloc::Counts::default()
    );
    assert_eq!(live.tracks[0].clips[7].audio_region.unwrap().region, region);
    assert!(Arc::ptr_eq(
        live.tracks[0].clips[7].audio.as_ref().unwrap(),
        &source
    ));
    let other = Region {
        reverse: false,
        transpose: -12.0,
        ..region
    };
    let (request, ack) = edit(&engine, &mut live, 1, source.clone(), other);
    live.apply(Command::AudioClipEdit(request));
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
    assert!(Arc::ptr_eq(
        live.tracks[0].clips[7].audio.as_ref().unwrap(),
        live.tracks[1].clips[7].audio.as_ref().unwrap()
    ));
    assert_ne!(
        live.tracks[0].clips[7].audio_region.unwrap().region,
        live.tracks[1].clips[7].audio_region.unwrap().region
    );
    let saved = capture(&engine, &mut live);
    assert_eq!(
        saved.state.tracks[0].clips[7].audio,
        saved.state.tracks[1].clips[7].audio
    );
}
#[test]
fn cancelled_stale_active_protected_and_over_budget_audio_edits_preserve_the_clip() {
    for refusal in 0..6 {
        let (engine, live) = Engine::headless_for_test(48000, 256);
        let mut live = Box::new(live);
        let source = pcm(48000);
        let (request, ack) = edit(
            &engine,
            &mut live,
            0,
            source.clone(),
            Region::full(&source, 120.0).unwrap(),
        );
        match refusal {
            0 => {
                assert!(ack.cancel());
            }
            1 => {
                live.session.namespace[0] ^= 1;
            }
            2 => live.apply(Command::TrackGain {
                track: 1,
                value: 0.42,
            }),
            3 => {
                live.tracks[0].playing = Some(crate::engine::PlayingClip {
                    scene: 7,
                    start_beat: 0.0,
                    midi_start_beat: 0.0,
                    last_beat: -1.0,
                    looping: true,
                });
            }
            4 => live.apply(Command::PerformanceMode(true)),
            5 => live.set_undo_budget_for_test(1024),
            _ => unreachable!(),
        }
        assert_eq!(
            test_alloc::measure(|| live.apply(Command::AudioClipEdit(request))),
            test_alloc::Counts::default(),
            "refusal {refusal}"
        );
        assert_eq!(live.tracks[0].clips[7].kind, crate::engine::ClipKind::Empty);
        assert_eq!(
            ack.state(),
            if refusal == 0 {
                crate::engine::midi_edit::Outcome::Cancelled
            } else {
                crate::engine::midi_edit::Outcome::Rejected
            }
        );
    }
}
#[test]
fn source_regions_reopen_at_other_rates_and_legacy_schemas_refuse_hidden_region_data() {
    let (engine, live) = Engine::headless_for_test(48000, 256);
    let mut live = Box::new(live);
    let source = pcm(48000);
    let region = Region {
        start: 12,
        end: 24000,
        loop_start: 200,
        loop_end: 22000,
        loop_enabled: true,
        reverse: true,
        transpose: 3.5,
        tempo: 131.0,
    };
    let (request, _) = edit(&engine, &mut live, 0, source, region);
    live.apply(Command::AudioClipEdit(request));
    let saved = capture(&engine, &mut live);
    let path = std::env::temp_dir().join(format!(
        "omat-audio-region-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    crate::project_file::save(
        &path,
        &crate::project_file::Bundle {
            state: saved.state.clone(),
            media: saved.media.clone(),
        },
        crate::project_file::Overwrite::Never,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let read =
        crate::project_file::load::<State>(&path, &Default::default(), &AtomicBool::new(false))
            .unwrap();
    std::fs::remove_file(&path).unwrap();
    for rate in [44100, 48000, 96000] {
        let reopened = Prepared::from_state(read.state.clone(), read.media.clone(), rate).unwrap();
        assert_eq!(
            reopened.rt.tracks[0].clips[7].audio_region.unwrap().region,
            region
        );
        assert_eq!(reopened.rt.tracks[0].clips[7].gain, 0.75);
        assert_eq!(
            reopened.rt.tracks[0].clips[7].audio.as_ref().unwrap().data,
            live.tracks[0].clips[7].audio.as_ref().unwrap().data
        );
    }
    let mut value = serde_json::to_value(&saved.state).unwrap();
    value["version"] = 17.into();
    assert!(serde_json::from_value::<State>(value.clone()).is_err());
    value["tracks"][0]["clips"][7]["audio_region"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<State>(value.clone()).is_err());
    value["tracks"][0]["clips"][7]
        .as_object_mut()
        .unwrap()
        .remove("audio_region");
    value["tracks"][0]["clips"][7]["bars"] = 1.0.into();
    let old: State = serde_json::from_value(value).unwrap();
    old.validate(&saved.media).unwrap();
    assert!(Prepared::from_state(old, saved.media, 48000)
        .unwrap()
        .rt
        .tracks[0]
        .clips[7]
        .audio_region
        .is_none());
}
#[test]
fn actual_track_audio_respects_trim_pitch_reverse_gain_input_monitor_and_half_open_duration() {
    for rate in [44100, 48000, 96000] {
        for (reverse, transpose) in [(false, 0.0), (true, 0.0), (false, 12.0), (true, -12.0)] {
            let (engine, live) = Engine::headless_for_test(rate, 256);
            let mut live = Box::new(live);
            live.apply(Command::Stop);
            live.bpm = 120.0;
            live.quant = 0.0;
            let source = pcm(rate);
            let region = Region {
                start: 300,
                end: 1300,
                loop_start: 300,
                loop_end: 1300,
                loop_enabled: false,
                reverse,
                transpose,
                tempo: 120.0,
            };
            let plan = region.prepare(&source).unwrap();
            let (request, ack) = edit(&engine, &mut live, 0, source.clone(), region);
            live.apply(Command::AudioClipEdit(request));
            assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
            live.tracks[0].fx.slots.clear();
            live.tracks[0].input_monitor = Some(crate::engine::input_monitor::Mode::Off);
            live.tracks[0].gain = 1.0;
            live.tracks[0].kind = 4;
            live.apply(Command::FireClip {
                track: 0,
                scene: 7,
                looping: false,
            });
            let step = 2.0 / f64::from(rate);
            let output_frames = (1000.0 / 2_f64.powf(f64::from(transpose) / 12.0)) as usize;
            for frame in 0..output_frames {
                live.beat = (frame + 1) as f64 * step;
                live.render_track(0, false);
                let actual = live.routing_track_taps[0];
                let expected = plan
                    .sample(&source, frame as f64 * step, false)
                    .map(|v| v * 0.75);
                for ch in 0..2 {
                    assert!(
                        (actual[ch] - expected[ch]).abs() < 0.000001,
                        "{rate}/{reverse}/{transpose}/frame {frame}: {actual:?} != {expected:?}"
                    );
                }
            }
            live.beat = (output_frames + 1) as f64 * step;
            live.render_track(0, false);
            assert_eq!(live.routing_track_taps[0], [0.0; 2]);
            assert!(live.tracks[0].playing.is_none());
            live.tracks[0].input_monitor = Some(crate::engine::input_monitor::Mode::In);
            live.apply(Command::FireClip {
                track: 0,
                scene: 7,
                looping: true,
            });
            for _ in 0..512 {
                live.beat += step;
                live.routing_track_input = Some([0.3, -0.4]);
                live.render_track(0, false);
            }
            assert_eq!(live.routing_track_taps[0], [0.3, -0.4]);
        }
    }
}
#[test]
fn audio_regions_render_multiple_shared_instances_through_devices_routes_pfl_and_export() {
    use crate::engine::audio::routing::{model::*, prepared};
    for rate in [44100, 48000, 96000] {
        let (engine, live) = Engine::headless_for_test(rate, 256);
        let mut live = Box::new(live);
        live.apply(Command::Stop);
        live.bpm = 120.0;
        live.quant = 0.0;
        live.master = 0.25;
        live.fx_wet.fill(0.0);
        for track in &mut live.tracks {
            track.fx.slots.clear();
        }
        for chain in &mut live.scene_fx {
            chain.slots.clear();
        }
        let source = pcm(rate);
        for (track, reverse, transpose) in [(0, false, 0.0), (1, true, -12.0)] {
            let region = Region {
                start: 200,
                end: u64::from(rate) / 2,
                loop_start: 1000,
                loop_end: u64::from(rate) / 3,
                loop_enabled: true,
                reverse,
                transpose,
                tempo: 120.0,
            };
            let (request, ack) = edit(&engine, &mut live, track, source.clone(), region);
            live.apply(Command::AudioClipEdit(request));
            assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
            let track = &mut live.tracks[usize::from(track)];
            track.kind = 4;
            track.gain = 0.8;
            track.input_monitor = Some(crate::engine::input_monitor::Mode::Off);
            let mut dist =
                crate::engine::fx::FxSlot::new(crate::engine::fx::FxId::Dist, rate as f32);
            dist.mix = 1.0;
            dist.p[0] = 0.25;
            track.fx.slots.push(dist);
        }
        let mut model = Model::default();
        model.version = 2;
        model.next_id = 3;
        model.ports.push(Port {
            id: 2,
            alias: "Headphones".into(),
            direction: Direction::Output,
            channels: vec![2, 3],
        });
        model.monitor_output = Some(2);
        live.routing = Some(Box::new(
            prepared::Prepared::new(Arc::new(model), &live.session).unwrap(),
        ));
        live.apply(Command::TrackPfl {
            track: 0,
            value: true,
        });
        live.apply(Command::Monitor(crate::engine::monitor::Control::Volume(
            1.0,
        )));
        live.apply(Command::Monitor(crate::engine::monitor::Control::Source(
            crate::engine::monitor::Source::Pfl,
        )));
        live.apply(Command::LaunchScene { scene: 7 });
        let captured = capture(&engine, &mut live);
        let export = crate::audio_delivery::Export {
            source: crate::audio_delivery::Source::Session,
            scene: 7,
            output_alias: Some(1),
            decks: false,
            start: 0.0,
            end: 0.1,
            tail: 0.0,
            repeats: 1,
            options: crate::audio_delivery::Options {
                rate,
                ..Default::default()
            },
        };
        let mut actual = vec![0.0; (rate as usize / 10) * 4];
        assert_eq!(
            test_alloc::measure(|| live.process_interleaved(&mut actual, 4)),
            test_alloc::Counts::default()
        );
        assert!(actual
            .chunks_exact(4)
            .any(|f| f[2].abs() > 0.01 && f[3].abs() > 0.01));
        assert!(actual.chunks_exact(4).any(|f| (f[0] - f[2]).abs() > 0.001));
        let root = std::env::temp_dir().join(format!(
            "omat-region-export-{rate}-{}",
            crate::sampler_bank::BankId::new().unwrap()
        ));
        let outcome = crate::audio_delivery::run(
            captured,
            &export,
            &root,
            &engine.cmd.performance().optional_work().unwrap(),
            &crate::background::Reporter::default(),
        )
        .unwrap();
        assert_eq!(outcome.frames, u64::from(rate) / 10);
        let rendered = crate::engine::decode::decode_audio(&root.join("master.wav")).unwrap();
        let expected: Vec<f32> = actual.chunks_exact(4).flat_map(|f| [f[0], f[1]]).collect();
        assert_eq!(
            rendered.sample.data, expected,
            "actual shared-region export differed at {rate}"
        );
        assert!(expected.iter().any(|v| v.abs() > 0.01));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
#[test]
fn invalid_regions_and_cancelled_source_preparation_never_admit_partial_audio() {
    let source = pcm(48000);
    let region = Region::full(&source, 120.0).unwrap();
    for bad in [
        Region {
            start: region.end,
            ..region
        },
        Region {
            end: region.end + 1,
            ..region
        },
        Region {
            loop_end: region.end + 1,
            ..region
        },
        Region {
            loop_start: region.end,
            ..region
        },
        Region {
            tempo: f32::NAN,
            ..region
        },
        Region {
            transpose: f32::INFINITY,
            ..region
        },
        Region {
            transpose: 49.0,
            ..region
        },
    ] {
        assert!(bad.prepare(&source).is_err());
    }
    let (engine, live) = Engine::headless_for_test(48000, 256);
    let mut live = Box::new(live);
    let captured = capture(&engine, &mut live);
    let document = edit::Document::capture(&captured, 0, 7).unwrap();
    assert!(edit::Request::prepare(
        captured,
        document,
        source,
        region,
        "Never applied".into(),
        1.0,
        &AtomicBool::new(true)
    )
    .is_err());
    assert_eq!(live.tracks[0].clips[7].kind, crate::engine::ClipKind::Empty);
}
#[test]
fn replacing_audio_keeps_both_sources_reserved_across_undo_and_rejects_active_replay() {
    let (engine, live) = Engine::headless_for_test(48000, 256);
    let mut live = Box::new(live);
    let original = pcm(48000);
    let region = Region::full(&original, 120.0).unwrap();
    let (request, _) = edit(&engine, &mut live, 0, original.clone(), region);
    live.apply(Command::AudioClipEdit(request));
    let mut next = (*original).clone();
    next.data.iter_mut().for_each(|v| *v *= 0.5);
    let next = Arc::new(next);
    let (request, ack) = edit(&engine, &mut live, 0, next.clone(), region);
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::AudioClipEdit(request))),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), crate::engine::midi_edit::Outcome::Applied);
    for redo in [false, true, false, true] {
        assert_eq!(
            test_alloc::measure(|| live.apply(if redo { Command::Redo } else { Command::Undo })),
            test_alloc::Counts::default()
        );
        assert!(Arc::ptr_eq(
            live.tracks[0].clips[7].audio.as_ref().unwrap(),
            if redo { &next } else { &original }
        ));
    }
    live.apply(Command::FireClip {
        track: 0,
        scene: 7,
        looping: true,
    });
    live.apply(Command::Undo);
    assert!(Arc::ptr_eq(
        live.tracks[0].clips[7].audio.as_ref().unwrap(),
        &next
    ));
    live.apply(Command::Stop);
    live.apply(Command::Undo);
    assert!(Arc::ptr_eq(
        live.tracks[0].clips[7].audio.as_ref().unwrap(),
        &original
    ));
}
