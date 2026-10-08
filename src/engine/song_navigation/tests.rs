use super::*;
use crate::engine::{
    clip_launch::Grid,
    midi_edit::Outcome,
    project::{Captured, Prepared},
    test_alloc, Command, Engine, RtEngine,
};
use metadata::Loop;
use std::sync::atomic::AtomicBool;
fn capture(engine: &Engine, rt: &mut RtEngine) -> Captured {
    let handle = engine.project.clone();
    let worker = std::thread::spawn(move || handle.capture(&AtomicBool::new(false)).unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !worker.is_finished() {
        rt.process(&mut []);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    worker.join().unwrap()
}
fn model() -> Model {
    let mut model = Model::default();
    model.add("Intro".into(), 0.0).unwrap();
    model.add("Verse".into(), 4.0).unwrap();
    model.add("Outro".into(), 12.0).unwrap();
    model.loop_region = Some(Loop {
        start: 4.0,
        end: 12.0,
    });
    model
}
fn prepare(
    engine: &Engine,
    rt: &mut RtEngine,
    model: Model,
    looping: bool,
    jump: Option<(f64, Grid)>,
) -> (Box<Request>, crate::engine::midi_edit::Ack) {
    let expected = rt.navigation.saved.clone();
    Request::prepare(
        capture(engine, rt),
        expected,
        model,
        looping,
        jump,
        &AtomicBool::new(false),
    )
    .unwrap()
}
#[test]
fn stable_sections_validate_reorder_delete_duplicate_positions_and_half_open_loops() {
    let mut model = model();
    let id = model.add("同じ / Another".into(), 4.0).unwrap();
    model.update(2, "Moved verse".into(), 15.0).unwrap();
    assert_eq!(model.destination(2), Some(15.0));
    assert_eq!(model.adjacent(4.0, true), Some(12.0));
    assert_eq!(model.adjacent(4.0, false), Some(0.0));
    assert_eq!(
        model.loop_to_next(id).unwrap(),
        Loop {
            start: 4.0,
            end: 12.0
        }
    );
    assert!(model.loop_to_next(2).is_err());
    model.delete(id).unwrap();
    assert!(model.destination(id).is_none());
    assert_eq!(model.add("Fresh".into(), 4.0).unwrap(), id + 1);
    let before = model.clone();
    for (name, beat) in [
        ("", 0.0),
        ("bad\nname", 1.0),
        ("Name", f64::NAN),
        ("Name", -1.0),
        ("Name", 262145.0),
    ] {
        assert!(model.add(name.into(), beat).is_err());
        assert_eq!(model, before);
    }
    let mut duplicate = model.clone();
    duplicate.locators[1].id = duplicate.locators[0].id;
    assert!(duplicate.validate().is_err());
    for region in [
        Loop {
            start: 1.0,
            end: 1.0,
        },
        Loop {
            start: f64::NAN,
            end: 2.0,
        },
        Loop {
            start: 2.0,
            end: 1.0,
        },
    ] {
        assert!(!region.valid());
    }
    model.next_id = 65536;
    assert!(model.add("Exhausted".into(), 1.0).is_err());
}
#[test]
fn live_metadata_ack_undo_redo_stale_and_cancelled_edits_preserve_renderer_ownership() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let (request, ack) = prepare(&engine, &mut rt, model(), false, None);
    rt.playing = true;
    rt.recording = true;
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::SongNavigationEdit(request))),
        Default::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    assert!(rt.playing && rt.recording);
    let original = rt.navigation.saved.clone();
    let (stale, stale_ack) = prepare(&engine, &mut rt, model(), false, None);
    let mut edited = model();
    edited.update(2, "Chorus".into(), 8.0).unwrap();
    let (request, ack) = prepare(&engine, &mut rt, edited.clone(), false, None);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::SongNavigationEdit(request))),
        Default::default()
    );
    assert_eq!(ack.state(), Outcome::Applied);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::SongNavigationEdit(stale))),
        Default::default()
    );
    assert_eq!(stale_ack.state(), Outcome::Rejected);
    assert_eq!(*rt.navigation.saved.as_ref().unwrap().model, edited);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        Default::default()
    );
    assert!(rt
        .navigation
        .saved
        .as_ref()
        .unwrap()
        .same(original.as_ref().unwrap()));
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Redo)),
        Default::default()
    );
    assert_eq!(*rt.navigation.saved.as_ref().unwrap().model, edited);
    let (request, ack) = prepare(&engine, &mut rt, model(), false, None);
    assert!(ack.cancel());
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::SongNavigationEdit(request))),
        Default::default()
    );
    assert_eq!(ack.state(), Outcome::Cancelled);
    assert_eq!(*rt.navigation.saved.as_ref().unwrap().model, edited);
    assert!(rt.playing && rt.recording);
}
#[test]
fn pending_navigation_cancels_on_stop_seek_invalid_destination_edit_and_native_replacement() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let (request, _) = prepare(&engine, &mut rt, model(), false, None);
    rt.apply(Command::SongNavigationEdit(request));
    rt.playing = true;
    rt.apply(Command::SongSeek(0.5));
    let queue = |rt: &mut RtEngine| {
        rt.apply(Command::SongNavigation(Action::Locator {
            id: 2,
            grid: Grid::Bar,
        }));
        assert!(rt.navigation.pending.is_some());
    };
    queue(&mut rt);
    rt.apply(Command::Stop);
    assert!(rt.navigation.pending.is_none());
    rt.playing = true;
    queue(&mut rt);
    rt.apply(Command::TimelineSeek(0.25));
    assert!(rt.navigation.pending.is_none());
    queue(&mut rt);
    rt.apply(Command::SongNavigation(Action::Locator {
        id: 65535,
        grid: Grid::Bar,
    }));
    assert!(rt.navigation.pending.is_none());
    assert_eq!(rt.navigation.error, Some(Error::MissingLocator));
    queue(&mut rt);
    let (request, _) = prepare(&engine, &mut rt, model(), false, None);
    rt.apply(Command::SongNavigationEdit(request));
    assert!(rt.navigation.pending.is_none());
    queue(&mut rt);
    let saved = capture(&engine, &mut rt);
    let mut replacement = Prepared::from_state(saved.state, saved.media, 48000).unwrap();
    replacement.swap_into(&mut rt);
    assert!(rt.navigation.pending.is_none());
}
#[test]
fn native_navigation_reopen_migrates_legacy_and_rejects_versioned_or_invalid_metadata() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let (request, _) = prepare(&engine, &mut rt, model(), true, None);
    rt.apply(Command::SongNavigationEdit(request));
    rt.playing = true;
    rt.apply(Command::SongSeek(0.5));
    rt.apply(Command::SongNavigation(Action::ToggleLoop));
    rt.apply(Command::SongNavigation(Action::Locator {
        id: 2,
        grid: Grid::Bar,
    }));
    let mut saved = capture(&engine, &mut rt);
    saved.state.navigation.as_mut().unwrap().looping = true;
    let raw = serde_json::to_vec(&saved.state).unwrap();
    let reopened: crate::engine::project::State = serde_json::from_slice(&raw).unwrap();
    reopened.validate(&saved.media).unwrap();
    assert_eq!(reopened.navigation, saved.state.navigation);
    let prepared = Prepared::from_state(reopened, saved.media.clone(), 48000).unwrap();
    assert!(prepared.rt.navigation.pending.is_none());
    assert!(prepared.rt.navigation.saved.as_ref().unwrap().looping);
    assert!(!prepared.rt.playing);
    let mut value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    value["version"] = 21.into();
    assert!(serde_json::from_value::<crate::engine::project::State>(value.clone()).is_err());
    value["navigation"] = serde_json::Value::Null;
    assert!(serde_json::from_value::<crate::engine::project::State>(value.clone()).is_err());
    value.as_object_mut().unwrap().remove("navigation");
    let old: crate::engine::project::State = serde_json::from_value(value).unwrap();
    assert!(old.navigation.is_none());
    old.validate(&saved.media).unwrap();
    let mut bad = model();
    bad.loop_region = Some(Loop {
        start: 9.0,
        end: 8.0,
    });
    assert!(Request::prepare(
        capture(&engine, &mut rt),
        rt.navigation.saved.clone(),
        bad,
        false,
        None,
        &AtomicBool::new(false)
    )
    .is_err());
}
#[test]
fn quantized_jumps_and_loop_wraps_follow_exact_sample_time_across_conductor_changes_without_heap() {
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    for rate in [8000, 44100, 48000, 96000] {
        let (engine, rt) = Engine::headless_for_test(rate, 256);
        let mut rt = Box::new(rt);
        rt.metronome = false;
        rt.conductor = Some(
            Conductor::native(
                960,
                vec![
                    Tempo::new(0, 120.0, false).unwrap(),
                    Tempo::new(1920, 180.0, false).unwrap(),
                ],
                vec![
                    Meter {
                        tick: 0,
                        numerator: 7,
                        denominator_power: 3,
                        clocks: 24,
                        thirty_seconds: 8,
                    },
                    Meter {
                        tick: 3360,
                        numerator: 5,
                        denominator_power: 2,
                        clocks: 24,
                        thirty_seconds: 8,
                    },
                ],
                TimingSettings {
                    pickup: 0.5,
                    ..Default::default()
                },
            )
            .unwrap(),
        );
        let (request, _) = prepare(&engine, &mut rt, model(), false, None);
        rt.apply(Command::SongNavigationEdit(request));
        rt.apply(Command::SongSeek(0.25));
        rt.playing = true;
        assert_eq!(
            test_alloc::measure(|| rt.apply(Command::SongNavigation(Action::Locator {
                id: 2,
                grid: Grid::Bar
            }))),
            Default::default()
        );
        assert_eq!(rt.navigation.pending.unwrap().when, 0.5);
        let n = ((0.5 - 0.25) * 0.5 * f64::from(rate)).ceil() as usize;
        let mut before = vec![0.0; n * 2];
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut before)),
            Default::default()
        );
        assert!(rt.navigation.pending.is_some());
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut [0.0; 2])),
            Default::default()
        );
        assert!(rt.navigation.pending.is_none());
        assert!((rt.precise_midi_beat() - (4.0 + 3.0 / f64::from(rate))).abs() < 1e-9);
        let mut sections = model();
        sections.loop_region = Some(Loop {
            start: 1.0,
            end: 3.0,
        });
        let (request, _) = prepare(&engine, &mut rt, sections, true, None);
        rt.apply(Command::SongNavigationEdit(request));
        rt.seek_timeline(rt.conductor.as_ref().unwrap().seconds_at(1.0));
        let period = rt.conductor.as_ref().unwrap().seconds_at(3.0)
            - rt.conductor.as_ref().unwrap().seconds_at(1.0);
        let frames = (period * f64::from(rate)).ceil() as usize;
        let mut output = vec![0.0; 2 * (frames + 1)];
        assert_eq!(
            test_alloc::measure(|| rt.process(&mut output)),
            Default::default()
        );
        let elapsed = (frames + 1) as f64 / f64::from(rate);
        let expected = rt.conductor.as_ref().unwrap().seconds_at(1.0) + elapsed.rem_euclid(period);
        assert!(
            (rt.timeline_seconds() - expected).abs() < 1e-9,
            "rate {rate}: {} vs {expected}",
            rt.timeline_seconds()
        );
        assert!(rt.playing);
        assert!(rt.navigation.saved.as_ref().unwrap().looping);
    }
}
#[test]
fn native_container_and_navigation_midi_presets_roundtrip_with_strict_legacy_guards() {
    use crate::engine::midi::{self, Binding, MsgKind};
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let (request, _) = prepare(&engine, &mut rt, model(), false, None);
    rt.apply(Command::SongNavigationEdit(request));
    let saved = capture(&engine, &mut rt);
    let path = std::env::temp_dir().join(format!(
        "omatainer-song-sections-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    let limits = crate::project_file::Limits::default();
    let cancel = AtomicBool::new(false);
    crate::project_file::save(
        &path,
        &crate::project_file::Bundle {
            state: saved.state.clone(),
            media: saved.media,
        },
        crate::project_file::Overwrite::Never,
        &limits,
        &cancel,
    )
    .unwrap();
    let reopened =
        crate::project_file::load::<crate::engine::project::State>(&path, &limits, &cancel)
            .unwrap();
    assert_eq!(reopened.state.navigation, saved.state.navigation);
    reopened.state.validate(&reopened.media).unwrap();
    std::fs::remove_file(path).unwrap();
    let binding = Binding {
        kind: MsgKind::Note,
        ch: 3,
        data: 60,
        action: midi::Action::SongLocator,
        deck: 0,
        extra: 2,
        relative: None,
        controls: None,
        pair_order: None,
    };
    let port = midi::learn::Endpoint {
        name: "Software navigation fixture".into(),
        id: "fixture:navigation".into(),
    };
    let config = midi::learn::Config {
        mappings: vec![midi::learn::Mapping {
            endpoint: port.clone(),
            binding,
        }],
    };
    let preset =
        midi::presets::Preset::capture("Song sections".into(), String::new(), &port, &config)
            .unwrap();
    assert_eq!(preset.version, midi::presets::VERSION);
    let mut value = serde_json::to_value(&preset).unwrap();
    for version in [1, 2] {
        value["version"] = version.into();
        assert!(midi::presets::Preset::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    }
    let mut preferences =
        crate::preferences::Preferences::defaults(std::path::Path::new("/home/michael"));
    let profile = preferences.profiles.get_mut(&preferences.active).unwrap();
    profile.midi_learn = config;
    profile.midi_presets = vec![preset];
    let bytes = serde_json::to_vec(&preferences).unwrap();
    let (reopened, migrated) = crate::preferences::storage::decode(&bytes).unwrap();
    assert!(!migrated);
    assert_eq!(reopened, preferences);
    let mut previous_navigation = preferences.clone();
    previous_navigation.version = 20;
    for profile in previous_navigation.profiles.values_mut() { for preset in &mut profile.midi_presets { preset.version = 3; } }
    let (reopened, migrated) = crate::preferences::storage::decode(&serde_json::to_vec(&previous_navigation).unwrap()).unwrap();
    assert!(migrated);
    previous_navigation.version = crate::preferences::VERSION;
    assert_eq!(reopened, previous_navigation);
    let mut value = serde_json::to_value(&preferences).unwrap();
    value["version"] = 19.into();
    assert!(crate::preferences::storage::decode(&serde_json::to_vec(&value).unwrap()).is_err());
    value["profiles"]["Studio"]["midi_learn"]["mappings"] = serde_json::json!([]);
    value["profiles"]["Studio"]["midi_presets"] = serde_json::json!([]);
    let (old, migrated) =
        crate::preferences::storage::decode(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(migrated);
    assert_eq!(old.version, crate::preferences::VERSION);
    let encoder = Binding {
        kind: MsgKind::Cc,
        action: midi::Action::Master,
        extra: 0,
        controls: Some(midi::ControlSpec {
            invert: true,
            min: 0.2,
            max: 0.8,
        }),
        ..binding
    };
    let config = midi::learn::Config {
        mappings: vec![midi::learn::Mapping {
            endpoint: port.clone(),
            binding: encoder,
        }],
    };
    let mut encoder_preset = midi::presets::Preset::capture(
        "Previous release encoder".into(),
        String::new(),
        &port,
        &config,
    )
    .unwrap();
    encoder_preset.version = 2;
    assert_eq!(
        midi::presets::Preset::decode(&serde_json::to_vec(&encoder_preset).unwrap()).unwrap(),
        encoder_preset
    );
    let mut previous =
        crate::preferences::Preferences::defaults(std::path::Path::new("/home/fixture"));
    previous.version = 19;
    previous.profiles.get_mut("Studio").unwrap().midi_learn = config;
    previous.profiles.get_mut("Studio").unwrap().midi_presets = vec![encoder_preset];
    let (migrated, changed) =
        crate::preferences::storage::decode(&serde_json::to_vec(&previous).unwrap()).unwrap();
    assert!(changed);
    previous.version = crate::preferences::VERSION;
    assert_eq!(migrated, previous);
}
#[test]
fn learned_navigation_uses_stable_ids_and_only_press_edges_through_production_input_worker() {
    use crate::engine::midi::{self, Binding, MidiMap, MsgKind, UnmappedNotes};
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let (request, _) = prepare(&engine, &mut rt, model(), false, None);
    rt.apply(Command::SongNavigationEdit(request));
    let binding = Binding {
        kind: MsgKind::Note,
        ch: 3,
        data: 60,
        action: midi::Action::SongLocator,
        deck: 0,
        extra: 2,
        relative: None,
        controls: None,
        pair_order: None,
    };
    let port = midi::learn::Endpoint {
        name: "Software navigation fixture".into(),
        id: "fixture:navigation".into(),
    };
    engine
        .cmd
        .midi_learn()
        .configure(midi::learn::Config {
            mappings: vec![midi::learn::Mapping {
                endpoint: port.clone(),
                binding,
            }],
        })
        .unwrap();
    let mut input = engine.midi.open_for_test(
        &engine.cmd,
        510,
        MidiMap {
            name: port.name.clone(),
            matchers: vec![],
            bindings: vec![],
            unmapped_notes: UnmappedNotes::Ignore,
        },
        &port.name,
        &port.id,
    );
    let mut changed = model();
    changed.update(2, "Moved and renamed".into(), 15.0).unwrap();
    let (request, _) = prepare(&engine, &mut rt, changed, false, None);
    rt.apply(Command::SongNavigationEdit(request));
    input.push(&[0x93, 60, 100]);
    input.push(&[0x83, 60, 0]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while rt.beat != 15.0 {
        rt.process(&mut []);
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(rt.beat, 15.0);
    rt.apply(Command::SongSeek(7.0));
    input.push(&[0x83, 60, 0]);
    for _ in 0..20 {
        rt.process(&mut []);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(rt.beat, 7.0);
    let bad = Binding {
        extra: 0,
        ..binding
    };
    assert!(midi::learn::validate_binding(&bad).is_err());
    assert!(midi::learn::validate_binding(&Binding {
        kind: MsgKind::Cc,
        ..binding
    })
    .is_err());
}
#[test]
fn quantized_audio_jump_changes_to_the_destination_on_the_exact_rendered_sample() {
    use crate::engine::{
        arrangement::{self, Instance, Source},
        audio_clip,
        dsp::Sample,
        session,
    };
    use std::sync::Arc;
    for rate in [8000, 44100, 48000] {
        let (engine, rt) = Engine::headless_for_test(rate, 256);
        let mut rt = Box::new(rt);
        rt.apply(Command::Stop);
        rt.bpm = 120.0;
        rt.metronome = false;
        let mut captured = capture(&engine, &mut rt);
        let audio = Arc::new(Sample {
            name: "independent jump oracle".into(),
            sr: rate,
            ch: 2,
            data: (0..rate * 3)
                .flat_map(|i| {
                    let a = (i as f32 * 0.031).sin() * 0.04;
                    [a, a * 0.7]
                })
                .collect(),
            peaks: Default::default(),
            spectrum: None,
            bpm: 120.0,
            path: String::new(),
        });
        let index = captured.media.len();
        captured.media.push(audio.clone());
        let mut clip = captured.state.tracks[0].clips[7].clone();
        clip.kind = crate::engine::ClipKind::Audio;
        clip.notes.clear();
        clip.lanes = None;
        clip.region = None;
        clip.name = audio.name.clone();
        clip.audio = Some(index);
        clip.audio_region = Some(audio_clip::Region::full(&audio, 120.0).unwrap());
        clip.bars = 1.5;
        let song = arrangement::Model {
            enabled: true,
            next_id: 3,
            sources: vec![Source {
                id: 1,
                clip,
                audio_clock: None,
            }],
            instances: vec![Instance {
                id: 2,
                source: 1,
                track: captured
                    .state
                    .session
                    .as_ref()
                    .unwrap()
                    .reference(session::Axis::Track, 0)
                    .unwrap(),
                start: 0.0,
                offset: 0.0,
                duration: 6.0,
                repeating: false,
                gain: 1.0,
                            fades: None,
                fade_link: 0,
                crossfade: None,
}],
        };
        let (request, _) =
            arrangement::edit::Request::prepare(captured, song, &AtomicBool::new(false)).unwrap();
        rt.apply(Command::ArrangementEdit(request));
        let (request, _) = prepare(&engine, &mut rt, model(), false, None);
        rt.apply(Command::SongNavigationEdit(request));
        let saved = capture(&engine, &mut rt);
        let mut oracle = Prepared::from_state(saved.state, saved.media, rate)
            .unwrap()
            .into_offline();
        rt.apply(Command::SongSeek(0.25));
        oracle.apply(Command::SongSeek(0.25));
        rt.playing = true;
        oracle.playing = true;
        rt.apply(Command::SongNavigation(Action::Locator {
            id: 2,
            grid: Grid::Quarter,
        }));
        let boundary = ((1.0 - 0.25) * 0.5 * f64::from(rate)).ceil() as usize;
        let mut actual = [0.0; 2];
        let mut expected = [0.0; 2];
        let mut nonzero = false;
        assert_eq!(
            test_alloc::measure(|| {
                for frame in 0..boundary + 512 {
                    if frame == boundary {
                        oracle.apply(Command::SongSeek(4.0));
                    }
                    rt.process(&mut actual);
                    oracle.process(&mut expected);
                    for c in 0..2 {
                        assert!(
                            (actual[c] - expected[c]).abs() < 1e-6,
                            "rate{rate} frame{frame} {actual:?} vs{expected:?}"
                        );
                    }
                    nonzero |= actual.iter().any(|s| s.abs() > 0.001);
                }
            }),
            Default::default()
        );
        assert!(nonzero);
    }
}
#[test]
fn song_jump_finishes_the_actual_recording_take_and_keeps_live_input_held_until_its_release() {
    use crate::engine::{dsp::VoiceOwner, InputKey};
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    rt.selected_track = 1;
    rt.selected_scene = 7;
    rt.quant = 0.0;
    let (request, _) = prepare(&engine, &mut rt, model(), false, None);
    rt.apply(Command::SongNavigationEdit(request));
    rt.tracks[1].clips[7].kind = crate::engine::ClipKind::Midi;
    rt.tracks[1].clips[7].bars = 2.0;
    rt.apply(Command::FireClip {
        track: 1,
        scene: 7,
        looping: true,
    });
    rt.apply(Command::Record);
    rt.apply(Command::LiveNoteOn {
        source: 511,
        ch: 3,
        note: 65,
        vel: 100,
    });
    rt.process(&mut [0.0; 4800]);
    assert!(rt.recording);
    assert_eq!(rt.tracks[1].clips[7].notes.len(), 1);
    let input = InputKey::Midi {
        source: 511,
        ch: 3,
        note: 65,
    };
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .any(|v| v.input == Some(input) && matches!(v.env.stage, 1..=3)));
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::SongNavigation(Action::Locator {
            id: 2,
            grid: Grid::Immediate
        }))),
        Default::default()
    );
    assert!(!rt.recording);
    assert!(rt.playing);
    assert!(rt.tracks[1].clips[7].notes[0].len > 0.0);
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .any(|v| v.input == Some(input) && matches!(v.env.stage, 1..=3)));
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .filter(|v| v.owner == VoiceOwner::Clip)
        .all(|v| !matches!(v.env.stage, 1..=3)));
    rt.apply(Command::LiveNoteOff {
        source: 511,
        ch: 3,
        note: 65,
    });
    assert!(rt.tracks[1]
        .poly
        .voices
        .iter()
        .filter(|v| v.input == Some(input))
        .all(|v| !matches!(v.env.stage, 1..=3)));
}
#[test]
fn undo_branching_and_native_reopen_never_reuse_a_previously_mapped_section_id() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    let (request, _) = prepare(&engine, &mut rt, model(), false, None);
    rt.apply(Command::SongNavigationEdit(request));
    let mut draft = model();
    let old_id = draft.add("Mapped bridge".into(), 9.0).unwrap();
    let (request, _) = prepare(&engine, &mut rt, draft, false, None);
    rt.apply(Command::SongNavigationEdit(request));
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        Default::default()
    );
    let current = rt.navigation.saved.as_ref().unwrap();
    assert!(current.model.destination(old_id).is_none());
    assert_eq!(current.next_id, u32::from(old_id) + 1);
    let mut draft = (*current.model).clone();
    draft.next_id = current.next_id;
    let fresh = draft.add("New branch".into(), 9.0).unwrap();
    assert_ne!(fresh, old_id);
    let (request, _) = prepare(&engine, &mut rt, draft, false, None);
    rt.apply(Command::SongNavigationEdit(request));
    let saved = capture(&engine, &mut rt);
    let mut reopened = Prepared::from_state(saved.state, saved.media, 48000)
        .unwrap()
        .into_offline();
    assert_eq!(
        reopened.navigation.saved.as_ref().unwrap().next_id,
        u32::from(fresh) + 1
    );
    reopened.apply(Command::SongNavigation(Action::Locator {
        id: old_id,
        grid: Grid::Immediate,
    }));
    assert_eq!(reopened.navigation.error, Some(Error::MissingLocator));
    assert_eq!(reopened.beat, 0.0);
    let mut zeros = Model::default();
    zeros.add("Positive zero".into(), 0.0).unwrap();
    zeros.add("Negative zero".into(), -0.0).unwrap();
    zeros.validate().unwrap();
    assert_eq!(zeros.locators[0].id, 1);
    assert_eq!(zeros.locators[1].id, 2);
}
#[test]
fn a_tempo_change_cannot_leave_an_out_of_range_queued_jump_repeating_forever() {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    rt.bpm = 240.0;
    rt.playing = true;
    rt.apply(Command::SongSeek(0.5));
    rt.apply(Command::SongNavigation(Action::Beat {
        beat: 262144.0,
        grid: Grid::Quarter,
    }));
    assert!(rt.navigation.pending.is_some());
    rt.bpm = 30.0;
    rt.apply(Command::SongSeek(0.5));
    rt.bpm = 240.0;
    rt.apply(Command::SongNavigation(Action::Beat {
        beat: 262144.0,
        grid: Grid::Quarter,
    }));
    rt.bpm = 30.0;
    rt.navigation.pending.as_mut().unwrap().when = rt.precise_midi_beat();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [0.0; 2])),
        Default::default()
    );
    assert!(rt.navigation.pending.is_none());
    assert_eq!(rt.navigation.error, Some(Error::PositionLimit));
    assert!(rt.beat < 1.0);
}
