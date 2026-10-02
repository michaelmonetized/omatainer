use super::*;
fn fixture() -> (Engine, RtEngine) {
    Engine::headless_for_test(48000, 256)
}
fn tick(rt: &mut RtEngine) {
    rt.process(&mut [0.0; 128]);
}
fn send(engine: &Engine, rt: &mut RtEngine, c: Command) {
    engine.send(c).unwrap();
    tick(rt);
}
fn note(pitch: u8) -> MidiNote {
    MidiNote {
        id: crate::engine::midi_edit::NoteId::new(), muted: false,
        pitch,
        start: 0.0,
        len: 8.0,
        vel: 100,
    }
}
fn notes(rt: &RtEngine, t: usize, s: usize) -> Vec<u8> {
    rt.tracks[t].clips[s]
        .notes
        .iter()
        .map(|n| n.pitch)
        .collect()
}

#[test]
fn grid_acknowledges_real_history_budget_rejection_without_callback_heap_work() {
    use crate::engine::beatgrid::{Grid, GridEditAck, GridEditState};
    let (engine, mut rt) = fixture();
    send(&engine, &mut rt, Command::SetNotes {
        track: 0, scene: 0, notes: vec![note(62)],
    });
    assert!(rt.undo.bytes > 2, "the fixture must own real history storage");
    rt.undo.budget = 1;
    let checkpoint = rt.undo.checkpoint();
    let history_len = rt.undo.entries.len();
    let before = rt.decks[0].grid;
    let ack = GridEditAck::new();
    engine.send(Command::DeckGrid {
        deck: 0, grid: Some(Grid::new(0.375, 127.0).unwrap()),
        receipt: engine.initial_playback[0].clone(), ack: ack.clone(),
    }).unwrap();
    assert_eq!(ack.state(), GridEditState::Pending);
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(rt.undo.failure, Some(Failure::Capacity));
    assert_eq!(rt.decks[0].grid, before);
    assert_eq!(rt.undo.checkpoint(), checkpoint);
    assert_eq!(rt.undo.entries.len(), history_len);
    assert_eq!(ack.state(), GridEditState::Rejected);
}

#[test]
fn renderer_controls_undo_and_redo_without_stopping_unrelated_live_owners() {
    let (engine, mut rt) = fixture();
    let initial = Global::get(&rt);
    rt.selected_track = 2;
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOn {
            source: 9,
            ch: 2,
            note: 67,
            vel: 90,
        },
    );
    let commands = [
        Command::SetBpm(141.0),
        Command::Xfader(0.2),
        Command::Master(0.6),
        Command::CueMix(0.8),
        Command::FxWet {
            slot: 2,
            value: 0.4,
        },
        Command::Quant(0.5),
        Command::Metronome,
        Command::SamplerInst(SamplerInstrument::Synth(SynthInstrument::Analog)),
    ];
    for command in commands {
        send(&engine, &mut rt, command);
    }
    let edited = Global::get(&rt);
    rt.playing = true;
    for _ in 0..8 {
        send(&engine, &mut rt, Command::Undo);
        assert!(rt.playing);
    }
    assert_eq!(Global::get(&rt), initial);
    assert!(rt.tracks[2].poly.voices.iter().any(|v| v.input
        == Some(InputKey::Midi {
            source: 9,
            ch: 2,
            note: 67
        })
        && v.env.stage != 4));
    for _ in 0..8 {
        send(&engine, &mut rt, Command::Redo);
    }
    assert_eq!(Global::get(&rt), edited);
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOff {
            source: 9,
            ch: 2,
            note: 67,
        },
    );
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .filter(|v| v.input
            == Some(InputKey::Midi {
                source: 9,
                ch: 2,
                note: 67
            }))
        .all(|v| v.env.stage == 4 || v.env.stage == 0));
}

#[test]
fn gestures_coalesce_keep_foreign_edits_separate_and_restore_saved_checkpoint() {
    let (engine, mut rt) = fixture();
    let clean = engine.undo.checkpoint();
    let id = engine.undo.gesture();
    for value in [0.2, 0.4, 0.6] {
        engine
            .send(Command::Gesture {
                id,
                command: Box::new(Command::Master(value)),
            })
            .unwrap();
    }
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(engine.undo.view().cursor, 1);
    assert_eq!(rt.master, 0.6);
    let middle = engine.undo.checkpoint();
    send(
        &engine,
        &mut rt,
        Command::Gesture {
            id,
            command: Box::new(Command::Master(0.8)),
        },
    );
    assert_ne!(engine.undo.checkpoint(), middle);
    assert_eq!(engine.undo.view().cursor, 1);
    send(&engine, &mut rt, Command::Undo);
    assert_eq!(engine.undo.checkpoint(), clean);
    send(&engine, &mut rt, Command::Redo);
    assert_eq!(rt.master, 0.8);
    send(
        &engine,
        &mut rt,
        Command::TrackGain {
            track: 1,
            value: 0.1,
        },
    );
    send(
        &engine,
        &mut rt,
        Command::Gesture {
            id,
            command: Box::new(Command::Master(0.9)),
        },
    );
    assert_eq!(engine.undo.view().cursor, 3);
    send(&engine, &mut rt, Command::Undo);
    assert_eq!(rt.master, 0.8);
    assert_eq!(rt.tracks[1].gain, 0.1);
    send(&engine, &mut rt, Command::Select { track: 4, scene: 7 });
    assert_ne!(engine.undo.checkpoint().untracked, clean.untracked);
}

#[test]
fn note_replacement_owns_original_and_preserves_live_notes_while_playing() {
    let (engine, mut rt) = fixture();
    let t = 2;
    let s = 7;
    rt.tracks[t].clips[s] = Clip {
        region: None,
        kind: ClipKind::Midi,
        name: "original".into(),
        bars: 2.0,
        notes: vec![note(60), note(64)],
        gain: 0.7,
        audio: None,
    };
    rt.apply(Command::LaunchClip {
        track: t as u8,
        scene: s as u8,
    });
    rt.apply(Command::LiveNoteOn {
        source: 44,
        ch: 0,
        note: 72,
        vel: 100,
    });
    send(
        &engine,
        &mut rt,
        Command::SetNotes {
            track: t as u8,
            scene: s as u8,
            notes: vec![note(61)],
        },
    );
    assert_eq!(notes(&rt, t, s), [61]);
    let counts = test_alloc::measure(|| {
        rt.apply(Command::Undo);
        rt.apply(Command::Redo);
        rt.apply(Command::Undo);
    });
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(notes(&rt, t, s), [60, 64]);
    assert_eq!(rt.tracks[t].clips[s].name, "original");
    assert_eq!(rt.tracks[t].clips[s].gain, 0.7);
    assert!(rt.tracks[t].poly.voices.iter().any(|v| v.input
        == Some(InputKey::Midi {
            source: 44,
            ch: 0,
            note: 72
        })
        && v.env.stage != 4));
    assert!(rt.playing);
    assert_eq!(rt.tracks[t].playing.unwrap().scene, s as u8);
}

#[test]
fn media_undo_retains_exact_sample_and_receipt_without_crediting_playback() {
    let (engine, mut rt) = fixture();
    let old = rt.decks[0].audio.clone().unwrap();
    let receipt = rt.decks[0].load_receipt.clone().unwrap();
    rt.decks[0].pos = 100.5;
    rt.decks[0].hotcues[4] = HotCue {
        set: true,
        pos: 88.25,
    };
    let next = rt.builtin[1].clone().unwrap();
    send(
        &engine,
        &mut rt,
        Command::DeckAudio {
            deck: 0,
            audio: next.clone(),
        },
    );
    assert!(receipt.retained_by_history());
    assert_eq!(receipt.state(), load_receipt::State::Superseded);
    let played = receipt.last_play();
    let counts = test_alloc::measure(|| rt.apply(Command::Undo));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &old));
    assert_eq!(rt.decks[0].pos, 100.5);
    assert_eq!(rt.decks[0].hotcues[4].pos, 88.25);
    assert_eq!(receipt.state(), load_receipt::State::Current);
    assert_eq!(receipt.last_play(), played);
    assert!(!rt.decks[0].playing);
    rt.apply(Command::Redo);
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &next));
}

#[test]
fn one_recording_take_groups_all_cells_and_undo_does_not_misdirect_releases() {
    let (engine, mut rt) = fixture();
    rt.playing = true;
    rt.recording = true;
    for t in 0..TRACKS {
        for s in 0..SCENES {
            rt.tracks[t].clips[s] = Clip::empty();
        }
    }
    for t in 0..TRACKS {
        for s in 0..SCENES {
            rt.selected_track = t;
            rt.selected_scene = s;
            rt.apply(Command::LiveNoteOn {
                source: 1,
                ch: 0,
                note: (t * 8 + s) as u8,
                vel: 100,
            });
        }
    }
    tick(&mut rt);
    assert_eq!(engine.undo.view().cursor, 1);
    assert_eq!(engine.undo.view().items[0].unwrap().patches, 64);
    rt.apply(Command::Undo);
    for track in &rt.tracks {
        for clip in &track.clips {
            assert_eq!(clip.kind, ClipKind::Empty);
            assert!(clip.notes.is_empty());
        }
    }
    // Off after undo cannot finalize a cancelled capture into another note list.
    for pitch in 0..64 {
        rt.apply(Command::LiveNoteOff {
            source: 1,
            ch: 0,
            note: pitch,
        });
    }
    rt.apply(Command::Redo);
    for track in &rt.tracks {
        for clip in &track.clips {
            assert_eq!(clip.kind, ClipKind::Midi);
            assert_eq!(clip.notes.len(), 1);
        }
    }
}

#[test]
fn oversized_payload_is_rejected_before_mutation_and_retired_off_callback() {
    let (engine, mut rt) = fixture();
    let original = notes(&rt, 0, 0);
    let mut many = Vec::with_capacity(NOTE_LIMIT + 1);
    many.resize(NOTE_LIMIT + 1, note(60));
    engine
        .send(Command::SetNotes {
            track: 0,
            scene: 0,
            notes: many,
        })
        .unwrap();
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(notes(&rt, 0, 0), original);
    assert_eq!(engine.undo.view().failure, Some(Failure::Notes));
    assert_eq!(engine.undo.view().cursor, 0);
    rt.undo.budget = 1;
    rt.selected_track = 2;
    rt.selected_scene = 7;
    rt.playing = true;
    rt.recording = true;
    rt.apply(Command::LiveNoteOn {
        source: 1,
        ch: 0,
        note: 60,
        vel: 90,
    });
    assert!(rt.tracks[2].clips[7].notes.is_empty());
    assert!(rt.tracks[2].poly.voices.iter().any(|v| v.input
        == Some(InputKey::Midi {
            source: 1,
            ch: 0,
            note: 60
        })));
    assert_eq!(rt.undo.failure, Some(Failure::Budget));
}

#[test]
fn effect_add_and_controls_replay_owned_processor_without_allocating() {
    let (engine, mut rt) = fixture();
    rt.fx_view = 2;
    send(&engine, &mut rt, Command::FxAdd(5));
    send(
        &engine,
        &mut rt,
        Command::FxParam {
            slot: 0,
            p: 1,
            value: 0.85,
        },
    );
    let old = rt.tracks[2].fx.slots[0].p[1];
    assert_eq!(old, 0.85);
    let counts = test_alloc::measure(|| {
        rt.apply(Command::Undo);
        rt.apply(Command::Undo);
        rt.apply(Command::Redo);
        rt.apply(Command::Redo);
    });
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(rt.tracks[2].fx.slots[0].p[1], old);
}

#[test]
fn real_midi_worker_and_ipc_cue_both_enter_renderer_history() {
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixStream;
    let (engine, mut rt) = fixture();
    let original = rt.tracks[3].gain;
    engine
        .midi
        .receive_for_test(&engine.cmd, 14, "APC40 mkII", &[0xb3, 7, 25]);
    tick(&mut rt);
    assert_eq!(rt.tracks[3].gain, 25.0 / 127.0);
    rt.apply(Command::Undo);
    assert_eq!(rt.tracks[3].gain, original);
    let (mut client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let cmd = engine.cmd.clone();
    let snap = engine.snap.clone();
    let worker = std::thread::spawn(move || crate::handle_client(server, cmd, snap));
    rt.decks[0].pos = 1123.75;
    rt.decks[0].cue_pos = 12.0;
    client
        .write_all(b"{\"op\":\"deckCue\",\"deck\":0}\n")
        .unwrap();
    let mut reply = String::new();
    std::io::BufReader::new(&client)
        .read_line(&mut reply)
        .unwrap();
    assert!(reply.contains("accepted"));
    tick(&mut rt);
    assert_eq!(rt.decks[0].cue_pos, 1123.75);
    rt.apply(Command::Undo);
    assert_eq!(rt.decks[0].cue_pos, 12.0);
    drop(client);
    worker.join().unwrap().unwrap();
}

#[test]
fn held_recording_changes_invalidate_saved_states_before_later_mixer_undo() {
    let (engine, mut rt) = fixture();
    rt.selected_track = 2;
    rt.selected_scene = 7;
    rt.recording = true;
    rt.playing = true;
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOn {
            source: 4,
            ch: 0,
            note: 62,
            vel: 100,
        },
    );
    let saved = engine.undo.checkpoint();
    send(&engine, &mut rt, Command::Master(0.4));
    tick(&mut rt);
    rt.apply(Command::Undo);
    assert_ne!(
        engine.undo.checkpoint(),
        saved,
        "the held note kept growing while the mixer edit was undone"
    );
    tick(&mut rt);
    rt.apply(Command::Redo);
    tick(&mut rt);
    rt.apply(Command::Undo);
    assert_ne!(
        engine.undo.checkpoint(),
        saved,
        "the retained redo branch must reflect the growing earlier take"
    );
    rt.apply(Command::Undo);
    assert_eq!(engine.undo.checkpoint().state, 0);
    assert!(rt.tracks[2].clips[7].notes.is_empty());
}

#[test]
fn full_history_wrap_keeps_retained_entries_in_place_and_exact_undo_redo_without_heap_work() {
    let (_engine, mut rt) = fixture();
    let capacity = rt.undo.entries.capacity();
    let value = |index: usize| 0.25 + index as f32 / 2048.0;
    let settle = |rt: &RtEngine| {
        let deadline = Instant::now() + Duration::from_secs(2);
        while rt.undo.preflight(0).is_err() {
            assert!(Instant::now() < deadline, "history retirement did not settle");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    let total = MAX_ENTRIES * 2 + 17;
    for index in 0..total {
        settle(&rt); // Worker progress is outside the renderer heap measurement.
        let retained = (index >= MAX_ENTRIES).then(|| {
            let entry = rt.undo.entries[64].as_ref().unwrap();
            (entry.id, entry as *const Entry)
        });
        let measured = test_alloc::measure(|| rt.apply(Command::Master(value(index))));
        assert_eq!(measured, test_alloc::Counts::default());
        assert_eq!(rt.undo.failures, 0);
        assert_eq!(rt.undo.entries.capacity(), capacity);
        if let Some((id, address)) = retained {
            let entry = rt.undo.entries[63].as_ref().unwrap();
            assert_eq!(entry.id, id);
            assert_eq!(entry as *const Entry, address,
                "eviction must not copy the retained inline transaction array");
        }
    }
    assert_eq!(rt.undo.entries.len(), MAX_ENTRIES);
    for index in (total - MAX_ENTRIES..total).rev() {
        let measured = test_alloc::measure(|| rt.apply(Command::Undo));
        assert_eq!(measured, test_alloc::Counts::default());
        assert_eq!(rt.master, value(index - 1));
    }
    assert_eq!(rt.undo.cursor, 0);
    for index in total - MAX_ENTRIES..total {
        let measured = test_alloc::measure(|| rt.apply(Command::Redo));
        assert_eq!(measured, test_alloc::Counts::default());
        assert_eq!(rt.master, value(index));
    }
    assert_eq!(rt.undo.cursor, MAX_ENTRIES);
    assert_eq!(rt.undo.failures, 0);
}

#[test]
fn oldest_whole_transactions_evict_redo_branches_retire_and_pcm_is_deduplicated() {
    let (engine, mut rt) = fixture();
    for i in 0..MAX_ENTRIES + 10 {
        rt.apply(Command::Master(i as f32 / 1000.0));
    }
    assert_eq!(rt.undo.entries.len(), MAX_ENTRIES);
    assert_eq!(rt.undo.cursor, MAX_ENTRIES);
    for _ in 0..2 {
        rt.apply(Command::Undo);
    }
    rt.apply(Command::Xfader(0.1));
    assert_eq!(rt.undo.entries.len(), MAX_ENTRIES - 1);
    let sample = rt.decks[0].audio.clone().unwrap();
    for _ in 0..8 {
        rt.apply(Command::DeckAudio {
            deck: 0,
            audio: sample.clone(),
        });
    }
    assert!(
        rt.undo.bytes < sample_bytes(&sample) + 9 * TEXT_LIMIT,
        "repeated refs must not multiply PCM charge: {}",
        rt.undo.bytes
    );
    tick(&mut rt);
    assert!(engine.undo.view().fixed_bytes > 0);
    eprintln!(
        "undo fixed storage {} bytes; live {} bytes",
        engine.undo.view().fixed_bytes,
        rt.undo.bytes
    );
}

#[test]
fn effect_gesture_targets_do_not_alias_across_large_racks_and_capacity_is_atomic() {
    let (engine, mut rt) = fixture();
    rt.tracks[0].fx.slots = (0..128)
        .map(|_| fx::FxSlot::new(fx::FxId::Balance, rt.sr))
        .collect();
    rt.tracks[1]
        .fx
        .slots
        .push(fx::FxSlot::new(fx::FxId::Balance, rt.sr));
    let id = engine.undo.gesture();
    rt.fx_view = 0;
    send(
        &engine,
        &mut rt,
        Command::Gesture {
            id,
            command: Box::new(Command::FxMix {
                slot: 64,
                value: 0.1,
            }),
        },
    );
    rt.fx_view = 1;
    send(
        &engine,
        &mut rt,
        Command::Gesture {
            id,
            command: Box::new(Command::FxMix {
                slot: 0,
                value: 0.8,
            }),
        },
    );
    rt.apply(Command::Undo);
    assert_eq!(rt.tracks[1].fx.slots[0].mix, 0.5);
    assert_eq!(rt.tracks[0].fx.slots[64].mix, 0.1);
    rt.apply(Command::Undo);
    assert_eq!(rt.tracks[0].fx.slots[64].mix, 0.5);
    rt.fx_view = 0;
    let before = rt.undo.checkpoint();
    rt.apply(Command::FxAdd(0));
    assert_eq!(rt.tracks[0].fx.slots.len(), 128);
    assert_eq!(rt.undo.checkpoint(), before);
    assert_eq!(rt.undo.failure, Some(Failure::Effects));
}

#[test]
fn reserved_processor_storage_matches_real_primitives_at_all_supported_rates() {
    for sr in [8000.0, 44100.0, 48000.0, 96000.0, 384000.0] {
        for &id in fx::FxId::all() {
            assert_eq!(
                fx::FxSlot::required_storage(id, sr),
                fx::FxSlot::new(id, sr).storage_bytes(),
                "{id:?} at {sr}"
            );
        }
    }
}

#[test]
fn active_recording_inverse_cannot_be_evicted_by_later_controls() {
    let (engine, mut rt) = fixture();
    rt.selected_track = 2;
    rt.selected_scene = 7;
    rt.playing = true;
    rt.recording = true;
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOn {
            source: 21,
            ch: 0,
            note: 60,
            vel: 100,
        },
    );
    for i in 1..MAX_ENTRIES {
        rt.apply(Command::Master(i as f32 / 1000.0));
    }
    let before = rt.master;
    let original = rt.undo.entries[0].as_ref().unwrap().id;
    rt.apply(Command::Master(1.4));
    assert_eq!(rt.master, before);
    assert_eq!(rt.undo.failure, Some(Failure::Capacity));
    assert_eq!(rt.undo.entries[0].as_ref().unwrap().id, original);
    rt.apply(Command::LiveNoteOff {
        source: 21,
        ch: 0,
        note: 60,
    });
    for _ in 0..MAX_ENTRIES {
        rt.apply(Command::Undo);
    }
    assert!(rt.tracks[2].clips[7].notes.is_empty());
}

#[test]
fn retire_worker_pressure_closes_creative_admission_but_keeps_owned_releases_safe() {
    let (engine, mut rt) = fixture();
    rt.selected_track = 2;
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOn {
            source: 31,
            ch: 0,
            note: 60,
            vel: 100,
        },
    );
    rt.undo.shared.worker_hold.store(true, Ordering::Release);
    // The held worker has bounded capacity. Every incoming boxed gesture is
    // retained in that capacity or its pre-reserved callback remainder.
    for i in 0..RETIRE_CAPACITY * 2 {
        if !rt.undo.available() {
            break;
        }
        let id = engine.undo.gesture();
        engine
            .send(Command::Gesture {
                id,
                command: Box::new(Command::Master((i % 100) as f32 / 100.0)),
            })
            .unwrap();
        tick(&mut rt);
    }
    assert!(!rt.undo.available());
    tick(&mut rt);
    assert_eq!(
        engine.send(Command::Master(1.0)),
        Err(SubmissionError::HistoryBusy)
    );
    engine
        .send(Command::LiveNoteOff {
            source: 31,
            ch: 0,
            note: 60,
        })
        .unwrap();
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(rt.tracks[2]
        .poly
        .voices
        .iter()
        .filter(|v| v.input
            == Some(InputKey::Midi {
                source: 31,
                ch: 0,
                note: 60
            }))
        .all(|v| v.env.stage == 0 || v.env.stage == 4));
    rt.undo.shared.worker_hold.store(false, Ordering::Release);
    let until = Instant::now() + Duration::from_secs(2);
    while !rt.undo.available() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    tick(&mut rt);
    engine.send(Command::Master(0.2)).unwrap();
}

#[test]
fn rejected_load_reports_unavailable_and_keeps_original_media_and_receipt() {
    let (engine, mut rt) = fixture();
    let old = rt.decks[0].audio.clone().unwrap();
    let receipt = rt.decks[0].load_receipt.clone().unwrap();
    rt.undo.budget = 1;
    let next = load_receipt::Receipt::new();
    engine
        .send(Command::DeckLoadRequested {
            deck: 0,
            media: load_receipt::Media::Builtin(1),
            receipt: next.clone(),
        })
        .unwrap();
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(next.state(), load_receipt::State::Unavailable);
    assert_eq!(receipt.state(), load_receipt::State::Current);
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &old));
    assert_eq!(rt.undo.cursor, 0);
}

#[test]
fn disconnected_retirement_keeps_last_owned_rejected_payload_and_closes_admission() {
    let (engine, mut rt) = fixture();
    let (dead, receiver) = bounded(1);
    drop(receiver);
    let live = rt.undo.retired.replace(dead);
    drop(live);
    let mut notes = Vec::with_capacity(NOTE_LIMIT + 1);
    notes.resize(NOTE_LIMIT + 1, note(60));
    engine
        .send(Command::SetNotes {
            track: 0,
            scene: 0,
            notes,
        })
        .unwrap();
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(rt.undo.stranded.len(), 1);
    assert_eq!(rt.undo.failure, Some(Failure::Unavailable));
    assert_eq!(
        engine.send(Command::Master(0.4)),
        Err(SubmissionError::HistoryBusy)
    );
    engine.send(Command::Stop).unwrap();
    tick(&mut rt);
    assert!(!rt.playing);
}

#[test]
fn failed_or_cancelled_project_install_preserves_history_success_clears_it() {
    use std::sync::atomic::AtomicBool;
    let (engine, mut rt) = fixture();
    send(&engine, &mut rt, Command::Master(0.4));
    let before = engine.undo.checkpoint();
    let prepared = project::Prepared::empty(48000).unwrap();
    let handle = engine.project.clone();
    let worker = std::thread::spawn(move || handle.install(prepared, 999, &AtomicBool::new(false)));
    let until = Instant::now() + Duration::from_secs(2);
    while !worker.is_finished() {
        assert!(Instant::now() < until);
        tick(&mut rt);
        std::thread::yield_now();
    }
    assert!(matches!(
        worker.join().unwrap(),
        Err(project::Error::Conflict)
    ));
    assert_eq!(engine.undo.checkpoint(), before);
    let prepared = project::Prepared::empty(48000).unwrap();
    assert!(matches!(
        engine
            .project
            .install(prepared, engine.project.revision(), &AtomicBool::new(true)),
        Err(project::Error::Cancelled)
    ));
    assert_eq!(engine.undo.checkpoint(), before);
    let prepared = project::Prepared::empty(48000).unwrap();
    let handle = engine.project.clone();
    let revision = handle.revision();
    let worker =
        std::thread::spawn(move || handle.install(prepared, revision, &AtomicBool::new(false)));
    let until = Instant::now() + Duration::from_secs(2);
    while !worker.is_finished() {
        assert!(Instant::now() < until);
        tick(&mut rt);
        std::thread::yield_now();
    }
    let applied = worker.join().unwrap().unwrap();
    assert_ne!(applied.checkpoint.epoch, before.epoch);
    assert_eq!(rt.undo.cursor, 0);
    assert_eq!(engine.undo.checkpoint(), applied.checkpoint);
    rt.apply(Command::Undo);
    assert_eq!(rt.master, 0.85);
}

#[test]
#[ignore = "comparative local timing evidence; not a physical audio XRUN test"]
fn dense_history_replay_benchmark() {
    let (_, mut rt) = fixture();
    for i in 0..MAX_ENTRIES {
        rt.apply(Command::Master((i % 100) as f32 / 100.0));
    }
    let begin = Instant::now();
    let counts = test_alloc::measure(|| {
        for _ in 0..100 {
            for _ in 0..MAX_ENTRIES {
                rt.apply(Command::Undo);
            }
            for _ in 0..MAX_ENTRIES {
                rt.apply(Command::Redo);
            }
        }
    });
    let elapsed = begin.elapsed();
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    eprintln!("{} populated-history replays in {:?} ({:.2} us/op); {} fixed bytes; no audio device involved",MAX_ENTRIES*200,elapsed,elapsed.as_secs_f64()*1e6/(MAX_ENTRIES*200) as f64,rt.undo.fixed_bytes());
}

#[test]
fn undo_while_note_is_held_preserves_elapsed_duration_in_redo_and_live_gate() {
    let (engine, mut rt) = fixture();
    rt.selected_track = 2;
    rt.selected_scene = 7;
    rt.playing = true;
    rt.recording = true;
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOn {
            source: 5,
            ch: 0,
            note: 66,
            vel: 100,
        },
    );
    let mut out = vec![0.0; 48000 * 2];
    rt.process(&mut out);
    let expected = rt.note_recording.clock;
    let counts = test_alloc::measure(|| rt.apply(Command::Undo));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(rt.tracks[2].clips[7].notes.is_empty());
    assert!(rt.tracks[2].poly.voices.iter().any(|v| v.input
        == Some(InputKey::Midi {
            source: 5,
            ch: 0,
            note: 66
        })
        && v.env.stage != 4));
    rt.apply(Command::Redo);
    let len = rt.tracks[2].clips[7].notes[0].len;
    assert!((len as f64 - expected).abs() < 1e-5, "{len} vs {expected}");
    assert!(len > 1.0);
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOff {
            source: 5,
            ch: 0,
            note: 66,
        },
    );
    assert_eq!(rt.tracks[2].clips[7].notes[0].len, len);
}

#[test]
fn stopped_sample_rate_preparation_updates_retained_processors_before_redo() {
    let (engine, mut rt) = fixture();
    rt.fx_view = 2;
    send(&engine, &mut rt, Command::FxAdd(5));
    rt.apply(Command::Undo);
    rt.set_sample_rate(96000).unwrap();
    let counts = test_alloc::measure(|| rt.apply(Command::Redo));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(
        rt.tracks[2].fx.slots[0].storage_bytes(),
        fx::FxSlot::required_storage(fx::FxId::Delay, 96000.0)
    );
}

#[test]
fn repeated_same_media_does_not_deadlock_retirement_or_project_clear() {
    let (engine, mut rt) = fixture();
    let sample = rt.decks[0].audio.clone().unwrap();
    for i in 0..400 {
        rt.apply(Command::DeckAudio {
            deck: 0,
            audio: sample.clone(),
        });
        if i % 16 == 0 {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    assert_ne!(rt.undo.failure, Some(Failure::Budget));
    let until = Instant::now() + Duration::from_secs(2);
    while rt.undo.preflight(0).is_err() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    rt.clear_undo_for_test();
    assert!(rt.undo.assets.is_empty());
    tick(&mut rt);
    assert_eq!(engine.undo.view().cursor, 0);
}

#[test]
fn rate_budget_pruning_preserves_only_current_holds_and_never_claims_saved_content() {
    let (engine, mut rt) = fixture();
    let saved = engine.undo.checkpoint();
    rt.selected_track = 2;
    rt.selected_scene = 6;
    rt.playing = true;
    rt.recording = true;
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOn {
            source: 7,
            ch: 0,
            note: 70,
            vel: 100,
        },
    );
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOff {
            source: 7,
            ch: 0,
            note: 70,
        },
    );
    rt.selected_scene = 7;
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOn {
            source: 7,
            ch: 0,
            note: 69,
            vel: 100,
        },
    );
    rt.fx_view = 2;
    send(&engine, &mut rt, Command::FxAdd(5));
    send(&engine, &mut rt, Command::Master(0.7));
    let original_audio = rt.decks[0].audio.clone().unwrap();
    rt.undo.budget = 1100 * 1024;
    rt.set_sample_rate(96000).unwrap();
    assert_eq!(rt.undo.failure, Some(Failure::RateHistoryPruned));
    assert_eq!(rt.undo.cursor, 1);
    assert_eq!(
        rt.undo.entries[0].as_ref().unwrap().len,
        1,
        "released targets are not retained by the rate-budget exception"
    );
    assert!(rt.undo.bytes <= rt.undo.budget);
    assert_ne!(engine.undo.checkpoint().epoch, saved.epoch);
    assert_eq!(rt.master, 0.7);
    assert_eq!(rt.tracks[2].clips[7].notes.len(), 1);
    assert!(Arc::ptr_eq(
        rt.decks[0].audio.as_ref().unwrap(),
        &original_audio
    ));
    let mut out = vec![0.0; 96000];
    rt.process(&mut out);
    rt.apply(Command::LiveNoteOff {
        source: 7,
        ch: 0,
        note: 69,
    });
    assert!(rt.tracks[2].clips[7].notes[0].len > 0.5);
    rt.apply(Command::Undo);
    assert!(rt.tracks[2].clips[7].notes.is_empty());
    assert_eq!(
        rt.tracks[2].clips[6].notes.len(),
        1,
        "undo preserves the released target whose inverse was trimmed"
    );
    assert_eq!(rt.master, 0.7);
    assert_ne!(engine.undo.checkpoint(), saved);
}

#[test]
fn rejected_last_sample_owner_is_retired_by_worker_not_renderer() {
    let (engine, mut rt) = fixture();
    rt.undo.budget = 1;
    let sample = Arc::new(Sample {
        name: "owned rejection".into(),
        path: "private fixture".into(),
        sr: 48000,
        ch: 2,
        data: vec![0.25; 65536],
        peaks: Arc::new(Vec::new()),
        bpm: 120.0,
    });
    let weak = Arc::downgrade(&sample);
    engine
        .send(Command::DeckAudio {
            deck: 0,
            audio: sample,
        })
        .unwrap();
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    let until = Instant::now() + Duration::from_secs(2);
    while weak.strong_count() != 0 {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(rt.undo.cursor, 0);
}

#[test]
fn fallback_effect_targets_are_undoable_and_audio_clip_inputs_only_monitor() {
    let (engine, mut rt) = fixture();
    for view in [-1, 99, 500] {
        rt.fx_view = view;
        let rack = Rack::selected(&rt).unwrap();
        let before = rack.get(&rt).slots.len();
        send(&engine, &mut rt, Command::FxAdd(0));
        assert_eq!(rack.get(&rt).slots.len(), before + 1);
        send(&engine, &mut rt, Command::Undo);
        assert_eq!(rack.get(&rt).slots.len(), before);
        send(&engine, &mut rt, Command::Redo);
        assert_eq!(rack.get(&rt).slots.len(), before + 1);
    }
    rt.clear_undo_for_test();
    rt.selected_track = 2;
    rt.selected_scene = 7;
    rt.tracks[2].clips[7].kind = ClipKind::Audio;
    rt.playing = true;
    rt.recording = true;
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOn {
            source: 17,
            ch: 0,
            note: 67,
            vel: 90,
        },
    );
    assert_eq!(rt.undo.cursor, 0);
    assert!(rt.tracks[2].clips[7].notes.is_empty());
    assert!(rt.tracks[2].poly.voices.iter().any(|voice| voice.input
        == Some(InputKey::Midi {
            source: 17,
            ch: 0,
            note: 67,
        })
        && voice.env.stage != 4));
    send(
        &engine,
        &mut rt,
        Command::LiveNoteOff {
            source: 17,
            ch: 0,
            note: 67,
        },
    );
    assert_eq!(rt.note_recording.held_targets(), 0);
}

#[test]
fn all_owned_request_early_exits_retire_last_payloads_off_renderer() {
    use std::cell::RefCell;
    thread_local! {
        // A zero-capture hook exercises the actual cancellation/claim boundary
        // without a heap-allocated test closure contaminating the measurement.
        static CANCEL_AT_CLAIM: RefCell<Option<load_receipt::Receipt>> = const { RefCell::new(None) };
    }
    let (engine, mut rt) = fixture();
    let original = rt.decks[0].audio.clone().unwrap();
    let loader = media_load::Loader::start().unwrap();
    let mut weak_samples = Vec::new();
    for case in 0..7 {
        let mut token = loader
            .request(0, "/nonexistent/omatainer-undo-private-fixture.wav".into())
            .unwrap();
        let audio = Arc::new(Sample {
            name: "last-owned request".into(),
            path: "private fixture".into(),
            sr: 48000,
            ch: 2,
            data: vec![0.25; 65536],
            peaks: Arc::new(vec![[0.0; 3]; 8]),
            bpm: 120.0,
        });
        weak_samples.push(Arc::downgrade(&audio));
        let receipt = load_receipt::Receipt::new();
        let command = match case {
            0 => {
                loader.invalidate(0).unwrap();
                Command::DeckDecoded {
                    request: token,
                    audio,
                }
            }
            1 => {
                token.deck = 255;
                Command::DeckDecoded {
                    request: token,
                    audio,
                }
            }
            2 => {
                receipt.cancel_pending();
                Command::DeckLoadRequested {
                    deck: 0,
                    media: load_receipt::Media::Decoded { token, audio },
                    receipt,
                }
            }
            3 => Command::DeckLoadRequested {
                deck: 255,
                media: load_receipt::Media::Decoded { token, audio },
                receipt,
            },
            4 => Command::DeckLoadRequested {
                deck: 1,
                media: load_receipt::Media::Decoded { token, audio },
                receipt,
            },
            5 => {
                loader.invalidate(0).unwrap();
                Command::DeckLoadRequested {
                    deck: 0,
                    media: load_receipt::Media::Decoded { token, audio },
                    receipt,
                }
            }
            6 => {
                CANCEL_AT_CLAIM.with(|slot| *slot.borrow_mut() = Some(receipt.clone()));
                rt.load_test_hooks[0] = Some(Box::new(|| {
                    CANCEL_AT_CLAIM.with(|slot| slot.borrow().as_ref().unwrap().cancel_pending())
                }));
                Command::DeckLoadRequested {
                    deck: 0,
                    media: load_receipt::Media::Decoded { token, audio },
                    receipt,
                }
            }
            _ => unreachable!(),
        };
        engine.send(command).unwrap();
        let counts = test_alloc::measure(|| tick(&mut rt));
        assert_eq!((counts.allocations, counts.frees), (0, 0), "case {case}");
        CANCEL_AT_CLAIM.with(|slot| *slot.borrow_mut() = None);
        assert_eq!(rt.undo.cursor, 0);
        assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &original));
    }
    for command in [
        Command::DeckLoadRequested {
            deck: 0,
            media: load_receipt::Media::Builtin(255),
            receipt: load_receipt::Receipt::new(),
        },
        Command::LearnCapture {
            param: "owned ignored mapping".repeat(100),
            ch: 0,
            d1: 1,
            d2: 2,
            status: 0xb0,
        },
        Command::SetNotes {
            track: 255,
            scene: 0,
            notes: vec![note(60); 256],
        },
        Command::SetNotes {
            track: 0,
            scene: 255,
            notes: vec![note(60); 256],
        },
    ] {
        engine.send(command).unwrap();
    }
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    let until = Instant::now() + Duration::from_secs(2);
    while weak_samples.iter().any(|sample| sample.strong_count() != 0) {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(rt.undo.cursor, 0);
}

#[test]
fn accepted_media_receipt_follows_mutation_if_retirement_disconnects_after_preflight() {
    let (_engine, mut rt) = fixture();
    let audio = rt.builtin[1].clone().unwrap();
    let receipt = load_receipt::Receipt::new();
    // Simulate the receiver disappearing after its last published availability
    // check. The capture still has its pre-reserved owned remainder.
    let (sender, receiver) = bounded(RETIRE_CAPACITY);
    drop(receiver);
    // Keep the original worker connected so it cannot race this deliberately
    // stale publication by replacing shared.connected with false.
    let _original_sender = rt.undo.retired.replace(sender);
    let command = Command::DeckLoadRequested {
        deck: 0,
        media: load_receipt::Media::Builtin(1),
        receipt: receipt.clone(),
    };
    let counts = test_alloc::measure(|| rt.apply(command));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(receipt.state(), load_receipt::State::Current);
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &audio));
    assert_eq!(rt.undo.cursor, 1);
    assert_eq!(rt.undo.failure, Some(Failure::Unavailable));
    assert!(!rt.undo.available());
    assert_eq!(rt.undo.stranded.len(), 2);
}

fn begin_owned_history_take(rt: &mut RtEngine) {
    rt.selected_track = 2;
    rt.selected_scene = 7;
    rt.recording = true;
    rt.playing = true;
    assert!(rt.tracks[2].clips[7].notes.is_empty());
}
fn owned_history_on(source: u64, note: u8) -> Command {
    Command::LiveNoteOn { source, ch: 0, note, vel: 100 }
}
fn owned_history_off(source: u64, note: u8) -> Command {
    Command::LiveNoteOff { source, ch: 0, note }
}
fn owned_history_id(rt: &RtEngine) -> u64 {
    rt.undo.entries.back().unwrap().as_ref().unwrap().id
}

#[test]
fn independent_short_holds_do_not_pin_prior_recording_inverses_during_dense_controls() {
    let (engine, mut rt) = fixture();
    begin_owned_history_take(&mut rt);
    let mut total_allocations = 0;
    let mut total_frees = 0;
    for cycle in 0..64 {
        // Waiting for the real retirement/replenishment worker is outside the
        // renderer measurement. It must not disguise active-owner pinning.
        rt.refresh_history_protection();
        let deadline = Instant::now() + Duration::from_secs(2);
        while rt.undo.preflight(0).is_err() || rt.undo.scratch.as_ref().unwrap().len() < 2 {
            assert!(Instant::now() < deadline, "history worker failed to settle");
            std::thread::sleep(Duration::from_millis(1));
        }
        for on in [true, false] {
            for track in 0..TRACKS {
                engine.send(Command::TrackGain { track: track as u8, value: 0.2 + cycle as f32 / 100.0 }).unwrap();
            }
            engine.send(if on { owned_history_on(901, 60) } else { owned_history_off(901, 60) }).unwrap();
            let counts = test_alloc::measure(|| tick(&mut rt));
            total_allocations += counts.allocations;
            total_frees += counts.frees;
            assert_eq!(rt.undo.failures, 0, "cycle {cycle}, on={on}, {:?}", rt.undo.failure);
            assert_eq!(engine.cmd.len(), 0);
        }
        std::thread::yield_now();
    }
    assert_eq!((total_allocations, total_frees), (0, 0));
    assert_eq!(rt.tracks[2].clips[7].notes.len(), 64);
    assert_eq!(rt.undo.entries.len(), MAX_ENTRIES);
    assert_eq!(rt.note_recording.held_targets(), 0);
    for track in &rt.tracks { assert_eq!(track.gain, 0.83); }
}

#[test]
fn overlapping_same_cell_owners_finalize_later_inverses_and_release_protection_independently() {
    let (_engine, mut rt) = fixture();
    begin_owned_history_take(&mut rt);
    rt.apply(owned_history_on(1, 60));
    let first = owned_history_id(&rt);
    rt.note_recording.clock += 0.4;
    rt.apply(Command::Master(0.5));
    rt.apply(owned_history_on(2, 64));
    let second = owned_history_id(&rt);
    assert_ne!(first, second);
    rt.refresh_history_protection();
    assert_eq!(rt.undo.protected, Some(first));
    rt.note_recording.clock += 0.8;
    rt.apply(owned_history_off(1, 60));
    let first_duration = rt.tracks[2].clips[7].notes[0].len;
    assert!((first_duration - 1.2).abs() < 1e-6);
    rt.refresh_history_protection();
    assert_eq!(rt.undo.protected, Some(second), "a released older note no longer pins its inverse");
    rt.note_recording.clock += 0.7;
    let counts = test_alloc::measure(|| rt.apply(Command::Undo));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(notes(&rt, 2, 7), [60]);
    assert_eq!(rt.tracks[2].clips[7].notes[0].len, first_duration, "later inverse must not restore old onset preview");
    assert_eq!(rt.note_recording.held_targets(), 0);
    rt.apply(owned_history_off(2, 64));
    rt.apply(Command::Redo);
    assert_eq!(notes(&rt, 2, 7), [60, 64]);
    assert_eq!(rt.tracks[2].clips[7].notes[0].len, first_duration);
    assert!((rt.tracks[2].clips[7].notes[1].len - 1.5).abs() < 1e-6);
}

#[test]
fn undo_of_later_inverse_finalizes_both_still_held_sources_before_swap() {
    let (_engine, mut rt) = fixture();
    begin_owned_history_take(&mut rt);
    rt.apply(owned_history_on(1, 60));
    rt.note_recording.clock += 0.4;
    rt.apply(Command::Master(0.5));
    rt.apply(owned_history_on(2, 60));
    rt.note_recording.clock += 0.8;
    let counts = test_alloc::measure(|| rt.apply(Command::Undo));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(rt.tracks[2].clips[7].notes.len(), 1);
    assert!((rt.tracks[2].clips[7].notes[0].len - 1.2).abs() < 1e-6);
    assert_eq!(rt.note_recording.held_targets(), 0);
    rt.note_recording.clock += 2.0;
    rt.apply(owned_history_off(1, 60));
    rt.apply(owned_history_off(2, 60));
    rt.apply(Command::Redo);
    assert_eq!(rt.tracks[2].clips[7].notes.len(), 2);
    assert!((rt.tracks[2].clips[7].notes[0].len - 1.2).abs() < 1e-6);
    assert!((rt.tracks[2].clips[7].notes[1].len - 0.8).abs() < 1e-6);
    for _ in 0..3 { rt.apply(Command::Undo); }
    assert!(rt.tracks[2].clips[7].notes.is_empty());
    for _ in 0..3 { rt.apply(Command::Redo); }
    assert!((rt.tracks[2].clips[7].notes[0].len - 1.2).abs() < 1e-6);
}

#[test]
fn rate_pruning_preserves_distinct_held_inverse_owners_in_the_same_cell() {
    let (engine, mut rt) = fixture();
    begin_owned_history_take(&mut rt);
    rt.apply(owned_history_on(1, 60));
    let first = owned_history_id(&rt);
    rt.note_recording.clock += 0.4;
    rt.apply(Command::Master(0.5));
    rt.apply(owned_history_on(2, 64));
    let second = owned_history_id(&rt);
    rt.fx_view = 2;
    rt.apply(Command::FxAdd(5));
    let before = engine.undo.checkpoint();
    rt.undo.budget = 1100 * 1024;
    rt.set_sample_rate(96000).unwrap();
    assert_eq!(rt.undo.failure, Some(Failure::RateHistoryPruned));
    assert_eq!(rt.undo.cursor, 2, "both actual held inverses survive, even with the same cell");
    assert_eq!(rt.undo.entries[0].as_ref().unwrap().id, first);
    assert_eq!(rt.undo.entries[1].as_ref().unwrap().id, second);
    assert_ne!(engine.undo.checkpoint().epoch, before.epoch);
    rt.note_recording.clock += 0.8;
    rt.apply(owned_history_off(1, 60));
    rt.refresh_history_protection();
    assert_eq!(rt.undo.protected, Some(second));
    rt.note_recording.clock += 0.7;
    rt.apply(owned_history_off(2, 64));
    for _ in 0..2 { rt.apply(Command::Undo); }
    assert!(rt.tracks[2].clips[7].notes.is_empty());
    for _ in 0..2 { rt.apply(Command::Redo); }
    assert_eq!(notes(&rt, 2, 7), [60, 64]);
    assert!((rt.tracks[2].clips[7].notes[0].len - 1.2).abs() < 1e-6);
    assert!((rt.tracks[2].clips[7].notes[1].len - 1.5).abs() < 1e-6);
}
