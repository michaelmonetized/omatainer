use super::*;
use crate::engine::{test_alloc, Engine};

fn submit(engine: &Engine, rt: &RtEngine, action: Action) -> Ack {
    let (request, ack) =
        Request::metadata(&rt.session, rt.undo.checkpoint().epoch, action).unwrap();
    engine.send(Command::SessionEdit(request)).unwrap();
    ack
}

#[test]
fn reorder_retains_playing_clip_note_identities_processor_storage_and_live_gate_destination() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 144);
    engine.send(Command::Select { track: 2, scene: 0 }).unwrap();
    engine
        .send(Command::FireClip {
            track: 2,
            scene: 0,
            looping: true,
        })
        .unwrap();
    engine
        .send(Command::LiveNoteOn {
            source: 17,
            ch: 4,
            note: 72,
            vel: 91,
        })
        .unwrap();
    rt.process(&mut [0.0; 128]);
    let identity = rt.session.tracks[2].id;
    let scene_identity = rt.session.scenes[0].id;
    let notes = rt.tracks[2].clips[0].notes.clone();
    let track_address = &rt.tracks[2] as *const _ as usize;
    let rack_address = &rt.scene_fx[0] as *const _ as usize;
    let input = crate::engine::dsp::InputKey::Midi {
        source: 17,
        ch: 4,
        note: 72,
    };
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|voice| voice.input == Some(input) && voice.env.stage < 4));
    let ack = submit(
        &engine,
        &rt,
        Action::Move {
            axis: Axis::Track,
            id: identity,
            position: 0,
        },
    );
    let counts = test_alloc::measure(|| rt.process(&mut [0.0; 128]));
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(ack.state(), Outcome::Applied);
    assert_eq!(&rt.tracks[2] as *const _ as usize, track_address);
    assert_eq!(rt.session.track_order[0], 2);
    assert_eq!(rt.selected_track, 2);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    assert_eq!(rt.tracks[2].clips[0].notes, notes);
    let ack = submit(
        &engine,
        &rt,
        Action::Move {
            axis: Axis::Scene,
            id: scene_identity,
            position: 7,
        },
    );
    rt.process(&mut [0.0; 128]);
    assert_eq!(ack.state(), Outcome::Applied);
    assert_eq!(&rt.scene_fx[0] as *const _ as usize, rack_address);
    assert_eq!(rt.session.scene_order[7], 0);
    assert_eq!(rt.selected_scene, 0);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    assert_eq!(rt.tracks[2].scene_bus, 0);
    engine.send(Command::Select { track: 7, scene: 7 }).unwrap();
    engine
        .send(Command::LiveNoteOff {
            source: 17,
            ch: 4,
            note: 72,
        })
        .unwrap();
    rt.process(&mut [0.0; 128]);
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .filter(|voice| voice.input == Some(input))
        .all(|voice| voice.env.stage >= 4));
    let counts = test_alloc::measure(|| rt.apply(Command::Undo));
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(rt.session.scene_order[0], 0);
    assert_eq!(rt.selected_track, 7);
    assert_eq!(rt.selected_scene, 7);
    rt.apply(Command::Redo);
    assert_eq!(rt.session.scene_order[7], 0);
    assert_eq!(rt.tracks[2].clips[0].notes, notes);
}

#[test]
fn rename_and_color_are_atomic_undoable_and_stale_or_cancelled_edits_do_not_mutate() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 144);
    let id = rt.session.tracks[1].id;
    let old_name = rt.tracks[1].name.clone();
    let (stale, stale_ack) = Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        Action::Color {
            axis: Axis::Track,
            id,
            color: Some([1, 2, 3]),
        },
    )
    .unwrap();
    let ack = submit(
        &engine,
        &rt,
        Action::Rename {
            axis: Axis::Track,
            id,
            name: "Bass melody".into(),
        },
    );
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Applied);
    assert_eq!(rt.tracks[1].name, "Bass melody");
    assert_eq!(rt.session.tracks[1].name, "Bass melody");
    engine.send(Command::SessionEdit(stale)).unwrap();
    rt.process(&mut []);
    assert_eq!(stale_ack.state(), Outcome::Rejected);
    assert_eq!(rt.session.tracks[1].color, None);
    let (cancelled, ack) = Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        Action::Rename {
            axis: Axis::Track,
            id,
            name: "Cancelled".into(),
        },
    )
    .unwrap();
    assert!(ack.cancel());
    engine.send(Command::SessionEdit(cancelled)).unwrap();
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Cancelled);
    assert_eq!(rt.tracks[1].name, "Bass melody");
    let generation = rt.session.generation;
    rt.apply(Command::Undo);
    assert_eq!(rt.tracks[1].name, old_name);
    assert_eq!(rt.session.tracks[1].name, old_name);
    assert!(rt.session.generation > generation);
    rt.apply(Command::Redo);
    assert_eq!(rt.tracks[1].name, "Bass melody");
}

#[test]
fn reordering_remains_available_during_protected_playback() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 144);
    engine.send(Command::PerformanceMode(true)).unwrap();
    let id = rt.session.tracks[2].id;
    let ack = submit(
        &engine,
        &rt,
        Action::Move {
            axis: Axis::Track,
            id,
            position: 0,
        },
    );
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Applied);
    let (delete, ack) = Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        Action::Delete {
            axis: Axis::Track,
            id,
        },
    )
    .unwrap();
    assert!(engine.send(Command::SessionEdit(delete)).is_err());
    assert_eq!(ack.state(), Outcome::Rejected);
    assert!(rt.session.tracks[2].active);
}

#[test]
fn deleting_a_scene_moves_only_its_bus_references_and_undo_preserves_another_scene_launched_later()
{
    let (engine, mut rt) = Engine::headless_for_test(48_000, 144);
    rt.selected_track = 2;
    rt.selected_scene = 0;
    rt.apply(Command::FireClip {
        track: 2,
        scene: 0,
        looping: true,
    });
    rt.process(&mut [0.0; 128]);
    rt.tracks[2].clips[1] = rt.tracks[2].clips[0].clone();
    let id = rt.session.scenes[0].id;
    let ack = submit(
        &engine,
        &rt,
        Action::Delete {
            axis: Axis::Scene,
            id,
        },
    );
    let counts = test_alloc::measure(|| rt.process(&mut [0.0; 128]));
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(ack.state(), Outcome::Applied);
    assert!(!rt.session.scenes[0].active);
    assert_eq!(rt.selected_scene, 1);
    assert_eq!(rt.tracks[2].scene_bus, 1);
    assert!(rt.tracks[2].playing.is_none());
    rt.apply(Command::Undo);
    assert!(rt.session.scenes[0].active);
    assert_eq!(rt.tracks[2].scene_bus, 0);
    rt.apply(Command::Redo);
    assert_eq!(rt.tracks[2].scene_bus, 1);
    rt.apply(Command::FireClip {
        track: 2,
        scene: 1,
        looping: true,
    });
    rt.process(&mut [0.0; 128]);
    assert_eq!(rt.tracks[2].scene_bus, 1);
    rt.apply(Command::Undo);
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 1);
    assert_eq!(rt.tracks[2].scene_bus, 1);
}

#[test]
fn deleted_track_closes_live_gates_and_an_old_routed_target_cannot_play_a_reused_slot() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 144);
    let old = rt.session.reference(Axis::Track, 2).unwrap();
    rt.apply(Command::RoutedNoteOn {
        source: 19,
        ch: 1,
        note: 72,
        vel: 91,
        track: 2,
        target: Some(old),
    });
    let input = crate::engine::dsp::InputKey::Midi {
        source: 19,
        ch: 1,
        note: 72,
    };
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .any(|v| v.input == Some(input) && v.env.stage < 4));
    let ack = submit(
        &engine,
        &rt,
        Action::Delete {
            axis: Axis::Track,
            id: old.id,
        },
    );
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Applied);
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .filter(|v| v.input == Some(input))
        .all(|v| v.env.stage >= 4));
    let (_, slot) = rt
        .session
        .create(Axis::Track, "Replacement".into(), None, 0)
        .unwrap();
    assert_eq!(slot, 2);
    rt.midi_routing.identity.publish(&rt.session);
    rt.apply(Command::RoutedNoteOn {
        source: 20,
        ch: 1,
        note: 73,
        vel: 91,
        track: 2,
        target: Some(old),
    });
    assert!(!rt.tracks[2].poly.voices.iter().any(|v| v.input
        == Some(crate::engine::dsp::InputKey::Midi {
            source: 20,
            ch: 1,
            note: 73
        })));
}
