use super::*;
use crate::engine::{test_alloc, Engine};
use std::{
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

pub(crate) fn capture(engine: &Engine, rt: &mut RtEngine) -> project::Captured {
    let handle = engine.project.clone();
    let job = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    while !job.is_finished() {
        rt.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    job.join().unwrap()
}
fn selection(source: &project::State) -> Selection {
    let layout = source.session.as_ref().unwrap();
    Selection {
        tracks: vec![layout.tracks[2].id, layout.tracks[1].id],
        scenes: vec![layout.scenes[0].id, layout.scenes[1].id],
        clips: true,
        devices: true,
        keep_timing: true,
    }
}

#[test]
fn import_undo_redo_preserve_existing_processors_live_inputs_source_and_embedded_dependencies_without_heap_work(
) {
    let (source_engine, mut source_rt) = Engine::headless_for_test(44_100, 256);
    let mut source = capture(&source_engine, &mut source_rt);
    source.state.bpm = 91.0;
    source.state.tracks[2].clips[0].lanes = Some(
        crate::engine::midi_data::Lanes::new(
            960,
            3840,
            vec![crate::midi_file::Message {
                tick: 120,
                order: 100,
                bytes: [0xb2, 74, 95],
                length: 3,
            }],
            vec![],
        )
        .unwrap(),
    );
    let source_before = serde_json::to_value(&source.state).unwrap();
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    rt.apply(Command::FireClip {
        track: 2,
        scene: 0,
        looping: true,
    });
    rt.apply(Command::LiveNoteOn {
        source: 18,
        ch: 2,
        note: 77,
        vel: 94,
    });
    let current_address = &*rt.tracks[2] as *const _ as usize;
    let current_notes = rt.tracks[2].clips[0].notes.clone();
    let old_tracks = rt.tracks.len();
    let old_scenes = rt.scene_fx.len();
    let namespace = rt.session.namespace;
    let bpm = rt.bpm;
    let captured = capture(&engine, &mut rt);
    let (request, ack) = Request::import(
        captured,
        &source.state,
        &source.media,
        &selection(&source.state),
        48_000,
    )
    .unwrap();
    engine.send(Command::SessionEdit(request)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    assert_eq!(rt.tracks.len(), old_tracks + 2);
    assert_eq!(rt.scene_fx.len(), old_scenes + 2);
    assert_eq!(rt.session.namespace, namespace);
    assert_eq!(&*rt.tracks[2] as *const _ as usize, current_address);
    assert_eq!(rt.tracks[2].clips[0].notes, current_notes);
    assert!(rt.tracks[2].playing.is_some());
    assert_eq!(rt.bpm, bpm);
    let imported = &rt.tracks[old_tracks];
    assert!(!imported.armed && !imported.solo && imported.playing.is_none());
    assert_eq!(imported.scene_bus, old_scenes);
    assert_eq!(
        imported.clips[old_scenes].lanes,
        source.state.tracks[2].clips[0].lanes
    );
    assert_eq!(
        imported.clips[old_scenes].notes.len(),
        source.state.tracks[2].clips[0].notes.len()
    );
    for (new, old) in imported.clips[old_scenes]
        .notes
        .iter()
        .zip(&source.state.tracks[2].clips[0].notes)
    {
        assert_ne!(new.id, old.id);
        assert_eq!(new.pitch, old.pitch);
        assert_eq!(new.source_timing, old.source_timing);
    }
    for (new, &old) in imported
        .drum_samples
        .iter()
        .zip(&source.state.tracks[2].drums)
    {
        assert!(Arc::ptr_eq(new, &source.media[old]));
    }
    let new_ids: Vec<_> = rt.session.tracks[old_tracks..]
        .iter()
        .map(|t| t.id)
        .collect();
    let saved = capture(&engine, &mut rt);
    saved.state.validate(&saved.media).unwrap();
    let reopened = project::Prepared::from_state(saved.state, saved.media, 96_000)
        .unwrap()
        .into_offline();
    assert_eq!(reopened.tracks.len(), old_tracks + 2);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        test_alloc::Counts::default()
    );
    assert_eq!(
        (rt.tracks.len(), rt.scene_fx.len()),
        (old_tracks, old_scenes)
    );
    assert_eq!(&*rt.tracks[2] as *const _ as usize, current_address);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Redo)),
        test_alloc::Counts::default()
    );
    assert_eq!(
        rt.session.tracks[old_tracks..]
            .iter()
            .map(|t| t.id)
            .collect::<Vec<_>>(),
        new_ids
    );
    assert_eq!(serde_json::to_value(&source.state).unwrap(), source_before);
}

#[test]
fn invalid_selection_stale_revision_and_cancellation_leave_destination_intact() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    let source = capture(&engine, &mut rt);
    for change in 0..4 {
        let mut selected = selection(&source.state);
        match change {
            0 => selected.tracks.clear(),
            1 => selected.tracks.push(selected.tracks[0]),
            2 => selected.scenes = vec![Id(u64::MAX)],
            _ => selected.keep_timing = false,
        }
        assert!(Request::import(
            capture(&engine, &mut rt),
            &source.state,
            &source.media,
            &selected,
            48_000
        )
        .is_err());
    }
    let tracks = rt.tracks.len();
    let (request, ack) = Request::import(
        capture(&engine, &mut rt),
        &source.state,
        &source.media,
        &selection(&source.state),
        48_000,
    )
    .unwrap();
    rt.apply(Command::SetBpm(145.0));
    engine.send(Command::SessionEdit(request)).unwrap();
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Rejected);
    assert_eq!(rt.tracks.len(), tracks);
    let (request, ack) = Request::import(
        capture(&engine, &mut rt),
        &source.state,
        &source.media,
        &selection(&source.state),
        48_000,
    )
    .unwrap();
    assert!(ack.cancel());
    engine.send(Command::SessionEdit(request)).unwrap();
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Cancelled);
    assert_eq!(rt.tracks.len(), tracks);
}

#[test]
fn imported_audio_and_unavailable_devices_survive_output_rate_changes_and_empty_configuration_choices(
) {
    let (source_engine, mut source_rt) = Engine::headless_for_test(44_100, 256);
    let mut source = capture(&source_engine, &mut source_rt);
    let device = Arc::new(
        fx::OfflineDevice::new(
            "org.example.absent-delay".into(),
            Some(fx::DeviceState {
                schema: 3,
                data: vec![0, 42, 255],
            }),
        )
        .unwrap(),
    );
    source.state.tracks[2].fx.push(project::Effect {
        id: fx::FxId::Unavailable,
        on: true,
        mix: 0.4,
        p: [0.2; 4],
        offline: Some(device.clone()),
    });
    source.state.scene_fx[0].push(project::Effect {
        id: fx::FxId::Delay,
        on: true,
        mix: 0.3,
        p: [0.2; 4],
        offline: None,
    });
    let cell = &mut source.state.tracks[2].clips[0];
    cell.kind = crate::engine::ClipKind::Audio;
    cell.notes.clear();
    cell.audio = Some(0);
    source.state.validate(&source.media).unwrap();
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    let base_tracks = rt.tracks.len();
    let base_scenes = rt.scene_fx.len();
    let (request, ack) = Request::import(
        capture(&engine, &mut rt),
        &source.state,
        &source.media,
        &selection(&source.state),
        48_000,
    )
    .unwrap();
    engine.send(Command::SessionEdit(request)).unwrap();
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Applied);
    assert_eq!(rt.tracks[base_tracks].fx.slots[0].offline, Some(device));
    assert!(Arc::ptr_eq(
        rt.tracks[base_tracks].clips[base_scenes]
            .audio
            .as_ref()
            .unwrap(),
        &source.media[0]
    ));
    rt.set_sample_rate(96_000).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        test_alloc::Counts::default()
    );
    rt.set_sample_rate(44_100).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Redo)),
        test_alloc::Counts::default()
    );
    assert_eq!(rt.scene_fx[base_scenes].slots[0].id(), fx::FxId::Delay);
    let imported = capture(&engine, &mut rt);
    imported.state.validate(&imported.media).unwrap();
    let mut settings = selection(&source.state);
    settings.clips = false;
    settings.devices = false;
    settings.scenes.clear();
    settings.tracks = vec![source.state.session.as_ref().unwrap().tracks[0].id];
    let (request, ack) =
        Request::import(imported, &source.state, &source.media, &settings, 44_100).unwrap();
    engine.send(Command::SessionEdit(request)).unwrap();
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Applied);
    assert!(rt.tracks.last().unwrap().fx.slots.is_empty());
    assert_eq!(rt.tracks.last().unwrap().kind, 2);
    assert!(rt
        .tracks
        .last()
        .unwrap()
        .clips
        .iter()
        .all(|clip| clip.kind == crate::engine::ClipKind::Empty));
}

#[test]
fn two_saveable_projects_cannot_import_into_an_unsaveable_metadata_envelope() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    let mut source = capture(&engine, &mut rt);
    let device = Arc::new(
        fx::OfflineDevice::new(
            "org.example.serialized".into(),
            Some(fx::DeviceState {
                schema: 1,
                data: vec![255; 1024 * 1024],
            }),
        )
        .unwrap(),
    );
    source.state.tracks[2].fx = vec![
        project::Effect {
            id: fx::FxId::Unavailable,
            on: true,
            mix: 0.5,
            p: [0.2; 4],
            offline: Some(device)
        };
        9
    ];
    let mut destination = capture(&engine, &mut rt);
    destination.state.tracks[2].fx = source.state.tracks[2].fx.clone();
    let cancel = AtomicBool::new(false);
    let limits = crate::project_file::Limits::default();
    for state in [&source.state, &destination.state] {
        crate::project_file::validate_metadata(
            &crate::project_file::Bundle {
                state,
                media: source.media.clone(),
            },
            &limits,
            &cancel,
        )
        .unwrap();
    }
    let tracks = rt.tracks.len();
    let revision = engine.project.revision();
    let error = Request::import(
        destination,
        &source.state,
        &source.media,
        &selection(&source.state),
        48_000,
    )
    .err()
    .unwrap();
    assert!(error.contains("metadata serialization"), "{error}");
    assert_eq!(rt.tracks.len(), tracks);
    assert_eq!(engine.project.revision(), revision);
}

#[test]
fn imported_media_numbering_matches_native_capture_across_decimal_widths() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    let baseline = capture(&engine, &mut rt);
    let target = if baseline.media.len() < 10 { 9 } else { 99 };
    let extra = target - baseline.media.len();
    let mut added = 0;
    for track in &mut rt.tracks {
        for clip in &mut track.clips {
            if added == extra {
                break;
            }
            let mut sample = (*baseline.media[0]).clone();
            sample.name = format!("Numbering {added}");
            sample.path = format!("/virtual/numbering-{added}.wav");
            clip.kind = crate::engine::ClipKind::Audio;
            clip.notes.clear();
            clip.region = None;
            clip.lanes = None;
            clip.audio = Some(Arc::new(sample));
            added += 1;
        }
    }
    assert_eq!(added, extra);
    let (source_engine, mut source_rt) = Engine::headless_for_test(48_000, 256);
    let mut source = capture(&source_engine, &mut source_rt);
    let mut sample = (*source.media[0]).clone();
    sample.name = "Separate clip audio".into();
    sample.path = "/virtual/separate.wav".into();
    let source_index = source.media.len();
    source.media.push(Arc::new(sample));
    let clip = &mut source.state.tracks[2].clips[0];
    clip.kind = crate::engine::ClipKind::Audio;
    clip.notes.clear();
    clip.region = None;
    clip.lanes = None;
    clip.audio = Some(source_index);
    source.state.tracks[2].drums = [0; 6];
    let mut expected = None;
    let (request, ack) = Request::import_with_preflight(
        capture(&engine, &mut rt),
        &source.state,
        &source.media,
        &selection(&source.state),
        48_000,
        |state, media| {
            let refs = state
                .tracks
                .iter()
                .map(|t| (t.drums, t.clips.iter().map(|c| c.audio).collect::<Vec<_>>()))
                .collect::<Vec<_>>();
            expected = Some((
                refs,
                media
                    .iter()
                    .map(|s| Arc::as_ptr(s) as usize)
                    .collect::<Vec<_>>(),
            ));
            Ok(())
        },
    )
    .unwrap();
    engine.send(Command::SessionEdit(request)).unwrap();
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Applied);
    let actual = capture(&engine, &mut rt);
    let refs = actual
        .state
        .tracks
        .iter()
        .map(|t| (t.drums, t.clips.iter().map(|c| c.audio).collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    assert!(actual.media.len() >= 10);
    assert_eq!(
        (
            refs,
            actual
                .media
                .iter()
                .map(|s| Arc::as_ptr(s) as usize)
                .collect::<Vec<_>>()
        ),
        expected.unwrap()
    );
}
