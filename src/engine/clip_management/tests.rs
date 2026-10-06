use super::*;
use crate::engine::{
    midi_edit::{NoteId, Outcome},
    project::{Captured, Prepared, State},
    test_alloc, ClipKind, Command, Engine, MidiNote, RtEngine, Sample,
};
use edit::{Action, Request, Slot};
use std::sync::{atomic::AtomicBool, Arc};
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
fn slot(rt: &RtEngine, t: usize, s: usize) -> Slot {
    Slot::at(&rt.session, t, s).unwrap()
}
fn apply(engine: &Engine, rt: &mut RtEngine, action: Action) {
    let captured = capture(engine, rt);
    let (request, ack) = Request::prepare(captured, action, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::ClipManage(request))),
        Default::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
}
fn pcm() -> Arc<Sample> {
    Arc::new(Sample {
        name: "shared preset PCM".into(),
        sr: 48000,
        ch: 2,
        data: (0..48000)
            .flat_map(|i| {
                [
                    (i as f32 * 0.041).sin() * 0.1,
                    (i as f32 * 0.037).cos() * 0.1,
                ]
            })
            .collect(),
        peaks: Default::default(),
        spectrum: None,
        bpm: 120.0,
        path: String::new(),
    })
}
fn midi(t: usize, s: usize) -> crate::engine::Clip {
    crate::engine::Clip {
        properties: Properties {
            color: Some([20 + t as u8, 40 + s as u8, 210]),
            disabled: false,
        },
        audio_region: None,
        lanes: Some(
            crate::engine::midi_data::Lanes::named(
                480,
                1920,
                vec![crate::midi_file::Message {
                    tick: 0,
                    order: 1,
                    bytes: [0xb0, 74, 93],
                    length: 3,
                }],
                vec![],
                vec![crate::engine::midi_data::Label {
                    channel: 0,
                    control: crate::engine::midi_data::ControlKind::Cc { controller: 74 },
                    name: "Color sweep".into(),
                }],
            )
            .unwrap(),
        ),
        region: None,
        kind: ClipKind::Midi,
        name: format!("phrase {t}/{s}"),
        bars: 1.0,
        notes: vec![MidiNote {
            id: NoteId::new(),
            pitch: 60,
            vel: 90,
            start: 0.0,
            len: 1.0,
            muted: false,
            channel: 0,
            release_vel: 64,
            source_timing: None,
        }],
        gain: 0.7,
        audio: None,
    }
}
#[test]
fn eight_contrasting_clips_copy_move_metadata_delete_undo_and_native_reopen_retain_content() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let audio = pcm();
    for t in 0..4 {
        for s in 0..2 {
            rt.tracks[t].clips[s] = if s == 0 {
                midi(t, s)
            } else {
                let mut region =
                    crate::engine::audio_clip::Region::full(&audio, 110.0 + t as f32 * 5.0)
                        .unwrap();
                region.start = 1000;
                region.end = 36000;
                region.loop_start = 2000;
                region.loop_end = 20000;
                region.loop_enabled = true;
                region.reverse = t % 2 == 0;
                region.transpose = t as f32;
                let plan = region.prepare(&audio).unwrap();
                crate::engine::Clip {
                    properties: Properties {
                        color: Some([200, t as u8 * 40, 80]),
                        disabled: t == 3,
                    },
                    kind: ClipKind::Audio,
                    name: format!("audio {t}"),
                    audio_region: Some(plan),
                    bars: (plan.duration_beats / 4.0) as f32,
                    notes: vec![],
                    region: None,
                    lanes: None,
                    gain: 0.6,
                    audio: Some(audio.clone()),
                }
            };
        }
    }
    let a = slot(&rt, 0, 0);
    let b = slot(&rt, 1, 7);
    let c = slot(&rt, 2, 7);
    let original = rt.tracks[0].clips[0].notes[0].id;
    apply(
        &engine,
        &mut rt,
        Action::Copy {
            source: a,
            destination: b,
        },
    );
    assert_eq!(rt.tracks[0].clips[0].notes[0].id, original);
    let duplicate = rt.tracks[1].clips[7].notes[0].id;
    assert_ne!(duplicate, original);
    apply(
        &engine,
        &mut rt,
        Action::Move {
            source: b,
            destination: c,
        },
    );
    assert_eq!(rt.tracks[1].clips[7].kind, ClipKind::Empty);
    assert_eq!(rt.tracks[2].clips[7].notes[0].id, duplicate);
    let p = Properties {
        color: Some([17, 240, 65]),
        disabled: true,
    };
    apply(
        &engine,
        &mut rt,
        Action::Metadata {
            target: c,
            name: "colored disabled phrase".into(),
            properties: p,
        },
    );
    for cmd in [Command::Undo, Command::Redo] {
        assert_eq!(test_alloc::measure(|| rt.apply(cmd)), Default::default());
    }
    assert_eq!(rt.tracks[2].clips[7].properties, p);
    apply(&engine, &mut rt, Action::Delete { target: c });
    assert_eq!(rt.tracks[2].clips[7].kind, ClipKind::Empty);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        Default::default()
    );
    assert_eq!(rt.tracks[2].clips[7].properties, p);
    let source = slot(&rt, 0, 1);
    let destination = slot(&rt, 3, 7);
    apply(
        &engine,
        &mut rt,
        Action::Copy {
            source,
            destination,
        },
    );
    assert!(Arc::ptr_eq(
        rt.tracks[3].clips[7].audio.as_ref().unwrap(),
        &audio
    ));
    let captured = capture(&engine, &mut rt);
    captured.state.validate(&captured.media).unwrap();
    let root = std::env::temp_dir().join(format!("clip-manager-project-{}.omat", NoteId::new()));
    crate::project_file::save(
        &root,
        &crate::project_file::Bundle {
            state: captured.state,
            media: captured.media,
        },
        crate::project_file::Overwrite::Never,
        &Default::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let bundle =
        crate::project_file::load::<State>(&root, &Default::default(), &AtomicBool::new(false))
            .unwrap();
    let reopened = Prepared::from_state(bundle.state, bundle.media, 48000)
        .unwrap()
        .into_offline();
    assert_eq!(reopened.tracks[2].clips[7].properties, p);
    assert_eq!(reopened.tracks[2].clips[7].name, "colored disabled phrase");
    assert_eq!(reopened.tracks[2].clips[7].notes[0].id, duplicate);
    let shared = reopened.tracks[0].clips[1].audio.as_ref().unwrap();
    for t in 0..4 {
        assert!(Arc::ptr_eq(
            reopened.tracks[t].clips[1].audio.as_ref().unwrap(),
            shared
        ));
    }
    assert!(Arc::ptr_eq(
        reopened.tracks[3].clips[7].audio.as_ref().unwrap(),
        shared
    ));
    std::fs::remove_file(root).unwrap();
}
#[test]
fn presets_embed_sources_and_retain_regions_metadata_lanes_without_overwriting_files() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let audio = pcm();
    rt.tracks[0].clips[7] = midi(0, 7);
    let captured = capture(&engine, &mut rt);
    let bundle = preset::Preset::capture(
        captured.state.tracks[0].clips[7].clone(),
        &captured.media,
        &AtomicBool::new(false),
    )
    .unwrap();
    let root = std::env::temp_dir().join(format!("clip-preset-{}.omatclip", NoteId::new()));
    assert_eq!(
        preset::write(&bundle, &root, &AtomicBool::new(false)).unwrap(),
        crate::project_file::SaveOutcome::Durable
    );
    let before = std::fs::read(&root).unwrap();
    assert!(preset::write(&bundle, &root, &AtomicBool::new(false)).is_err());
    assert_eq!(before, std::fs::read(&root).unwrap());
    let read = preset::read(&root, &AtomicBool::new(false)).unwrap();
    assert_eq!(read.state.clip.lanes, bundle.state.clip.lanes);
    let target = slot(&rt, 1, 7);
    apply(
        &engine,
        &mut rt,
        Action::Insert {
            target,
            preset: read,
        },
    );
    assert_ne!(
        rt.tracks[1].clips[7].notes[0].id,
        rt.tracks[0].clips[7].notes[0].id
    );
    std::fs::remove_file(&root).unwrap();
    let mut captured = capture(&engine, &mut rt);
    let mut saved = captured.state.tracks[0].clips[7].clone();
    saved.kind = ClipKind::Audio;
    saved.notes.clear();
    saved.lanes = None;
    saved.audio = Some(captured.media.len());
    captured.media.push(audio.clone());
    let mut region = crate::engine::audio_clip::Region::full(&audio, 120.0).unwrap();
    region.start = 1500;
    region.end = 40000;
    region.loop_start = 2000;
    region.loop_end = 8000;
    region.loop_enabled = true;
    region.reverse = true;
    region.transpose = -3.0;
    saved.audio_region = Some(region);
    saved.bars = (region.prepare(&audio).unwrap().duration_beats / 4.0) as f32;
    saved.properties.disabled = true;
    let bundle =
        preset::Preset::capture(saved.clone(), &captured.media, &AtomicBool::new(false)).unwrap();
    assert!(Arc::ptr_eq(&bundle.media[0], &audio));
    preset::write(&bundle, &root, &AtomicBool::new(false)).unwrap();
    let read = preset::read(&root, &AtomicBool::new(false)).unwrap();
    assert_eq!(read.state.clip.audio_region, saved.audio_region);
    assert_eq!(read.state.clip.properties, saved.properties);
    let mut media = vec![audio.clone()];
    let inserted = read
        .state
        .insert(&read.media, &mut media, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(inserted.audio, Some(0));
    assert_eq!(media.len(), 1);
    assert!(Arc::ptr_eq(&media[0], &audio));
    assert!(preset::read(&root, &AtomicBool::new(true)).is_err());
    std::fs::remove_file(root).unwrap();
}
#[test]
fn cancelled_stale_active_protected_and_budget_refusals_acknowledge_and_preserve_slots() {
    for mode in 0..6 {
        let (engine, rt) = Engine::headless_for_test(48000, 256);
        let mut rt = Box::new(rt);
        rt.tracks[0].clips[7] = midi(0, 7);
        let target = slot(&rt, 0, 7);
        let captured = capture(&engine, &mut rt);
        let (request, ack) =
            Request::prepare(captured, Action::Delete { target }, &AtomicBool::new(false)).unwrap();
        match mode {
            0 => {
                ack.cancel();
            }
            1 => rt.apply(Command::Master(0.4)),
            2 => rt.apply(Command::FireClip {
                track: 0,
                scene: 7,
                looping: true,
            }),
            3 => {
                rt.apply(Command::PerformanceMode(true));
            }
            4 => rt.set_undo_budget_for_test(1),
            5 => rt.recording = true,
            _ => unreachable!(),
        }
        if mode == 3 {
            assert!(engine.cmd.send(Command::ClipManage(request)).is_err());
        } else {
            assert_eq!(
                test_alloc::measure(|| rt.apply(Command::ClipManage(request))),
                Default::default()
            );
        }
        assert_ne!(ack.state(), Outcome::Applied);
        assert_ne!(ack.state(), Outcome::Pending);
        assert_eq!(rt.tracks[0].clips[7].name, "phrase 0/7");
        assert_eq!(rt.tracks[0].clips[7].kind, ClipKind::Midi);
    }
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let captured = capture(&engine, &mut rt);
    let target = slot(&rt, 0, 7);
    assert!(Request::prepare(
        captured,
        Action::CreateMidi {
            target,
            name: "cancelled".into(),
            bars: 1.0
        },
        &AtomicBool::new(true)
    )
    .is_err());
}
#[test]
fn disabled_launch_preserves_another_clip_and_native_resume_never_starts_it() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    rt.tracks[2].clips[0] = midi(2, 0);
    rt.tracks[2].clips[7] = midi(2, 7);
    rt.tracks[2].clips[7].properties.disabled = true;
    rt.apply(Command::FireClip {
        track: 2,
        scene: 0,
        looping: true,
    });
    let prior = rt.tracks[2].playing.unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::FireClip {
            track: 2,
            scene: 7,
            looping: false
        })),
        Default::default()
    );
    assert_eq!(rt.tracks[2].playing.unwrap().scene, prior.scene);
    assert_eq!(rt.tracks[2].playing.unwrap().looping, prior.looping);
    let mut captured = capture(&engine, &mut rt);
    captured.state.tracks[2].launch = Some(crate::engine::project::Launch {
        scene: 7,
        start_beat: 0.0,
        looping: true,
    });
    let mut reopened = Prepared::from_state(captured.state, captured.media, 48000)
        .unwrap()
        .into_offline();
    reopened.apply(Command::Play);
    reopened.process(&mut [0.0; 256]);
    assert!(reopened.tracks[2].playing.is_none());
}
#[test]
fn editing_a_stopped_slot_preserves_other_clip_and_live_voice_owners_and_blocks_active_undo() {
    use crate::engine::dsp::VoiceOwner;
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    rt.quantize = false;
    rt.tracks[2].clips[0] = midi(2, 0);
    rt.tracks[2].clips[7] = midi(2, 7);
    rt.apply(Command::Select { track: 2, scene: 7 });
    rt.apply(Command::FireClip {
        track: 2,
        scene: 0,
        looping: true,
    });
    rt.apply(Command::LiveNoteOn {
        source: 44,
        ch: 0,
        note: 72,
        vel: 100,
    });
    rt.process(&mut [0.0; 512]);
    let held = |rt: &RtEngine, owner| {
        rt.tracks[2]
            .poly
            .voices
            .iter()
            .filter(|v| v.owner == owner && matches!(v.env.stage, 1..=3))
            .count()
    };
    let clips = held(&rt, VoiceOwner::Clip);
    let live = held(&rt, VoiceOwner::Live);
    assert!(clips > 0 && live > 0);
    let target = slot(&rt, 2, 7);
    apply(
        &engine,
        &mut rt,
        Action::Metadata {
            target,
            name: "changed stopped slot".into(),
            properties: Default::default(),
        },
    );
    assert_eq!(rt.tracks[2].playing.unwrap().scene, 0);
    assert_eq!(held(&rt, VoiceOwner::Clip), clips);
    assert_eq!(held(&rt, VoiceOwner::Live), live);
    rt.apply(Command::FireClip {
        track: 2,
        scene: 7,
        looping: true,
    });
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        Default::default()
    );
    assert_eq!(rt.tracks[2].clips[7].name, "changed stopped slot");
    rt.apply(Command::StopTrack { track: 2 });
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        Default::default()
    );
    assert_eq!(rt.tracks[2].clips[7].name, "phrase 2/7");
    assert_eq!(held(&rt, VoiceOwner::Live), live);
}
#[test]
fn occupied_destinations_removed_identities_and_source_memory_budgets_preserve_content() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    rt.tracks[0].clips[7] = midi(0, 7);
    rt.tracks[1].clips[7] = midi(1, 7);
    let a = slot(&rt, 0, 7);
    let b = slot(&rt, 1, 7);
    for action in [
        Action::Copy {
            source: a,
            destination: b,
        },
        Action::Move {
            source: a,
            destination: b,
        },
        Action::Copy {
            source: a,
            destination: a,
        },
    ] {
        assert!(
            Request::prepare(capture(&engine, &mut rt), action, &AtomicBool::new(false)).is_err()
        );
    }
    let mut removed = a;
    removed.track.namespace = [99, 99];
    assert!(Request::prepare(
        capture(&engine, &mut rt),
        Action::Delete { target: removed },
        &AtomicBool::new(false)
    )
    .is_err());
    let captured = capture(&engine, &mut rt);
    let mut saved = captured.state.tracks[0].clips[7].clone();
    saved.kind = ClipKind::Audio;
    saved.notes.clear();
    saved.lanes = None;
    saved.audio = Some(0);
    let audio = pcm();
    let bundle = preset::Preset::capture(saved, &[audio], &AtomicBool::new(false)).unwrap();
    let destination = slot(&rt, 2, 7);
    let (request, ack) = Request::prepare(
        captured,
        Action::Insert {
            target: destination,
            preset: bundle,
        },
        &AtomicBool::new(false),
    )
    .unwrap();
    rt.set_undo_budget_for_test(32768);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::ClipManage(request))),
        Default::default()
    );
    assert_eq!(ack.state(), Outcome::Rejected);
    assert_eq!(rt.tracks[2].clips[7].kind, ClipKind::Empty);
    assert_eq!(rt.tracks[0].clips[7].name, "phrase 0/7");
}
#[test]
fn native_clip_properties_migrate_defaults_and_refuse_legacy_injection_and_unknown_fields() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let captured = capture(&engine, &mut rt);
    let mut raw = serde_json::to_value(captured.state).unwrap();
    raw["version"] = 19.into();
    let legacy: State = serde_json::from_value(raw.clone()).unwrap();
    assert!(legacy
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .all(|c| c.properties.is_default()));
    raw["tracks"][0]["clips"][0]["properties"] = serde_json::json!({"color":null,"disabled":false});
    assert!(serde_json::from_value::<State>(raw.clone()).is_err());
    raw["version"] = 20.into();
    assert!(serde_json::from_value::<State>(raw.clone()).is_ok());
    raw["tracks"][0]["clips"][0]["properties"]["unknown"] = true.into();
    assert!(serde_json::from_value::<State>(raw).is_err());
}
#[test]
fn duplicate_refuses_aggregate_native_note_overflow_before_admission() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    for track in &mut rt.tracks {
        for clip in &mut track.clips {
            *clip = crate::engine::Clip::empty();
        }
    }
    for scene in 0..8 {
        let mut clip = midi(0, scene);
        clip.notes = (0..8192)
            .map(|i| MidiNote {
                id: NoteId::new(),
                start: i as f32 / 4096.0,
                ..clip.notes[0]
            })
            .collect();
        rt.tracks[0].clips[scene] = clip;
    }
    let captured = capture(&engine, &mut rt);
    captured.state.validate(&captured.media).unwrap();
    let source = slot(&rt, 0, 0);
    let destination = slot(&rt, 1, 7);
    let error = Request::prepare(
        captured,
        Action::Copy {
            source,
            destination,
        },
        &AtomicBool::new(false),
    )
    .unwrap_err();
    assert!(error.contains("note count"), "{error}");
    assert_eq!(rt.tracks[0].clips[0].notes.len(), 8192);
    assert_eq!(rt.tracks[1].clips[7].kind, ClipKind::Empty);
}
