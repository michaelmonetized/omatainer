use super::*;
use crate::engine::{
    midi_edit::Outcome,
    session::{Axis, Request, Structure},
    test_alloc, Engine,
};

fn edit(engine: &Engine, rt: &mut RtEngine, operation: Structure) {
    let (request, ack) = Request::structural(captured(rt), rt.sr as u32, operation).unwrap();
    engine.send(Command::SessionEdit(request)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    captured(rt).state.validate(&captured(rt).media).unwrap();
}
fn replay(rt: &mut RtEngine, command: Command) {
    assert_eq!(
        test_alloc::measure(|| rt.apply(command)),
        test_alloc::Counts::default()
    );
    let state = captured(rt);
    state.state.validate(&state.media).unwrap();
}

#[test]
fn create_audio_and_midi_tracks_and_scene_replay_without_touching_playing_nodes() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 144);
    rt.apply(Command::FireClip {
        track: 2,
        scene: 0,
        looping: true,
    });
    rt.process(&mut [0.0; 128]);
    let address = &*rt.tracks[2] as *const _ as usize;
    let notes = rt.tracks[2].clips[0].notes.clone();
    let beat = rt.tracks[2].playing.unwrap().start_beat;
    edit(
        &engine,
        &mut rt,
        Structure::Track {
            name: "Audio recording".into(),
            audio: true,
            position: 0,
        },
    );
    assert_eq!(rt.tracks.len(), 9);
    assert_eq!(rt.tracks[8].kind, 4);
    assert!(rt.tracks[8].clips.iter().all(|c| !c.occupied()));
    let audio_id = rt.session.tracks[8].id;
    replay(&mut rt, Command::Undo);
    assert_eq!(rt.tracks.len(), 8);
    replay(&mut rt, Command::Redo);
    assert_eq!(rt.tracks.len(), 9);
    assert_eq!(rt.session.tracks[8].id, audio_id);
    edit(
        &engine,
        &mut rt,
        Structure::Track {
            name: "MIDI recording".into(),
            audio: false,
            position: 9,
        },
    );
    assert_eq!(rt.tracks[9].kind, 2);
    assert!(rt.tracks[9].playing.is_none());
    edit(
        &engine,
        &mut rt,
        Structure::Scene {
            name: "Bridge".into(),
            position: 1,
        },
    );
    assert_eq!(rt.scene_fx.len(), 9);
    assert!(rt.tracks.iter().all(|t| t.clips.len() == 9));
    replay(&mut rt, Command::Undo);
    assert_eq!(rt.scene_fx.len(), 8);
    replay(&mut rt, Command::Redo);
    assert_eq!(rt.scene_fx.len(), 9);
    assert_eq!(&*rt.tracks[2] as *const _ as usize, address);
    assert_eq!(rt.tracks[2].playing.unwrap().start_beat, beat);
    assert_eq!(rt.tracks[2].clips[0].notes, notes);
}

#[test]
fn duplicate_track_and_scene_copy_music_with_fresh_ids_shared_audio_and_independent_fx() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 144);
    rt.tracks[2]
        .fx
        .slots
        .push(fx::FxSlot::new(fx::FxId::Delay, 48000.0));
    rt.scene_fx[0]
        .slots
        .push(fx::FxSlot::new(fx::FxId::Reverb, 48000.0));
    rt.tracks[2].clips[1].audio = Some(rt.tracks[2].drum_samples[0].clone());
    rt.tracks[2].clips[1].kind = ClipKind::Audio;
    let audio = rt.tracks[2].clips[1].audio.clone().unwrap();
    let track_id = rt.session.tracks[2].id;
    edit(
        &engine,
        &mut rt,
        Structure::Duplicate {
            axis: Axis::Track,
            id: track_id,
            name: "Keys copy".into(),
            position: 3,
        },
    );
    assert_ne!(rt.session.tracks[8].id, track_id);
    assert!(rt.tracks[8].playing.is_none());
    assert!(!rt.tracks[8].armed);
    let original = &rt.tracks[2].clips[0].notes;
    let duplicate = &rt.tracks[8].clips[0].notes;
    assert_eq!(original.len(), duplicate.len());
    for (old, new) in original.iter().zip(duplicate) {
        assert_ne!(old.id, new.id);
        let mut expected = old.clone();
        expected.id = new.id;
        assert_eq!(&expected, new);
    }
    assert!(Arc::ptr_eq(
        rt.tracks[8].clips[1].audio.as_ref().unwrap(),
        &audio
    ));
    rt.tracks[8].fx.slots[0].mix = 0.2;
    assert_ne!(rt.tracks[2].fx.slots[0].mix, 0.2);
    let scene_id = rt.session.scenes[0].id;
    edit(
        &engine,
        &mut rt,
        Structure::Duplicate {
            axis: Axis::Scene,
            id: scene_id,
            name: "Intro copy".into(),
            position: 1,
        },
    );
    assert_ne!(rt.session.scenes[8].id, scene_id);
    assert_eq!(rt.scene_fx[8].slots[0].id(), fx::FxId::Reverb);
    assert_ne!(
        rt.tracks[2].clips[8].notes[0].id,
        rt.tracks[2].clips[0].notes[0].id
    );
    replay(&mut rt, Command::Undo);
    replay(&mut rt, Command::Undo);
    assert_eq!(rt.tracks.len(), 8);
    assert_eq!(rt.scene_fx.len(), 8);
    replay(&mut rt, Command::Redo);
    replay(&mut rt, Command::Redo);
    assert!(Arc::ptr_eq(
        rt.tracks[8].clips[1].audio.as_ref().unwrap(),
        &audio
    ));
}

#[test]
fn slot_reuse_undo_and_branched_create_never_reuse_an_identity() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 144);
    let old = rt.session.tracks[2].id;
    let original = rt.tracks[2].clips[0].notes.clone();
    let (request, ack) = Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        crate::engine::session::Action::Delete {
            axis: Axis::Track,
            id: old,
        },
    )
    .unwrap();
    engine.send(Command::SessionEdit(request)).unwrap();
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Applied);
    edit(
        &engine,
        &mut rt,
        Structure::Track {
            name: "Replacement".into(),
            audio: true,
            position: 0,
        },
    );
    let replacement = rt.session.tracks[2].id;
    assert_ne!(replacement, old);
    assert_eq!(rt.tracks.len(), 8);
    replay(&mut rt, Command::Undo);
    assert!(!rt.session.tracks[2].active);
    assert_eq!(rt.tracks[2].clips[0].notes, original);
    replay(&mut rt, Command::Undo);
    assert_eq!(rt.session.tracks[2].id, old);
    assert!(rt.session.tracks[2].active);
    replay(&mut rt, Command::Redo);
    replay(&mut rt, Command::Redo);
    assert_eq!(rt.session.tracks[2].id, replacement);
    replay(&mut rt, Command::Undo);
    edit(
        &engine,
        &mut rt,
        Structure::Track {
            name: "Different branch".into(),
            audio: false,
            position: 7,
        },
    );
    assert!(rt.session.tracks[2].id.0 > replacement.0);
}

#[test]
fn stale_cancelled_and_budget_refused_graph_edits_retire_without_callback_heap_activity() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 144);
    let prepare = |rt: &RtEngine| {
        Request::structural(
            captured(rt),
            48000,
            Structure::Track {
                name: "Prepared".into(),
                audio: false,
                position: 8,
            },
        )
        .unwrap()
    };
    let (request, ack) = prepare(&rt);
    ack.cancel();
    engine.send(Command::SessionEdit(request)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Cancelled);
    let (request, ack) = prepare(&rt);
    rt.project.edited();
    engine.send(Command::SessionEdit(request)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Rejected);
    let (request, ack) = prepare(&rt);
    rt.set_undo_budget_for_test(1);
    engine.send(Command::SessionEdit(request)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Rejected);
    assert_eq!(rt.tracks.len(), 8);
}

#[test]
fn undo_graph_processors_follow_stopped_sample_rate_changes() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 144);
    rt.tracks[2]
        .fx
        .slots
        .push(fx::FxSlot::new(fx::FxId::Delay, 48000.0));
    let id = rt.session.tracks[2].id;
    edit(
        &engine,
        &mut rt,
        Structure::Duplicate {
            axis: Axis::Track,
            id,
            name: "Delayed".into(),
            position: 8,
        },
    );
    replay(&mut rt, Command::Undo);
    rt.set_sample_rate(96000).unwrap();
    replay(&mut rt, Command::Redo);
    assert_eq!(
        rt.tracks[8].fx.slots[0].storage_bytes(),
        fx::FxSlot::required_storage(fx::FxId::Delay, 96000.0)
    );
}

#[test]
fn undo_after_rate_change_cannot_restore_a_graph_above_the_processor_limit() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    for (track, count) in [(2, 128), (3, 128), (4, 8)] {
        for _ in 0..count {
            rt.tracks[track]
                .fx
                .slots
                .push(fx::FxSlot::new(fx::FxId::Delay, 48000.0));
        }
    }
    let old = rt.session.tracks[2].id;
    let (request, ack) = Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        crate::engine::session::Action::Delete {
            axis: Axis::Track,
            id: old,
        },
    )
    .unwrap();
    engine.send(Command::SessionEdit(request)).unwrap();
    rt.process(&mut []);
    assert_eq!(ack.state(), Outcome::Applied);
    edit(
        &engine,
        &mut rt,
        Structure::Track {
            name: "Empty replacement".into(),
            audio: false,
            position: 2,
        },
    );
    assert_ne!(rt.session.tracks[2].id, old);
    rt.set_sample_rate(96000).unwrap();
    let before = rt.undo.checkpoint();
    let replacement = rt.session.clone();
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        test_alloc::Counts::default()
    );
    rt.undo.publish();
    assert_eq!(engine.undo.view().failure, Some(crate::engine::undo::Failure::ProcessorBudget));
    assert_eq!(rt.undo.checkpoint(), before);
    assert_eq!(rt.session, replacement);
    assert!(rt.tracks[2].fx.slots.is_empty());
    // Lowering the stopped output rate makes that same retained undo valid.
    rt.set_sample_rate(48000).unwrap();
    replay(&mut rt, Command::Undo);
    assert_eq!(rt.session.tracks[2].id, old);
    assert!(!rt.session.tracks[2].active);
    assert_eq!(rt.tracks[2].fx.slots.len(), 128);
}

#[test]
fn stale_piano_roll_cannot_edit_a_reused_empty_cell_with_identical_musical_content() {
    use crate::engine::midi_edit::{Document, Region, Request as MidiRequest};
    let (engine, mut rt) = Engine::headless_for_test(48000, 144);
    let baseline = Document::capture(captured(&rt), 2, 7).unwrap();
    let old = rt.session.scenes[7].id;
    let (delete, _) = Request::metadata(
        &rt.session,
        rt.undo.checkpoint().epoch,
        crate::engine::session::Action::Delete {
            axis: Axis::Scene,
            id: old,
        },
    )
    .unwrap();
    rt.apply(Command::SessionEdit(delete));
    edit(
        &engine,
        &mut rt,
        Structure::Scene {
            name: "Replacement".into(),
            position: 7,
        },
    );
    assert_ne!(rt.session.scenes[7].id, old);
    assert_eq!(rt.tracks[2].clips[7].kind, baseline.kind);
    assert_eq!(rt.tracks[2].clips[7].name, baseline.name);
    assert_eq!(rt.tracks[2].clips[7].bars, baseline.bars);
    assert_eq!(rt.tracks[2].clips[7].notes, baseline.notes);
    let (request, ack, _) = MidiRequest::new(
        baseline,
        "Should be rejected".into(),
        Region::full(16.0),
        Vec::new(),
    )
    .unwrap();
    engine.send(Command::MidiEdit(request)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [])),
        test_alloc::Counts::default()
    );
    assert_eq!(ack.state(), Outcome::Rejected);
    assert_eq!(rt.tracks[2].clips[7].kind, ClipKind::Empty);
}

#[test]
fn excessive_processor_storage_is_refused_before_graph_allocation() {
    let mut saved = captured(&rt());
    let delay = Effect {
        id: fx::FxId::Delay,
        offline: None,
        on: true,
        mix: 0.5,
        p: [0.5; 4],
    };
    for track in &mut saved.state.tracks {
        track.fx = vec![delay.clone(); 128];
    }
    assert!(saved.state.validate(&saved.media).is_ok());
    let error = Prepared::from_state(saved.state, saved.media, 96000)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("256 MiB"), "{error}");
}
