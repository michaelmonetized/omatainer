use super::*;

#[test]
fn prepared_native_timing_is_atomic_undoable_cancelled_and_namespace_qualified_without_heap() {
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    use crate::engine::{midi_edit::Outcome, midi_interchange::Request};
    let map = Conductor::native(
        960,
        vec![
            Tempo::new(0, 120.0, true).unwrap(),
            Tempo::new(8160, 180.0, false).unwrap(),
        ],
        vec![Meter {
            tick: 0,
            numerator: 7,
            denominator_power: 3,
            clocks: 12,
            thirty_seconds: 8,
        }],
        TimingSettings {
            pickup: 0.5,
            subdivision: 2,
            count_in: 2,
            ..TimingSettings::default()
        },
    )
    .unwrap();
    let mut live = rt();
    let _history = live.enable_undo().unwrap();
    let node = (&*live.tracks[2]) as *const _ as usize;
    let notes = live.tracks[2].clips[0].notes.clone();
    let (request, ack) =
        Request::prepare_timing(captured(&live), Some(map.clone()), || false).unwrap();
    live.configure_clock_input(crate::engine::midi::clock_input::Config{source:Some(71),..Default::default()});
    assert!(live.clock_input.enabled());
    let counts = test_alloc::measure(|| live.apply(Command::MidiImport(request)));
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(ack.state(), Outcome::Applied);
    assert!(!live.clock_input.enabled());
    assert_eq!(live.conductor.as_ref(), Some(&map));
    assert_eq!(live.tracks[2].clips[0].notes, notes);
    assert_eq!((&*live.tracks[2]) as *const _ as usize, node);
    live.apply(Command::Play);
    assert!(live.count_in.is_some());
    live.apply(Command::Undo);
    assert_eq!(live.conductor.as_ref(), Some(&map));
    live.apply(Command::Stop);
    live.configure_clock_input(crate::engine::midi::clock_input::Config{source:Some(71),..Default::default()});
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::Undo)),
        test_alloc::Counts::default()
    );
    assert!(live.conductor.is_none());
    assert!(!live.clock_input.enabled());
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::Redo)),
        test_alloc::Counts::default()
    );
    assert_eq!(live.conductor.as_ref(), Some(&map));

    let (cancelled, cancel_ack) = Request::prepare_timing(captured(&live), None, || false).unwrap();
    assert!(cancel_ack.cancel());
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::MidiImport(cancelled))),
        test_alloc::Counts::default()
    );
    assert_eq!(cancel_ack.state(), Outcome::Cancelled);
    assert_eq!(live.conductor.as_ref(), Some(&map));
    assert!(Request::prepare_timing(captured(&live), None, || true).is_err());

    let (stale, stale_ack) = Request::prepare_timing(captured(&live), None, || false).unwrap();
    live.session.namespace[0] ^= 1;
    assert_eq!(
        test_alloc::measure(|| live.apply(Command::MidiImport(stale))),
        test_alloc::Counts::default()
    );
    assert_eq!(stale_ack.state(), Outcome::Rejected);
    assert_eq!(live.conductor.as_ref(), Some(&map));
    assert_eq!((&*live.tracks[2]) as *const _ as usize, node);
}

#[test]
fn schema_eight_reopens_native_tempo_ramps_and_older_files_refuse_new_timing_fields() {
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    let mut saved = captured(&rt());
    saved.state.conductor = Some(
        Conductor::native(
            960,
            vec![
                Tempo::new(0, 120.0, true).unwrap(),
                Tempo::new(8160, 180.0, false).unwrap(),
            ],
            vec![
                Meter {
                    tick: 0,
                    numerator: 7,
                    denominator_power: 3,
                    clocks: 12,
                    thirty_seconds: 8,
                },
                Meter {
                    tick: 3360,
                    numerator: 5,
                    denominator_power: 2,
                    clocks: 24,
                    thirty_seconds: 8,
                },
                Meter {
                    tick: 8160,
                    numerator: 4,
                    denominator_power: 2,
                    clocks: 24,
                    thirty_seconds: 8,
                },
            ],
            TimingSettings {
                pickup: 0.5,
                subdivision: 2,
                count_in: 2,
                accent_gain: 1.5,
                beat_gain: 0.75,
            },
        )
        .unwrap(),
    );
    saved.state.validate(&saved.media).unwrap();
    let wire = serde_json::to_value(&saved.state).unwrap();
    assert_eq!(wire["version"], STATE_VERSION);
    let reopened: State = serde_json::from_value(wire.clone()).unwrap();
    reopened.validate(&saved.media).unwrap();
    let prepared = Prepared::from_state(reopened, saved.media.clone(), 48000).unwrap();
    let recaptured = captured(&prepared.rt);
    assert_eq!(serde_json::to_value(&recaptured.state).unwrap(), wire);
    assert_eq!(prepared.rt.conductor.as_ref().unwrap().position(0.0).0, 0);
    assert_eq!(prepared.rt.conductor.as_ref().unwrap().position(8.5).0, 3);

    let mut version_eight = wire.clone();
    version_eight["version"] = 8.into();
    version_eight.as_object_mut().unwrap().remove("timeline_seconds");
    let compatible: State = serde_json::from_value(version_eight).unwrap();
    compatible.validate(&saved.media).unwrap();
    assert_eq!(compatible.conductor, saved.state.conductor);

    let mut old = wire;
    old["version"] = 7.into();
    old.as_object_mut().unwrap().remove("timeline_seconds");
    assert!(serde_json::from_value::<State>(old.clone()).is_err());
    old["conductor"].as_object_mut().unwrap().remove("native");
    for p in old["conductor"]["tempos"].as_array_mut().unwrap() {
        p.as_object_mut().unwrap().remove("ramp");
    }
    let legacy: State = serde_json::from_value(old.clone()).unwrap();
    legacy.validate(&saved.media).unwrap();
    for value in [serde_json::Value::Null, serde_json::json!(false)] {
        let mut invalid = old.clone();
        invalid["conductor"]["tempos"][0]["ramp"] = value;
        assert!(serde_json::from_value::<State>(invalid).is_err());
    }
    old["conductor"]["native"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<State>(old).is_err());
}

pub(in crate::engine::project) fn large_state() -> (State, Vec<Arc<Sample>>) {
    let base = captured(&rt());
    let mut state = base.state;
    let mut empty = state.tracks[7].clone();
    empty.launch = None;
    for clip in &mut empty.clips {
        *clip = State::blank().tracks[0].clips[0].clone();
    }
    empty.clips.resize_with(session::MAX_SCENES, || {
        State::blank().tracks[0].clips[0].clone()
    });
    let scene_count = session::MAX_SCENES;
    for track in &mut state.tracks {
        track
            .clips
            .resize_with(scene_count, || empty.clips[0].clone());
    }
    state
        .tracks
        .resize_with(session::MAX_TRACKS, || empty.clone());
    for (i, track) in state.tracks.iter_mut().enumerate() {
        track.name = format!("Track {}", i + 1);
    }
    state.scene_fx.resize_with(scene_count, Vec::new);
    let mut layout =
        session::Layout::fresh(state.tracks.iter().map(|t| t.name.clone()), scene_count);
    let track_id = layout.tracks[127].id;
    let scene_id = layout.scenes[511].id;
    layout.reorder(session::Axis::Track, track_id, 0).unwrap();
    layout.reorder(session::Axis::Scene, scene_id, 0).unwrap();
    layout
        .rename(session::Axis::Scene, scene_id, "Finale".into())
        .unwrap();
    layout
        .color(session::Axis::Track, track_id, Some([123, 42, 87]))
        .unwrap();
    state.session = Some(layout);
    state.selected_track = 127;
    state.selected_scene = 511;
    state.tracks[127].scene_bus = 511;
    state.tracks[127].launch = Some(Launch {
        scene: 511,
        start_beat: 0.0,
        looping: true,
    });
    let clip = &mut state.tracks[127].clips[511];
    clip.kind = ClipKind::Midi;
    clip.name = "Last MIDI cell".into();
    clip.notes = vec![MidiNote { variation: None,
        id: midi_edit::NoteId::new(),
        pitch: 64,
        vel: 87,
        start: 0.0,
        len: 1.5,
        muted: false,
        channel: 12,
        release_vel: 23,
        source_timing: None,
    }];
    let audio = &mut state.tracks[126].clips[510];
    audio.kind = ClipKind::Audio;
    audio.name = "Last audio cell".into();
    audio.audio = Some(0);
    (state, base.media)
}

#[test]
fn all_65536_cells_identities_focus_and_last_scene_launch_roundtrip() {
    let (state, media) = large_state();
    state.validate(&media).unwrap();
    let expected = serde_json::to_value(&state).unwrap();
    let bytes = serde_json::to_vec(&state).unwrap();
    assert!(bytes.len() < 64 * 1024 * 1024);
    let reopened: State = serde_json::from_slice(&bytes).unwrap();
    reopened.validate(&media).unwrap();
    assert_eq!(serde_json::to_value(&reopened).unwrap(), expected);
    let prepared = Prepared::from_state(reopened, media, 48_000).unwrap();
    let final_capture = captured(&prepared.rt);
    assert_eq!(
        serde_json::to_value(&final_capture.state).unwrap(),
        expected
    );
    assert_eq!(prepared.rt.tracks.len(), 128);
    assert_eq!(prepared.rt.scene_fx.len(), 512);
    assert_eq!(prepared.rt.tracks[127].project_resume.unwrap().scene, 511);
    assert_eq!(prepared.rt.selected_track, 127);
    assert_eq!(prepared.rt.selected_scene, 511);
    assert_eq!(prepared.rt.session.track_order[0], 127);
    assert_eq!(prepared.rt.session.scene_order[0], 511);
    assert!(prepared.rt.tracks[126].clips[510].audio.is_some());
}

#[test]
fn oversized_or_inconsistent_dimensions_fail_before_preparing_a_graph() {
    let base = captured(&rt());
    let mut state = base.state;
    let before = serde_json::to_value(&state).unwrap();
    state.tracks.push(state.tracks[0].clone());
    assert!(state
        .validate(&base.media)
        .unwrap_err()
        .contains("identity and storage"));
    state = serde_json::from_value(before).unwrap();
    let extra = state.tracks[0].clips[0].clone();
    state.tracks[0].clips.push(extra);
    assert!(state
        .validate(&base.media)
        .unwrap_err()
        .contains("dimensions"));
    state.tracks[0].clips.pop();
    state.scene_fx.resize_with(513, Vec::new);
    assert!(Prepared::from_state(state, base.media, 48_000).is_err());
}

#[test]
fn legacy_file_migration_is_deterministic_and_preserves_original_cell_storage() {
    let base = captured(&rt());
    let mut raw = serde_json::to_value(&base.state).unwrap();
    raw["version"] = 6.into();
    raw.as_object_mut().unwrap().remove("timeline_seconds");
    raw.as_object_mut().unwrap().remove("session");
    let old: State = serde_json::from_value(raw).unwrap();
    old.validate(&base.media).unwrap();
    let first = Prepared::from_state(old.clone(), base.media.clone(), 48_000).unwrap();
    let second = Prepared::from_state(old.clone(), base.media.clone(), 48_000).unwrap();
    assert_eq!(first.rt.session, second.rt.session);
    let mut different_pcm = base.media.clone();
    Arc::make_mut(&mut different_pcm[0]).data[0] += 0.03125;
    let third = Prepared::from_state(old, different_pcm, 48_000).unwrap();
    assert_ne!(first.rt.session.namespace, third.rt.session.namespace);
    assert_eq!(first.rt.session.tracks[0].id, third.rt.session.tracks[0].id);
    first.rt.session.validate().unwrap();
    assert_eq!(first.rt.session.track_order, (0..8).collect::<Vec<u8>>());
    assert_eq!(first.rt.session.scene_order, (0..8).collect::<Vec<u16>>());
    assert_eq!(
        first.rt.tracks[0].clips[0].notes,
        base.state.tracks[0].clips[0].notes
    );
}

#[test]
fn maximum_set_renders_and_reorders_its_last_populated_track_and_scene_without_callback_heap_work()
{
    let (state, media) = large_state();
    let mut prepared = Prepared::from_state(state, media, 48_000).unwrap();
    prepared.rt.enable_undo().unwrap();
    prepared.rt.apply(Command::Play);
    prepared.rt.process(&mut [0.0; 256]);
    let track_id = prepared.rt.session.tracks[127].id;
    let scene_id = prepared.rt.session.scenes[511].id;
    let notes = prepared.rt.tracks[127].clips[511].notes.clone();
    for (axis, id, position) in [
        (session::Axis::Track, track_id, 64),
        (session::Axis::Scene, scene_id, 255),
    ] {
        let (request, ack) = session::Request::metadata(
            &prepared.rt.session,
            prepared.rt.undo.checkpoint().epoch,
            session::Action::Move { axis, id, position },
        )
        .unwrap();
        let command=Command::session_edit(request);
        let counts = test_alloc::measure(|| {
            prepared.rt.apply(command);
            prepared.rt.process(&mut [0.0; 256]);
        });
        assert_eq!(counts, test_alloc::Counts::default());
        assert_eq!(ack.state(), midi_edit::Outcome::Applied);
        assert_eq!(prepared.rt.tracks[127].playing.unwrap().scene, 511);
        assert_eq!(prepared.rt.tracks[127].scene_bus, 511);
        assert_eq!(prepared.rt.tracks[127].clips[511].notes, notes);
        assert_eq!(prepared.rt.selected_track, 127);
        assert_eq!(prepared.rt.selected_scene, 511);
    }
    let captured = captured(&prepared.rt);
    assert_eq!(
        captured.state.session.as_ref().unwrap().track_order[64],
        127
    );
    assert_eq!(
        captured.state.session.as_ref().unwrap().scene_order[255],
        511
    );
}

#[test]
#[ignore = "frozen release qualification: maximum native container, embedded PCM and automation identity"]
fn maximum_native_container_reopens_music_automation_and_playback_reorder() {
    use crate::project_file::{self, Bundle, Limits, Overwrite, SaveOutcome};
    let (mut state, media) = large_state();
    state.tracks[127].clips[511].lanes = Some(
        crate::engine::midi_data::Lanes::new(
            960,
            3840,
            vec![
                crate::midi_file::Message {
                    tick: 0,
                    order: 0,
                    bytes: [0xcc, 9, 0],
                    length: 2,
                },
                crate::midi_file::Message {
                    tick: 0,
                    order: 1,
                    bytes: [0xbc, 74, 50],
                    length: 3,
                },
                crate::midi_file::Message {
                    tick: 1440,
                    order: 2,
                    bytes: [0xbc, 74, 65],
                    length: 3,
                },
                crate::midi_file::Message {
                    tick: 1919,
                    order: 3,
                    bytes: [0xec, 0, 60],
                    length: 3,
                },
            ],
            vec![],
        )
        .unwrap(),
    );
    let expected = serde_json::to_value(&state).unwrap();
    let evidence = std::env::var_os("OMAT_SESSION_EVIDENCE");
    let root = evidence
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!(
                "omat113-native-{}",
                crate::sampler_bank::BankId::new().unwrap()
            ))
        });
    std::fs::create_dir_all(&root).unwrap();
    struct Directory(std::path::PathBuf, bool);
    impl Drop for Directory {
        fn drop(&mut self) {
            if self.1 {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
    let directory = Directory(root, evidence.is_none());
    let path = directory.0.join("maximum-session.omat");
    let outcome = project_file::save(
        &path,
        &Bundle {
            state,
            media: media.clone(),
        },
        Overwrite::Never,
        &Limits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(matches!(outcome, SaveOutcome::Durable));
    let reopened: Bundle<State> =
        project_file::load(&path, &Limits::default(), &AtomicBool::new(false)).unwrap();
    assert_eq!(serde_json::to_value(&reopened.state).unwrap(), expected);
    assert_eq!(reopened.media.len(), media.len());
    for (a, b) in reopened.media.iter().zip(&media) {
        assert_eq!((a.sr, a.ch), (b.sr, b.ch));
        assert!(a
            .data
            .iter()
            .map(|v| v.to_bits())
            .eq(b.data.iter().map(|v| v.to_bits())));
    }
    let mut prepared = Prepared::from_state(reopened.state, reopened.media, 48000).unwrap();
    prepared.rt.enable_undo().unwrap();
    let rt = &mut prepared.rt;
    let lane = rt.tracks[127].clips[511].lanes.clone().unwrap();
    let address = &*rt.tracks[127] as *const _ as usize;
    let track = rt.session.reference(session::Axis::Track, 127).unwrap();
    let scene = rt.session.reference(session::Axis::Scene, 511).unwrap();
    rt.apply(Command::Play);
    rt.apply(Command::RoutedNoteOn {
        source: 913,
        ch: 12,
        note: 72,
        vel: 90,
        track: 127,
        target: Some(track),
    });
    rt.process(&mut [0.0; 256]);
    let launch = rt.tracks[127].playing.unwrap().start_beat;
    for (axis, id, position) in [
        (session::Axis::Track, track.id, 64),
        (session::Axis::Scene, scene.id, 255),
    ] {
        let (request, ack) = session::Request::metadata(
            &rt.session,
            rt.undo.checkpoint().epoch,
            session::Action::Move { axis, id, position },
        )
        .unwrap();
        let command=Command::session_edit(request);
        assert_eq!(
            test_alloc::measure(|| {
                rt.apply(command);
                rt.process(&mut [0.0; 256]);
            }),
            test_alloc::Counts::default()
        );
        assert_eq!(ack.state(), midi_edit::Outcome::Applied);
        assert_eq!(&*rt.tracks[127] as *const _ as usize, address);
        assert!(Arc::ptr_eq(
            rt.tracks[127].clips[511].lanes.as_ref().unwrap(),
            &lane
        ));
        assert_eq!(rt.tracks[127].playing.unwrap().start_beat, launch);
        assert_eq!((rt.selected_track, rt.selected_scene), (127, 511));
        assert!(rt.tracks[127].poly.voices.iter().any(|v| v.input
            == Some(crate::engine::dsp::InputKey::Midi {
                source: 913,
                ch: 12,
                note: 72
            })
            && v.env.stage < 4));
    }
    assert_eq!(lane.messages.len(), 4);
    assert_eq!(rt.tracks[127].scene_bus, 511);
    rt.apply(Command::LiveNoteOff {
        source: 913,
        ch: 12,
        note: 72,
    });
    if evidence.is_some() {
        std::fs::write(directory.0.join("native-container.json"),serde_json::to_vec_pretty(&serde_json::json!({
            "tracks":128,"scenes":512,"cells":65536,"container_bytes":std::fs::metadata(&path).unwrap().len(),
            "embedded_media":media.len(),"pcm_bits_preserved":true,"all_state_fields_preserved":true,
            "automation_messages":lane.messages.len(),"lane_identity_preserved":true,"dsp_address_preserved":true,
            "playing_clip_origin_preserved":true,"live_routed_note_preserved":true,"selected_slots":[127,511],
            "callback_allocations":0,"callback_frees":0,"physical_hardware":false
        })).unwrap()).unwrap();
    }
}
