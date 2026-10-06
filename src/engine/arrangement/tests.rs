use super::*;
use crate::engine::{
    midi_edit::Outcome,
    project::{Captured, Prepared},
    test_alloc, Command, Engine, RtEngine,
};
use std::sync::atomic::AtomicBool;
fn capture(engine: &Engine, rt: &mut RtEngine) -> Captured {
    let handle = engine.project.clone();
    let job = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let start = std::time::Instant::now();
    while !job.is_finished() {
        rt.process(&mut []);
        assert!(start.elapsed() < std::time::Duration::from_secs(15));
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    job.join().unwrap()
}
fn source(rate: u32) -> Arc<Sample> {
    Arc::new(Sample {
        name: "song source".into(),
        sr: rate,
        ch: 2,
        data: (0..rate * 2)
            .flat_map(|i| {
                [
                    0.05 + (i as f32 * 0.031).sin() * 0.02,
                    -0.07 + (i as f32 * 0.019).cos() * 0.03,
                ]
            })
            .collect(),
        peaks: Default::default(),
        spectrum: None,
        bpm: 120.0,
        path: String::new(),
    })
}
fn song_model(captured: &mut Captured, audio: Arc<Sample>) -> Model {
    let index = captured.media.len();
    captured.media.push(audio.clone());
    let mut clip = captured.state.tracks[0].clips[7].clone();
    clip.kind = ClipKind::Audio;
    clip.name = "retained audio".into();
    clip.notes.clear();
    clip.lanes = None;
    clip.region = None;
    clip.audio = Some(index);
    let region = audio_clip::Region::full(&audio, 120.0).unwrap();
    clip.audio_region = Some(region);
    clip.bars = (region.prepare(&audio).unwrap().duration_beats / 4.0) as f32;
    let track = captured
        .state
        .session
        .as_ref()
        .unwrap()
        .reference(session::Axis::Track, 0)
        .unwrap();
    Model {
        enabled: true,
        next_id: 4,
        sources: vec![Source { id: 1, clip }],
        instances: vec![
            Instance {
                id: 2,
                source: 1,
                track,
                start: 0.0,
                offset: 0.25,
                duration: 2.0,
                repeating: false,
                gain: 0.75,
            },
            Instance {
                id: 3,
                source: 1,
                track,
                start: 1.0,
                offset: 1.0,
                duration: 2.0,
                repeating: false,
                gain: 0.5,
            },
        ],
    }
}
#[test]
fn shared_offsets_overlaps_and_half_open_ends_match_source_coordinates() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let mut captured = capture(&engine, &mut rt);
    let audio = source(48000);
    let model = song_model(&mut captured, audio.clone());
    let clip = model.sources[0].clip.clone();
    let region = clip.audio_region.unwrap().prepare(&audio).unwrap();
    let plan = Plan::prepare(
        Arc::new(model),
        &captured.media,
        captured.state.session.as_ref().unwrap(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut playback = Playback::new(Some(plan), 0.0);
    for beat in [0.0, 0.25, 0.999, 1.0, 1.5, 2.0, 2.999, 3.0, 8192.0] {
        playback.reset(beat);
        let mut expected = [0.0; 2];
        if beat < 2.0 {
            let s = region.sample(&audio, beat + 0.25, false);
            for c in 0..2 {
                expected[c] += s[c] * 0.75;
            }
        }
        if (1.0..3.0).contains(&beat) {
            let s = region.sample(&audio, beat, false);
            for c in 0..2 {
                expected[c] += s[c] * 0.5;
            }
        }
        let actual = playback.sample(0, beat).0;
        for c in 0..2 {
            assert!(
                (actual[c] - expected[c]).abs() < 1e-7,
                "{beat}: {actual:?} vs {expected:?}"
            );
        }
    }
    assert_eq!(clip.audio, captured.media.len().checked_sub(1));
}
#[test]
fn song_edit_undo_seek_and_native_capture_preserve_shared_media_without_heap_work() {
    for rate in [44100, 48000, 96000] {
        let (engine, rt) = Engine::headless_for_test(rate, 256);
        let mut rt = Box::new(rt);
        let mut captured = capture(&engine, &mut rt);
        let audio = source(rate);
        let model = song_model(&mut captured, audio.clone());
        let (request, ack) =
            edit::Request::prepare(captured, model, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            test_alloc::measure(|| rt.apply(Command::ArrangementEdit(request))),
            Default::default()
        );
        assert_eq!(ack.state(), Outcome::Applied);
        assert!(rt.arrangement.enabled());
        for command in [Command::Undo, Command::Redo] {
            assert_eq!(
                test_alloc::measure(|| rt.apply(command)),
                Default::default()
            );
        }
        let captured = capture(&engine, &mut rt);
        let song = captured.state.arrangement.as_ref().unwrap();
        assert_eq!(song.instances.len(), 2);
        assert!(Arc::ptr_eq(
            &captured.media[song.sources[0].clip.audio.unwrap()],
            &audio
        ));
        let mut reopened =
            Prepared::from_state(captured.state.clone(), captured.media.clone(), rate)
                .unwrap()
                .into_offline();
        assert_eq!(
            test_alloc::measure(|| {
                rt.apply(Command::Play);
                rt.apply(Command::TimelineSeek(0.75));
            }),
            Default::default()
        );
        reopened.apply(Command::Play);
        reopened.apply(Command::TimelineSeek(0.75));
        let mut actual = [0.0; 1024];
        let mut expected = [0.0; 1024];
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut actual)),
            Default::default()
        );
        reopened.process(&mut expected);
        assert_eq!(actual, expected);
        let mut legacy = serde_json::to_value(&captured.state).unwrap();
        legacy["version"] = serde_json::json!(18);
        assert!(serde_json::from_value::<project::State>(legacy).is_err());
    }
}
#[test]
fn cancelled_stale_playing_invalid_and_over_capacity_song_edits_preserve_current_work() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let mut captured = capture(&engine, &mut rt);
    let mut model = song_model(&mut captured, source(48000));
    let layout = captured.state.session.as_ref().unwrap();
    let mut invalid = model.clone();
    invalid.instances[0].duration = 99.0;
    assert!(Plan::prepare(
        Arc::new(invalid),
        &captured.media,
        layout,
        &AtomicBool::new(false)
    )
    .is_err());
    assert!(Plan::prepare(
        Arc::new(model.clone()),
        &captured.media,
        layout,
        &AtomicBool::new(true)
    )
    .is_err());
    for n in 4..70 {
        let mut instance = model.instances[0];
        instance.id = n;
        model.instances.push(instance);
    }
    model.next_id = 70;
    assert!(Plan::prepare(
        Arc::new(model.clone()),
        &captured.media,
        layout,
        &AtomicBool::new(false)
    )
    .is_err());
    model.instances.truncate(2);
    let (request, ack) = edit::Request::prepare(captured, model, &AtomicBool::new(false)).unwrap();
    rt.apply(Command::Play);
    rt.apply(Command::ArrangementEdit(request));
    assert_eq!(ack.state(), Outcome::Rejected);
    assert!(rt.arrangement.plan.is_none());
    rt.apply(Command::Stop);
    let mut captured = capture(&engine, &mut rt);
    let model = song_model(&mut captured, source(48000));
    let (request, ack) = edit::Request::prepare(captured, model, &AtomicBool::new(false)).unwrap();
    assert!(ack.cancel());
    rt.apply(Command::ArrangementEdit(request));
    assert!(rt.arrangement.plan.is_none());
}
#[test]
fn midi_same_pitch_holds_and_seek_chase_keep_independent_instance_owners() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let captured = capture(&engine, &mut rt);
    let layout = captured.state.session.as_ref().unwrap();
    let mut clip = captured.state.tracks[1].clips[7].clone();
    clip.kind = ClipKind::Midi;
    clip.bars = 1.0;
    clip.notes = vec![MidiNote {
        id: midi_edit::NoteId::new(),
        muted: false,
        pitch: 64,
        start: 0.0,
        len: 2.0,
        vel: 100,
        channel: 3,
        release_vel: 7,
        source_timing: None,
    }];
    let track = layout.reference(session::Axis::Track, 1).unwrap();
    let instance = Instance {
        id: 2,
        source: 1,
        track,
        start: 0.0,
        offset: 0.0,
        duration: 4.0,
        repeating: false,
        gain: 1.0,
    };
    let model = Model {
        enabled: true,
        next_id: 4,
        sources: vec![Source { id: 1, clip }],
        instances: vec![
            instance,
            Instance {
                id: 3,
                start: 1.0,
                ..instance
            },
        ],
    };
    let plan = Plan::prepare(
        Arc::new(model),
        &captured.media,
        layout,
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut playback = Playback::new(Some(plan), 0.0);
    let first = playback.gate(1, 0.001).unwrap();
    let second = playback.gate(1, 1.001).unwrap();
    assert!(first.1 && second.1);
    assert_ne!(first.0.id, second.0.id);
    let off = playback.gate(1, 2.001).unwrap();
    assert!(!off.1 && !off.2);
    let off = playback.gate(1, 3.001).unwrap();
    assert!(!off.1 && off.2);
    playback.reset(1.5);
    assert!(playback.gate(1, 1.501).unwrap().1);
    assert!(playback.gate(1, 1.501).unwrap().1);
    assert!(playback.gate(1, 1.501).is_none());
}

#[test]
fn arrangement_native_container_and_selected_track_import_keep_shared_instances_and_undo() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let mut captured = capture(&engine, &mut rt);
    let song = song_model(&mut captured, source(48000));
    let (request, _) = edit::Request::prepare(captured, song, &AtomicBool::new(false)).unwrap();
    rt.apply(Command::ArrangementEdit(request));
    let saved = capture(&engine, &mut rt);
    let path = std::env::temp_dir().join(format!(
        "omatainer-arrangement-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    let limits = crate::project_file::Limits::default();
    crate::project_file::save(
        &path,
        &crate::project_file::Bundle {
            state: saved.state,
            media: saved.media,
        },
        crate::project_file::Overwrite::Never,
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reopened =
        crate::project_file::load::<project::State>(&path, &limits, &AtomicBool::new(false))
            .unwrap();
    reopened.state.validate(&reopened.media).unwrap();
    std::fs::remove_file(path).unwrap();
    let (destination, live) = Engine::headless_for_test(44100, 256);
    let mut live = Box::new(live);
    let base = live.tracks.len();
    let captured = capture(&destination, &mut live);
    let layout = reopened.state.session.as_ref().unwrap();
    let selection = session::ImportSelection {
        tracks: vec![layout.tracks[0].id],
        scenes: vec![layout.scenes[7].id],
        clips: true,
        devices: true,
        keep_timing: true,
    };
    let (request, ack) = session::Request::import(
        captured,
        &reopened.state,
        &reopened.media,
        &selection,
        44100,
    )
    .unwrap();
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::SessionEdit(request))),
        Default::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    let imported = capture(&destination, &mut live);
    let song = imported.state.arrangement.as_ref().unwrap();
    assert_eq!(song.instances.len(), 2);
    assert_eq!(song.sources.len(), 1);
    assert!(!song.enabled);
    assert_eq!(
        song.instances[0].track,
        live.session.reference(session::Axis::Track, base).unwrap()
    );
    let media = imported.media[song.sources[0].clip.audio.unwrap()].clone();
    assert!(live
        .arrangement
        .plan
        .as_ref()
        .unwrap()
        .media()
        .any(|a| Arc::ptr_eq(a, &media)));
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::Undo)),
        Default::default()
    );
    assert_eq!(live.tracks.len(), base);
    assert!(live.arrangement.plan.is_none());
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::Redo)),
        Default::default()
    );
    assert_eq!(live.tracks.len(), base + 1);
    assert!(live.arrangement.plan.is_some());
}

#[test]
fn arrangement_stale_protected_budget_recording_and_deck_refusals_acknowledge_without_heap_work() {
    for refusal in 0..5 {
        let (engine, rt) = Engine::headless_for_test(48000, 256);
        let mut rt = Box::new(rt);
        let mut captured = capture(&engine, &mut rt);
        let model = song_model(&mut captured, source(48000));
        let (request, ack) =
            edit::Request::prepare(captured, model, &AtomicBool::new(false)).unwrap();
        match refusal {
            0 => rt.apply(Command::Master(0.42)),
            1 => rt.apply(Command::PerformanceMode(true)),
            2 => rt.set_undo_budget_for_test(1024),
            3 => rt.recording = true,
            4 => rt.decks[0].touching = true,
            _ => unreachable!(),
        }
        assert_eq!(
            test_alloc::measure(|| rt.apply(Command::ArrangementEdit(request))),
            Default::default(),
            "refusal {refusal}"
        );
        assert_eq!(ack.state(), Outcome::Rejected, "refusal {refusal}");
        assert!(rt.arrangement.plan.is_none());
    }
}

#[test]
fn arrangement_count_in_and_conductor_song_seek_use_exact_musical_positions() {
    use midi_data::{Conductor, Meter, Tempo, TimingSettings};
    for rate in [44100, 48000, 96000] {
        let (engine, rt) = Engine::headless_for_test(rate, 256);
        let mut rt = Box::new(rt);
        rt.metronome = false;
        rt.conductor = Some(
            Conductor::native(
                960,
                vec![
                    Tempo::new(0, 120.0, true).unwrap(),
                    Tempo::new(3840, 180.0, false).unwrap(),
                ],
                vec![Meter {
                    tick: 0,
                    numerator: 4,
                    denominator_power: 2,
                    clocks: 24,
                    thirty_seconds: 8,
                }],
                TimingSettings {
                    count_in: 1,
                    accent_gain: 0.0,
                    beat_gain: 0.0,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let mut captured = capture(&engine, &mut rt);
        let model = song_model(&mut captured, source(rate));
        let (request, _) =
            edit::Request::prepare(captured, model, &AtomicBool::new(false)).unwrap();
        rt.apply(Command::ArrangementEdit(request));
        rt.apply(Command::Play);
        let mut count = vec![0.0; rate as usize * 4];
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut count)),
            Default::default()
        );
        assert!(count.iter().all(|v| *v == 0.0));
        assert_eq!(rt.precise_midi_beat(), 0.0);
        let mut onset = [0.0; 2];
        rt.process(&mut onset);
        assert!(rt.count_in.is_none());
        assert!(onset.iter().any(|v| v.abs() > 0.0));
        assert_eq!(
            test_alloc::measure(|| rt.apply(Command::SongSeek(1.5))),
            Default::default()
        );
        assert!((rt.precise_midi_beat() - 1.5).abs() < 1e-12);
        assert!(
            (rt.timeline_seconds() - rt.conductor.as_ref().unwrap().seconds_at(1.5)).abs() < 1e-12
        );
        rt.process(&mut [0.0; 512]);
        assert!(rt.precise_midi_beat() > 1.5);
        assert!(engine.send(Command::SongSeek(f64::NAN)).is_err());
        engine.cmd.performance().set_enabled(true).unwrap();
        assert!(engine.send(Command::SongSeek(0.0)).is_err());
    }
}

#[test]
fn deleted_reused_tracks_and_track_templates_cannot_retarget_or_smuggle_song_sources() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let mut captured = capture(&engine, &mut rt);
    let model = song_model(&mut captured, source(48000));
    let mut layout = captured.state.session.clone().unwrap();
    let old = layout.tracks[0].id;
    layout.delete(session::Axis::Track, old).unwrap();
    let plan = Plan::prepare(
        Arc::new(model.clone()),
        &captured.media,
        &layout,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(Playback::new(Some(plan), 0.0).sample(0, 0.5).0, [0.0; 2]);
    layout.tracks[0].id = session::Id(layout.next_id);
    layout.next_id += 1;
    layout.tracks[0].active = true;
    layout.track_order.insert(0, 0);
    layout.validate().unwrap();
    let plan = Plan::prepare(
        Arc::new(model.clone()),
        &captured.media,
        &layout,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(Playback::new(Some(plan), 0.0).sample(0, 0.5).0, [0.0; 2]);
    captured.state.arrangement = Some(Arc::new(model));
    let (mut template, media) = captured
        .state
        .track_configuration(&captured.media, 0)
        .unwrap();
    assert!(template.arrangement.is_none());
    template.arrangement = captured.state.arrangement.clone();
    let bus = captured.state.session.as_ref().unwrap().scenes[0]
        .name
        .clone();
    assert!(captured
        .state
        .apply_track_configuration(captured.media, &template, &media, 0, &bus)
        .is_err());
}

#[test]
fn arrangement_sources_refuse_ragged_or_nonfinite_pcm_before_preparation() {
    for bad in [false, true] {
        let (engine, rt) = Engine::headless_for_test(48000, 256);
        let mut rt = Box::new(rt);
        let mut captured = capture(&engine, &mut rt);
        let mut audio = (*source(48000)).clone();
        if bad {
            audio.data[0] = f32::NAN;
        } else {
            audio.data.pop();
        }
        let audio = Arc::new(audio);
        let mut model = song_model(&mut captured, source(48000));
        let index = model.sources[0].clip.audio.unwrap();
        captured.media[index] = audio;
        model.sources[0].clip.audio_region = None;
        model.sources[0].clip.bars = 1.0;
        assert!(Plan::prepare(
            Arc::new(model),
            &captured.media,
            captured.state.session.as_ref().unwrap(),
            &AtomicBool::new(false)
        )
        .is_err());
    }
}

#[test]
fn arrangement_audio_uses_track_devices_program_routes_prefader_cue_and_input_precedence() {
    use crate::engine::audio::routing::{model::*, prepared};
    for rate in [44100, 48000, 96000] {
        let (engine, rt) = Engine::headless_for_test(rate, 256);
        let mut rt = Box::new(rt);
        for track in &mut rt.tracks {
            track.fx.slots.clear();
        }
        for chain in &mut rt.scene_fx {
            chain.slots.clear();
        }
        rt.fx_wet.fill(0.0);
        let mut captured = capture(&engine, &mut rt);
        let model = song_model(&mut captured, source(rate));
        let (request, _) =
            edit::Request::prepare(captured, model, &AtomicBool::new(false)).unwrap();
        rt.apply(Command::ArrangementEdit(request));
        rt.tracks[0].input_monitor = Some(crate::engine::input_monitor::Mode::Off);
        rt.tracks[0].gain = 0.0;
        let mut effect = crate::engine::fx::FxSlot::new(crate::engine::fx::FxId::Dist, rate as f32);
        effect.mix = 1.0;
        effect.p[0] = 0.75;
        rt.tracks[0].fx.slots.push(effect);
        let mut graph = Model::default();
        graph.version = 2;
        graph.next_id = 3;
        graph.ports.push(Port {
            id: 2,
            alias: "Cue".into(),
            direction: Direction::Output,
            channels: vec![2, 3],
        });
        graph.monitor_output = Some(2);
        rt.routing = Some(Box::new(
            prepared::Prepared::new(Arc::new(graph), &rt.session).unwrap(),
        ));
        rt.apply(Command::TrackPfl {
            track: 0,
            value: true,
        });
        rt.apply(Command::Monitor(crate::engine::monitor::Control::Volume(
            1.0,
        )));
        rt.apply(Command::Monitor(crate::engine::monitor::Control::Source(
            crate::engine::monitor::Source::Pfl,
        )));
        rt.apply(Command::Play);
        let mut output = [0.0; 4096];
        assert_eq!(
            test_alloc::measure(|| rt.process_interleaved(&mut output, 4)),
            Default::default()
        );
        assert!(output
            .chunks_exact(4)
            .skip(512)
            .all(|f| f[0] == 0.0 && f[1] == 0.0));
        assert!(output
            .chunks_exact(4)
            .skip(512)
            .any(|f| f[2].abs() > 0.01 && f[3].abs() > 0.01));
        rt.render_track(0, false);
        let processed = rt.routing_track_taps[1];
        let position = (rt.precise_midi_beat() - rt.last_midi_step).max(0.0);
        let original = rt.arrangement.sample(0, position).0;
        assert_eq!(rt.routing_track_taps[0], original);
        assert!(processed
            .iter()
            .zip(original)
            .any(|(a, b)| (a - b).abs() > 0.001));
        rt.tracks[0].fx.slots.clear();
        rt.tracks[0].input_monitor = Some(crate::engine::input_monitor::Mode::In);
        for _ in 0..1024 {
            rt.beat += 2.0 / f64::from(rate);
            rt.routing_track_input = Some([0.3, -0.4]);
            rt.render_track(0, false);
        }
        assert_eq!(rt.routing_track_taps[0], [0.3, -0.4]);
    }
}
