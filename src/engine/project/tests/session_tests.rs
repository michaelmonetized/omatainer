use super::*;

fn large_state() -> (State, Vec<Arc<Sample>>) {
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
    clip.notes = vec![MidiNote {
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
    raw.as_object_mut().unwrap().remove("session");
    let old: State = serde_json::from_value(raw).unwrap();
    old.validate(&base.media).unwrap();
    let first = Prepared::from_state(old.clone(), base.media.clone(), 48_000).unwrap();
    let second = Prepared::from_state(old, base.media, 48_000).unwrap();
    assert_eq!(first.rt.session, second.rt.session);
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
        let counts = test_alloc::measure(|| {
            prepared.rt.apply(Command::SessionEdit(request));
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
