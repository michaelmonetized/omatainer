use super::*;
use crate::engine::{
    cue_metadata::{Name, Style},
    deck_controls::{Control, SavedLoopAction},
    load_receipt::{Media, Receipt},
    project::{Captured, Prepared},
    test_alloc, Command, Engine, RtEngine,
};
use std::sync::atomic::AtomicBool;

fn command(rt: &RtEngine, id: u8, action: SavedLoopAction) -> Command {
    Command::DeckControl {
        source: 77,
        deck: 0,
        control: Control::SavedLoop {
            media_key: rt.decks[0].history_key,
            id,
            action,
        },
    }
}
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
fn fill(rt: &mut RtEngine) {
    let rate = f64::from(rt.decks[0].audio.as_ref().unwrap().sr);
    for id in 1..=8 {
        rt.decks[0].loop_start = (f64::from(id) - 1.0) * rate * 0.5;
        rt.decks[0].loop_len = rate * 0.25;
        rt.apply(command(rt, id, SavedLoopAction::Save));
        let style = Style {
            name: Name::new(&format!("Loop {id} / 東京 🎵")).unwrap(),
            color: Some([id * 25, 100, 220]),
        };
        rt.apply(command(rt, id, SavedLoopAction::Style { style }));
    }
}
#[test]
fn eight_source_qualified_slots_preserve_identity_name_order_receipts_and_undo_without_callback_heap_work(
) {
    let (engine, rt) = Engine::headless_for_test(44_100, 256);
    let mut rt = Box::new(rt);
    fill(&mut rt);
    let bank = rt.decks[0].preparation().unwrap().saved_loops;
    assert!(bank.valid());
    let words = bank.words();
    assert_eq!(
        test_alloc::measure(|| assert_eq!(Bank::from_words(words), Some(bank))),
        Default::default()
    );
    assert_eq!(
        engine.initial_playback[0]
            .as_ref()
            .unwrap()
            .preparation()
            .unwrap()
            .1
            .saved_loops,
        bank
    );
    let cue = rt.decks[0].cue_pos;
    let other = rt.decks[1].preparation();
    rt.clear_undo_for_test();
    let move_slot = command(&rt, 8, SavedLoopAction::Move { position: 0 });
    assert_eq!(
        test_alloc::measure(|| rt.apply(move_slot)),
        Default::default()
    );
    let moved = rt.decks[0].preparation().unwrap().saved_loops;
    assert_eq!(moved.order, [8, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(moved.slots, bank.slots);
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        Default::default()
    );
    assert_eq!(rt.decks[0].preparation().unwrap().saved_loops, bank);
    rt.apply(Command::Redo);
    for id in 1..=8 {
        rt.apply(command(
            &rt,
            id,
            SavedLoopAction::Recall { activate: false },
        ));
        let slot = bank.slots[usize::from(id - 1)].unwrap();
        let d = &rt.decks[0];
        assert_eq!(d.controls.status().loop_slot, id - 1);
        assert!(
            (d.loop_start / f64::from(d.audio.as_ref().unwrap().sr) - slot.start).abs() < 1e-12
        );
        assert!((d.loop_len / f64::from(d.audio.as_ref().unwrap().sr) - slot.length).abs() < 1e-12);
        assert!(!d.playing && !d.loop_on);
    }
    rt.clear_undo_for_test();
    rt.apply(command(&rt, 8, SavedLoopAction::Delete));
    assert!(rt.decks[0].preparation().unwrap().saved_loops.slots[7].is_none());
    assert_eq!(
        rt.decks[0].preparation().unwrap().saved_loops.order,
        moved.order
    );
    assert_eq!(rt.decks[0].preparation().unwrap().saved_loops.selected, 8);
    rt.apply(Command::Undo);
    assert_eq!(
        rt.decks[0].preparation().unwrap().saved_loops.slots[7],
        bank.slots[7]
    );
    assert_eq!(rt.decks[0].cue_pos, cue);
    assert_eq!(rt.decks[1].preparation(), other);
    let stale = command(&rt, 1, SavedLoopAction::Recall { activate: true });
    rt.apply(Command::DeckLoadRequested {
        deck: 0,
        media: Media::Builtin(1),
        receipt: Receipt::new(),
    });
    let before = engine.undo.checkpoint();
    assert_eq!(test_alloc::measure(|| rt.apply(stale)), Default::default());
    assert_eq!(engine.undo.checkpoint(), before);
    assert!(rt.decks[0].preparation().unwrap().saved_loops.is_default());
}
#[test]
fn real_native_container_reopens_all_slots_and_armed_region_stopped_and_legacy_headers_refuse_injection(
) {
    let (engine, rt) = Engine::headless_for_test(48_000, 256);
    let mut rt = Box::new(rt);
    fill(&mut rt);
    rt.apply(command(&rt, 3, SavedLoopAction::Recall { activate: true }));
    let prepared = rt.decks[0].preparation().unwrap();
    let captured = capture(&engine, &mut rt);
    let path = std::env::temp_dir().join(format!(
        "omatainer-saved-loops-{}-{}.omat",
        std::process::id(),
        captured.state.session.as_ref().unwrap().namespace[0]
    ));
    let limits = crate::project_file::Limits::default();
    crate::project_file::save(
        &path,
        &crate::project_file::Bundle {
            state: captured.state.clone(),
            media: captured.media.clone(),
        },
        crate::project_file::Overwrite::Never,
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    let disk = crate::project_file::load::<crate::engine::project::State>(
        &path,
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(disk.state.version, crate::engine::project::STATE_VERSION);
    assert_eq!(disk.state.decks[0].saved_loops, prepared.saved_loops);
    let reopened = Prepared::from_state(disk.state.clone(), disk.media.clone(), 44_100).unwrap();
    assert!(!reopened.rt.playing && !reopened.rt.decks[0].playing);
    assert!(reopened.rt.decks[0].loop_on);
    assert_eq!(reopened.rt.decks[0].preparation().unwrap(), prepared);
    let mut legacy = serde_json::to_value(&disk.state).unwrap();
    legacy["version"] = 23.into();
    for deck in legacy["decks"].as_array_mut().unwrap() {
        deck.as_object_mut().unwrap().remove("saved_loops");
    }
    serde_json::from_value::<crate::engine::project::State>(legacy.clone())
        .unwrap()
        .validate(&disk.media)
        .unwrap();
    for injection in [
        serde_json::Value::Null,
        serde_json::to_value(Bank::default()).unwrap(),
        serde_json::to_value(prepared.saved_loops).unwrap(),
    ] {
        let mut value = legacy.clone();
        value["decks"][0]["saved_loops"] = injection;
        assert!(serde_json::from_value::<crate::engine::project::State>(value).is_err());
    }
    let mut invalid = disk.state.clone();
    invalid.decks[0].saved_loops.order = [1; 8];
    assert!(invalid.validate(&disk.media).is_err());
    let mut invalid = disk.state.clone();
    invalid.decks[0].saved_loops.slots[0]
        .as_mut()
        .unwrap()
        .length = 1.0e9;
    assert!(invalid.validate(&disk.media).is_err());
}
#[test]
fn actual_input_worker_recalled_ids_survive_reorder_and_preset_profile_files_guard_new_actions() {
    use crate::engine::midi::{self, Binding, MidiMap, MsgKind, UnmappedNotes};
    let (engine, rt) = Engine::headless_for_test(8_000, 256);
    let mut rt = Box::new(rt);
    fill(&mut rt);
    rt.apply(command(&rt, 8, SavedLoopAction::Move { position: 0 }));
    let bank = rt.decks[0].preparation().unwrap().saved_loops;
    let port = midi::learn::Endpoint {
        name: "Saved loop software fixture".into(),
        id: "fixture:saved-loops".into(),
    };
    let bindings: Vec<_> = (0..8)
        .map(|extra| Binding {
            kind: MsgKind::Note,
            ch: 2,
            data: 60 + extra as u8,
            action: midi::Action::DeckSavedLoopRecall,
            deck: 0,
            extra,
            relative: None,
            controls: None,
            pair_order: None,
        })
        .collect();
    let config = midi::learn::Config {
        mappings: bindings
            .iter()
            .map(|binding| midi::learn::Mapping {
                endpoint: port.clone(),
                binding: *binding,
            })
            .collect(),
    };
    engine.cmd.midi_learn().configure(config.clone()).unwrap();
    let mut input = engine.midi.open_for_test(
        &engine.cmd,
        2200,
        MidiMap {
            name: port.name.clone(),
            matchers: vec![],
            bindings: vec![],
            unmapped_notes: UnmappedNotes::Ignore,
        },
        &port.name,
        &port.id,
    );
    for id in 1..=8 {
        input.push(&[0x92, 59 + id, 100]);
        input.push(&[0x82, 59 + id, 0]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !rt.decks[0].loop_on || rt.decks[0].controls.status().loop_slot != id - 1 {
            rt.process(&mut []);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        let slot = bank.slots[usize::from(id - 1)].unwrap();
        let d = &rt.decks[0];
        assert!((d.pos / f64::from(d.audio.as_ref().unwrap().sr) - slot.start).abs() < 1e-12);
        assert!(!d.playing);
        assert_eq!(d.preparation().unwrap().saved_loops.order, bank.order);
    }
    let rate = f64::from(rt.decks[0].audio.as_ref().unwrap().sr);
    let (start, length) = (rt.decks[0].loop_start / rate, rt.decks[0].loop_len / rate);
    for action in [
        midi::Action::DeckSavedLoopSave,
        midi::Action::DeckSavedLoopDelete,
    ] {
        let binding = Binding {
            action,
            extra: 4,
            ..bindings[0]
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
        input.push(&[0x92, 60, 100]);
        input.push(&[0x82, 60, 0]);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            rt.process(&mut []);
            let bank = rt.decks[0].preparation().unwrap().saved_loops;
            if if action == midi::Action::DeckSavedLoopDelete {
                bank.slots[4].is_none()
            } else {
                bank.slots[4].is_some_and(|slot| slot.start == start && slot.length == length)
            } {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    assert_eq!(
        rt.decks[0].preparation().unwrap().saved_loops.order,
        bank.order
    );
    let preset =
        midi::presets::Preset::capture("Saved loops".into(), String::new(), &port, &config)
            .unwrap();
    assert_eq!(preset.version, midi::presets::VERSION);
    let bytes = serde_json::to_vec(&preset).unwrap();
    assert_eq!(midi::presets::Preset::decode(&bytes).unwrap(), preset);
    let mut previous = serde_json::to_value(&preset).unwrap();
    previous["version"] = 4.into();
    let previous = midi::presets::Preset::decode(&serde_json::to_vec(&previous).unwrap()).unwrap();
    assert_eq!(previous.version, 4);
    assert_eq!(previous.bindings, preset.bindings);
    assert_eq!(previous.target(&port, &config).unwrap(), config);
    for version in [1, 2, 3] {
        let mut old = serde_json::to_value(&preset).unwrap();
        old["version"] = version.into();
        assert!(midi::presets::Preset::decode(&serde_json::to_vec(&old).unwrap()).is_err());
    }
    let mut preferences =
        crate::preferences::Preferences::defaults(std::path::Path::new("/home/fixture"));
    preferences.profiles.get_mut("Studio").unwrap().midi_learn = config;
    preferences.profiles.get_mut("Studio").unwrap().midi_presets = vec![preset];
    let (decoded, migrated) =
        crate::preferences::storage::decode(&serde_json::to_vec(&preferences).unwrap()).unwrap();
    assert!(!migrated);
    assert_eq!(decoded, preferences);
    use crate::preferences::{
        storage,
        worker::{Event, Job, Worker},
    };
    let directory = std::env::temp_dir().join(format!(
        "omatainer-saved-loop-settings-{}",
        std::process::id()
    ));
    std::fs::create_dir(&directory).unwrap();
    let settings_path = directory.join("preferences.json");
    let preset_path = directory.join("saved-loops.json");
    let mut worker = Worker::with_discovery(settings_path.clone(), || {
        Err("Discovery is deliberately absent from this software fixture".into())
    })
    .unwrap();
    let wait = |worker: &mut Worker| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(event) = worker.poll() {
                break event;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    };
    let portable = preferences.profiles["Studio"].midi_presets[0].clone();
    worker
        .request(Job::ExportMidiPreset {
            path: preset_path.clone(),
            preset: portable.clone(),
        })
        .unwrap();
    assert!(matches!(wait(&mut worker), Event::MidiPresetExported(_)));
    worker
        .request(Job::ImportMidiPreset {
            path: preset_path.clone(),
            token: 220,
        })
        .unwrap();
    assert!(
        matches!(wait(&mut worker),Event::MidiPresetImported {token:220,preset} if preset==portable)
    );
    storage::save(
        &settings_path,
        &preferences,
        storage::Overwrite::New,
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        storage::load(&settings_path, &AtomicBool::new(false))
            .unwrap()
            .preferences,
        preferences
    );
    drop(worker);
    std::fs::remove_dir_all(&directory).unwrap();
    let mut old = serde_json::to_value(&preferences).unwrap();
    old["version"] = 21.into();
    assert!(crate::preferences::storage::decode(&serde_json::to_vec(&old).unwrap()).is_err());
    for profile in old["profiles"].as_object_mut().unwrap().values_mut() {
        profile["midi_learn"]["mappings"] = serde_json::json!([]);
        profile["midi_presets"] = serde_json::json!([]);
    }
    let (decoded, migrated) =
        crate::preferences::storage::decode(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert!(migrated);
    assert_eq!(decoded.version, crate::preferences::VERSION);
    for action in [
        midi::Action::DeckSavedLoopSave,
        midi::Action::DeckSavedLoopDelete,
    ] {
        let good = Binding {
            action,
            ..bindings[0]
        };
        midi::learn::validate_binding(&good).unwrap();
        for bad in [
            Binding { extra: 8, ..good },
            Binding { deck: 2, ..good },
            Binding {
                kind: MsgKind::Cc,
                ..good
            },
        ] {
            assert!(midi::learn::validate_binding(&bad).is_err());
        }
    }
}
#[test]
fn saved_loop_activation_runs_at_the_real_quantized_output_sample_and_edits_cancel_stale_pending_work(
) {
    use crate::engine::beatgrid::Grid;
    for output_rate in [8_000, 44_100, 48_000] {
        for (initial, boundary) in [(0.125, 0.5), (2.25, 2.75), (5.25, 5.75)] {
            let (_, rt) = Engine::headless_for_test(output_rate, 256);
            let mut rt = Box::new(rt);
            fill(&mut rt);
            let source_rate = rt.decks[0].audio.as_ref().unwrap().sr;
            rt.decks[0].grid = Some(
                Grid::new(0.0, 120.0)
                    .unwrap()
                    .with_anchor(4.0, 2.0)
                    .unwrap()
                    .with_anchor(8.0, 5.0)
                    .unwrap(),
            );
            rt.decks[0].loop_on = false;
            rt.decks[0].pos = initial * f64::from(source_rate);
            rt.decks[0].playing = true;
            rt.apply(Command::DeckControl {
                source: 0,
                deck: 0,
                control: Control::Quantize {
                    enabled: true,
                    division: 3,
                },
            });
            let original = rt.decks[0].pos;
            let recall = command(&rt, 3, SavedLoopAction::Recall { activate: true });
            assert_eq!(test_alloc::measure(|| rt.apply(recall)), Default::default());
            let pending = rt.decks[0].controls.status().pending.unwrap();
            assert_eq!(
                pending.action,
                crate::engine::deck_controls::QuantizedAction::SavedLoop { id: 3 }
            );
            assert_eq!(pending.source_seconds, boundary);
            assert_eq!(rt.decks[0].pos, original);
            let frames = ((boundary - initial) * f64::from(output_rate)).ceil() as usize;
            let mut buffer = vec![0.0; (frames - 1) * 2];
            assert_eq!(
                test_alloc::measure(|| rt.process(&mut buffer)),
                Default::default()
            );
            assert!(rt.decks[0].controls.status().pending.is_some());
            assert!(!rt.decks[0].loop_on);
            assert_eq!(
                test_alloc::measure(|| rt.process(&mut [0.0; 2])),
                Default::default()
            );
            assert!(rt.decks[0].controls.status().pending.is_none());
            assert!(rt.decks[0].loop_on);
            assert_eq!(rt.decks[0].controls.status().loop_slot, 2);
            assert!((rt.decks[0].pos / f64::from(source_rate) - 1.0).abs() < 1e-9);
            rt.decks[0].loop_on = false;
            rt.decks[0].pos = 2.25 * f64::from(source_rate);
            rt.apply(command(&rt, 8, SavedLoopAction::Recall { activate: true }));
            assert!(rt.decks[0].controls.status().pending.is_some());
            rt.apply(command(
                &rt,
                8,
                SavedLoopAction::Style {
                    style: Style {
                        name: Name::new("Changed while queued").unwrap(),
                        color: None,
                    },
                },
            ));
            assert!(rt.decks[0].controls.status().pending.is_none());
            rt.apply(command(&rt, 8, SavedLoopAction::Recall { activate: true }));
            assert!(rt.decks[0].controls.status().pending.is_some());
            rt.apply(command(&rt, 8, SavedLoopAction::Delete));
            assert!(rt.decks[0].controls.status().pending.is_none());
        }
    }
}
#[test]
fn saved_start_end_and_anchor_spanning_regions_match_independent_stereo_playback_and_refuse_out_of_source_edits(
) {
    use crate::engine::{beatgrid::Grid, DeckTransition, Sample};
    use std::sync::Arc;
    for source_rate in [44_100, 48_000, 96_000] {
        for output_rate in [44_100, 48_000] {
            let audio = Arc::new(Sample {
                spectrum: None,
                name: "Saved loop stereo reference".into(),
                sr: source_rate,
                ch: 2,
                data: (0..source_rate * 4)
                    .flat_map(|frame| {
                        let time = f64::from(frame) / f64::from(source_rate);
                        [
                            (time * 317.0).sin() as f32 * 0.3,
                            (time * 491.0).cos() as f32 * 0.2,
                        ]
                    })
                    .collect(),
                peaks: Vec::new().into(),
                bpm: 120.0,
                path: String::new(),
            });
            for (id, start, length) in [(1, 0.0, 0.02), (2, 3.98, 0.02), (3, 1.9, 0.35)] {
                let (_, actual) = Engine::headless_for_test(output_rate, 256);
                let mut actual = Box::new(actual);
                let (_, reference) = Engine::headless_for_test(output_rate, 256);
                let mut reference = Box::new(reference);
                for rt in [&mut actual, &mut reference] {
                    rt.apply(Command::DeckAudio {
                        deck: 0,
                        audio: audio.clone(),
                    });
                    rt.decks[0].grid = Some(
                        Grid::new(0.0, 120.0)
                            .unwrap()
                            .with_anchor(4.0, 2.0)
                            .unwrap()
                            .with_anchor(6.0, 3.5)
                            .unwrap(),
                    );
                    rt.decks[0].pos = 0.125 * f64::from(source_rate);
                }
                actual.decks[0].loop_start = start * f64::from(source_rate);
                actual.decks[0].loop_len = length * f64::from(source_rate);
                actual.apply(command(&actual, id, SavedLoopAction::Save));
                let bank = actual.decks[0].preparation().unwrap().saved_loops;
                assert!(bank.slots[usize::from(id - 1)].is_some());
                actual.apply(command(
                    &actual,
                    id,
                    SavedLoopAction::Recall { activate: true },
                ));
                let d = &mut reference.decks[0];
                d.loop_start = start * f64::from(source_rate);
                d.loop_len = length * f64::from(source_rate);
                d.loop_on = true;
                d.transition_to(
                    start * f64::from(source_rate),
                    output_rate as f32,
                    DeckTransition::Jump,
                );
                actual.decks[0].playing = true;
                reference.decks[0].playing = true;
                let mut energy = [0.0f64; 2];
                assert_eq!(
                    test_alloc::measure(|| {
                        for _ in 0..(f64::from(output_rate) * (length * 2.0 + 0.01)).ceil() as usize
                        {
                            let (al, ar) = actual.render_deck(0);
                            let (bl, br) = reference.render_deck(0);
                            assert!((al - bl).abs() < 1e-6 && (ar - br).abs() < 1e-6);
                            energy[0] += f64::from(al).powi(2);
                            energy[1] += f64::from(ar).powi(2);
                        }
                    }),
                    Default::default()
                );
                assert!(energy.into_iter().all(|energy| energy > 0.001));
                assert!(
                    actual.decks[0].pos >= start * f64::from(source_rate)
                        && actual.decks[0].pos < (start + length) * f64::from(source_rate) + 1e-6
                );
                assert_eq!(actual.decks[0].preparation().unwrap().saved_loops, bank);
                actual.clear_undo_for_test();
                for (bad_start, bad_length) in [
                    (-1.0, 64.0),
                    (0.0, 63.0),
                    (audio.frames() as f64 - 64.0, 65.0),
                    (f64::NAN, 64.0),
                ] {
                    actual.decks[0].loop_start = bad_start;
                    actual.decks[0].loop_len = bad_length;
                    actual.apply(command(&actual, 8, SavedLoopAction::Save));
                    assert_eq!(actual.decks[0].controls.saved_loops(source_rate), bank);
                }
            }
        }
    }
}
#[test]
fn cue_loop_links_preserve_markers_slots_codec_and_one_undo_through_reorder_resize_and_delete_without_heap(
) {
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    fill(&mut rt);
    let rate = f64::from(rt.decks[0].audio.as_ref().unwrap().sr);
    rt.decks[0].hotcues[7] = crate::engine::HotCue {
        set: true,
        pos: rate * 0.123,
    };
    rt.decks[0].cue_styles[7] = Style {
        name: Name::new("Linked drop / 東京").unwrap(),
        color: Some([12, 34, 56]),
    };
    let before = rt.decks[0].preparation().unwrap();
    let position = rt.decks[0].pos;
    let other = rt.decks[1].preparation();
    rt.clear_undo_for_test();
    let link = command(&rt, 3, SavedLoopAction::Cue { pad: 7 });
    assert_eq!(test_alloc::measure(|| rt.apply(link)), Default::default());
    let linked = rt.decks[0].preparation().unwrap();
    assert_eq!(linked.saved_loops.cue_loops[7], Some(3));
    assert_eq!(
        linked.hotcues[7],
        Some(linked.saved_loops.slots[2].unwrap().start)
    );
    assert_eq!(linked.hotcue_styles[7], before.hotcue_styles[7]);
    assert_eq!(rt.decks[0].pos, position);
    assert!(!rt.decks[0].playing);
    let words = linked.saved_loops.words();
    assert_eq!(
        test_alloc::measure(|| assert_eq!(Bank::from_words(words), Some(linked.saved_loops))),
        Default::default()
    );
    let mut bad = words;
    bad[2] |= 1 << 40;
    assert!(Bank::from_words(bad).is_none());
    bad = words;
    bad[2] = 9;
    assert!(Bank::from_words(bad).is_none());
    assert_eq!(
        test_alloc::measure(|| rt.apply(Command::Undo)),
        Default::default()
    );
    assert_eq!(rt.decks[0].preparation().unwrap(), before);
    rt.apply(Command::Redo);
    assert_eq!(rt.decks[0].preparation().unwrap(), linked);
    rt.apply(command(&rt, 3, SavedLoopAction::Move { position: 0 }));
    assert_eq!(
        rt.decks[0].preparation().unwrap().saved_loops.cue_loops[7],
        Some(3)
    );
    rt.decks[0].loop_start = rate * 1.25;
    rt.decks[0].loop_len = rate * 0.5;
    rt.apply(command(&rt, 3, SavedLoopAction::Save));
    assert_eq!(rt.decks[0].hotcues[7].pos, rate * 1.25);
    rt.apply(command(&rt, 3, SavedLoopAction::Delete));
    assert!(rt.decks[0].preparation().unwrap().saved_loops.cue_loops[7].is_none());
    assert!(rt.decks[0].hotcues[7].set);
    assert_eq!(rt.decks[1].preparation(), other);
    assert!(engine.initial_playback[0]
        .as_ref()
        .unwrap()
        .preparation()
        .unwrap()
        .1
        .saved_loops
        .cue_loops[7]
        .is_none());
}

#[test]
fn linked_cue_and_region_start_at_one_quantized_output_sample_through_tempo_anchors_and_cue_only_override(
) {
    use crate::engine::beatgrid::Grid;
    use crate::engine::deck_controls::QuantizedAction;
    for output_rate in [8000, 44100, 48000] {
        for (initial, boundary) in [(0.125, 0.5), (2.25, 2.75), (5.25, 5.75)] {
            let (_, rt) = Engine::headless_for_test(output_rate, 256);
            let mut rt = Box::new(rt);
            fill(&mut rt);
            rt.apply(command(&rt, 3, SavedLoopAction::Cue { pad: 7 }));
            let source_rate = f64::from(rt.decks[0].audio.as_ref().unwrap().sr);
            rt.decks[0].grid = Some(
                Grid::new(0.0, 120.0)
                    .unwrap()
                    .with_anchor(4.0, 2.0)
                    .unwrap()
                    .with_anchor(8.0, 5.0)
                    .unwrap(),
            );
            rt.decks[0].loop_on = false;
            rt.decks[0].pos = initial * source_rate;
            rt.decks[0].playing = true;
            rt.apply(Command::DeckControl {
                source: 0,
                deck: 0,
                control: Control::Quantize {
                    enabled: true,
                    division: 3,
                },
            });
            assert_eq!(
                test_alloc::measure(|| rt.apply(Command::DeckHotCue {
                    deck: 0,
                    pad: 7,
                    del: false
                })),
                Default::default()
            );
            let pending = rt.decks[0].controls.status().pending.unwrap();
            assert_eq!(pending.action, QuantizedAction::CueLoop { pad: 7, id: 3 });
            assert_eq!(pending.source_seconds, boundary);
            assert_eq!(rt.decks[0].pos, initial * source_rate);
            let frames = ((boundary - initial) * f64::from(output_rate)).ceil() as usize;
            let mut buffer = vec![0.0; (frames - 1) * 2];
            assert_eq!(
                test_alloc::measure(|| rt.process(&mut buffer)),
                Default::default()
            );
            assert!(rt.decks[0].controls.status().pending.is_some());
            assert!(!rt.decks[0].loop_on);
            assert_eq!(
                test_alloc::measure(|| rt.process(&mut [0.0; 2])),
                Default::default()
            );
            assert!(rt.decks[0].controls.status().pending.is_none());
            assert!(rt.decks[0].loop_on);
            assert_eq!(rt.decks[0].controls.status().loop_slot, 2);
            assert!((rt.decks[0].pos / source_rate - 1.0).abs() < 1e-9);
            assert!((rt.decks[0].loop_start / source_rate - 1.0).abs() < 1e-9);
            assert!((rt.decks[0].loop_len / source_rate - 0.25).abs() < 1e-9);
            rt.apply(Command::DeckControl {
                source: 0,
                deck: 0,
                control: Control::Quantize {
                    enabled: false,
                    division: 3,
                },
            });
            let media_key = rt.decks[0].history_key;
            assert_eq!(
                test_alloc::measure(|| rt.apply(Command::DeckControl {
                    source: 0,
                    deck: 0,
                    control: Control::CueOnly { media_key, pad: 7 }
                })),
                Default::default()
            );
            assert!(!rt.decks[0].loop_on);
            assert_eq!(
                rt.decks[0].preparation().unwrap().saved_loops.cue_loops[7],
                Some(3)
            );
            rt.apply(Command::DeckHotCue {
                deck: 0,
                pad: 7,
                del: false,
            });
            assert!(rt.decks[0].loop_on);
        }
    }
}

#[test]
fn cue_loop_reassignment_deletion_release_and_source_replacement_cancel_deferred_owners_without_mutating_the_new_deck(
) {
    use crate::engine::deck_controls::Button;
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    fill(&mut rt);
    rt.apply(command(&rt, 3, SavedLoopAction::Cue { pad: 7 }));
    let rate = f64::from(rt.decks[0].audio.as_ref().unwrap().sr);
    rt.decks[0].loop_on = false;
    rt.decks[0].pos = rate * 0.125;
    rt.decks[0].playing = true;
    rt.apply(Command::DeckControl {
        source: 0,
        deck: 0,
        control: Control::Quantize {
            enabled: true,
            division: 3,
        },
    });
    rt.apply(Command::DeckControl {
        source: 900,
        deck: 0,
        control: Control::Hold {
            button: Button::HotCue(7),
            on: true,
        },
    });
    assert!(rt.decks[0].controls.status().pending.is_some());
    rt.apply(Command::DeckControl {
        source: 900,
        deck: 0,
        control: Control::Hold {
            button: Button::HotCue(7),
            on: false,
        },
    });
    assert!(rt.decks[0].controls.status().pending.is_none());
    assert!(!rt.decks[0].loop_on);
    rt.apply(Command::DeckHotCue {
        deck: 0,
        pad: 7,
        del: false,
    });
    assert!(rt.decks[0].controls.status().pending.is_some());
    rt.apply(command(&rt, 3, SavedLoopAction::UnlinkCue { pad: 7 }));
    assert!(rt.decks[0].controls.status().pending.is_none());
    rt.apply(command(&rt, 3, SavedLoopAction::Cue { pad: 7 }));
    rt.apply(Command::DeckHotCue {
        deck: 0,
        pad: 7,
        del: false,
    });
    rt.apply(Command::DeckHotCue {
        deck: 0,
        pad: 7,
        del: true,
    });
    assert!(rt.decks[0].controls.status().pending.is_none());
    assert!(rt.decks[0].preparation().unwrap().saved_loops.cue_loops[7].is_none());
    assert!(rt.decks[0].preparation().unwrap().saved_loops.slots[2].is_some());
    let stale = command(&rt, 3, SavedLoopAction::Cue { pad: 7 });
    let old_key = rt.decks[0].history_key;
    rt.decks[0].playing = false;
    rt.apply(Command::DeckLoadRequested {
        deck: 0,
        media: Media::Builtin(1),
        receipt: Receipt::new(),
    });
    let before = rt.decks[0].preparation();
    let checkpoint = engine.undo.checkpoint();
    rt.apply(stale);
    rt.apply(Command::DeckControl {
        source: 0,
        deck: 0,
        control: Control::CueOnly {
            media_key: old_key,
            pad: 7,
        },
    });
    assert_eq!(rt.decks[0].preparation(), before);
    assert_eq!(engine.undo.checkpoint(), checkpoint);
    assert!(!rt.decks[0].playing);
}

#[test]
fn cue_loop_native_project_reopens_exact_associations_stopped_and_refuses_legacy_or_dangling_links()
{
    let (engine, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    fill(&mut rt);
    for pad in 0..8 {
        rt.apply(command(&rt, pad + 1, SavedLoopAction::Cue { pad }));
    }
    let preparation = rt.decks[0].preparation().unwrap();
    let captured = capture(&engine, &mut rt);
    let path = std::env::temp_dir().join(format!(
        "omatainer-cue-loops-{}.omat",
        crate::sampler_bank::BankId::new().unwrap()
    ));
    let limits = crate::project_file::Limits::default();
    crate::project_file::save(
        &path,
        &crate::project_file::Bundle {
            state: captured.state.clone(),
            media: captured.media.clone(),
        },
        crate::project_file::Overwrite::Never,
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    let disk = crate::project_file::load::<crate::engine::project::State>(
        &path,
        &limits,
        &AtomicBool::new(false),
    )
    .unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        disk.state.decks[0].saved_loops.cue_loops,
        preparation.saved_loops.cue_loops
    );
    let mut reopened = Prepared::from_state(disk.state.clone(), disk.media.clone(), 44100).unwrap();
    assert!(!reopened.rt.playing && !reopened.rt.decks[0].playing);
    assert_eq!(reopened.rt.decks[0].preparation().unwrap(), preparation);
    reopened.rt.apply(Command::DeckHotCue {
        deck: 0,
        pad: 4,
        del: false,
    });
    assert!(reopened.rt.decks[0].playing && reopened.rt.decks[0].loop_on);
    let mut legacy = serde_json::to_value(&disk.state).unwrap();
    legacy["version"] = 26.into();
    assert!(serde_json::from_value::<crate::engine::project::State>(legacy.clone()).is_err());
    for deck in legacy["decks"].as_array_mut().unwrap() {
        if let Some(bank) = deck.get_mut("saved_loops").and_then(|b| b.as_object_mut()) {
            bank.remove("cue_loops");
        }
    }
    serde_json::from_value::<crate::engine::project::State>(legacy.clone())
        .unwrap()
        .validate(&disk.media)
        .unwrap();
    for injection in [
        serde_json::Value::Null,
        serde_json::json!([null, null, null, null, null, null, null, null]),
        serde_json::json!([1, 2, 3, 4, 5, 6, 7, 8]),
    ] {
        let mut old = legacy.clone();
        old["decks"][0]["saved_loops"]["cue_loops"] = injection;
        assert!(serde_json::from_value::<crate::engine::project::State>(old).is_err());
    }
    let mut bad = disk.state.clone();
    bad.decks[0].hotcues[4] = None;
    assert!(bad.validate(&disk.media).is_err());
}

#[test]
fn quantized_cue_only_override_uses_the_exact_off_grid_marker_and_leaves_the_saved_region_linked() {
    use crate::engine::deck_controls::QuantizedAction;
    let (_, rt) = Engine::headless_for_test(48000, 256);
    let mut rt = Box::new(rt);
    fill(&mut rt);
    let rate = f64::from(rt.decks[0].audio.as_ref().unwrap().sr);
    rt.decks[0].loop_start = rate * 0.333;
    rt.decks[0].loop_len = rate * 0.2;
    rt.apply(command(&rt, 3, SavedLoopAction::Save));
    rt.apply(command(&rt, 3, SavedLoopAction::Cue { pad: 7 }));
    rt.decks[0].grid = Some(crate::engine::beatgrid::Grid::new(0.0, 120.0).unwrap());
    rt.decks[0].loop_on = false;rt.decks[0].pos = rate * 0.125;rt.decks[0].playing = true;
    rt.apply(Command::DeckControl { source: 0, deck: 0, control: Control::Quantize { enabled: true, division: 3 } });
    rt.apply(Command::DeckControl { source: 0, deck: 0, control: Control::CueOnly { media_key: rt.decks[0].history_key, pad: 7 } });
    assert_eq!(rt.decks[0].controls.status().pending.unwrap().action, QuantizedAction::HotCueOnly { pad: 7 });
    let mut block = vec![0.0; 18000 * 2];
    assert_eq!(test_alloc::measure(|| rt.process(&mut block)), Default::default());
    assert!(rt.decks[0].controls.status().pending.is_none());
    assert!(!rt.decks[0].loop_on);
    assert!((rt.decks[0].pos / rate - 0.333).abs() < 1e-9);
    assert_eq!(rt.decks[0].preparation().unwrap().saved_loops.cue_loops[7], Some(3));
}
