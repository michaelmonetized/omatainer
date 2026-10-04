use super::*;
use crate::engine::{
    dsp::{InputKey, Sample},
    load_receipt, test_alloc, Engine, MidiNote, RtEngine,
};
fn fixture() -> (Engine, RtEngine) {
    Engine::headless_for_test(48_000, 256)
}
fn tick(rt: &mut RtEngine) {
    rt.process(&mut [0.0; 128]);
}
#[test]
fn protected_loads_are_checked_again_on_renderer_without_dropping_payloads() {
    let (engine, mut rt) = fixture();
    let original = rt.decks[0].audio.clone().unwrap();
    let audio = Arc::new(Sample {
        name: "queued replacement".into(),
        path: String::new(),
        sr: 48_000,
        ch: 2,
        data: vec![0.5; 8192],
        peaks: Arc::new(Vec::new()),
        bpm: 120.0,
    });
    let weak = Arc::downgrade(&audio);
    // Accepted before protection, but the preceding queued Play makes it unsafe
    // by the time the renderer reaches the replacement.
    engine.send(Command::DeckPlay { deck: 0 }).unwrap();
    engine.send(Command::DeckAudio { deck: 0, audio }).unwrap();
    engine.send(Command::PerformanceMode(true)).unwrap();
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &original));
    assert!(matches!(
        engine.send(Command::LoadBuiltin { deck: 0, stem: 1 }),
        Err(crate::engine::SubmissionError::Performance(
            Error::PlayingDeck
        ))
    ));
    assert!(matches!(
        engine.send(Command::SetNotes {
            track: 2,
            scene: 0,
            notes: Vec::new()
        }),
        Err(crate::engine::SubmissionError::Performance(
            Error::Protected
        ))
    ));
    engine.send(Command::Master(0.7)).unwrap();
    engine.send(Command::DeckPlay { deck: 0 }).unwrap();
    tick(&mut rt);
    assert!(matches!(engine.send(Command::LoadBuiltin { deck: 0, stem: 1 }), Err(crate::engine::SubmissionError::Performance(Error::PlayingDeck))));
    tick(&mut rt);
    let receipt = load_receipt::Receipt::new();
    engine
        .send(Command::DeckLoadRequested {
            deck: 0,
            media: load_receipt::Media::Builtin(1),
            receipt: receipt.clone(),
        })
        .unwrap();
    tick(&mut rt);
    assert_eq!(receipt.state(), load_receipt::State::Current);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while weak.strong_count() != 0 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}
#[test]
fn safe_stop_finalizes_capture_releases_all_owners_and_rejects_queued_onsets() {
    let (engine, mut rt) = fixture();
    rt.tracks[2].clips[7].notes = vec![MidiNote {
        channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
        pitch: 50,
        start: 0.0,
        len: 0.5,
        vel: 80,
    }];
    engine
        .send(Command::ComposeArm { track: 2, scene: 7 })
        .unwrap();
    engine.send(Command::Play).unwrap();
    engine.send(Command::Record).unwrap();
    engine
        .send(Command::LiveNoteOn {
            source: 2,
            ch: 1,
            note: 63,
            vel: 90,
        })
        .unwrap();
    tick(&mut rt);
    rt.process(&mut [0.0; 4800]);
    assert!(rt.has_held_project_notes());
    for source in 10..100 {
        engine
            .send(Command::LiveNoteOn {
                source,
                ch: 0,
                note: 68,
                vel: 100,
            })
            .unwrap();
    }
    engine.send(Command::SafetyStop(Safety::Stop)).unwrap();
    let counts = test_alloc::measure(|| tick(&mut rt));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(!rt.playing && !rt.recording && !rt.has_held_project_notes());
    assert!(rt.tracks.iter().all(|track| track
        .poly
        .voices
        .iter()
        .all(|voice| !matches!(voice.env.stage, 1..=3))));
    assert!(rt.tracks[2].clips[7].notes.last().unwrap().len > 0.09);
    assert!(!engine.cmd.performance().status().output_muted);
    engine.send(Command::RecoverPerformance).unwrap();
    tick(&mut rt);
    assert!(
        engine.cmd.performance().status().recovery,
        "backlog must drain before reopening"
    );
    tick(&mut rt);
    assert!(!engine.cmd.performance().status().recovery);
    assert!(!rt.playing && rt.decks.iter().all(|d| !d.playing));
    engine
        .send(Command::LiveNoteOn {
            source: 2,
            ch: 1,
            note: 63,
            vel: 90,
        })
        .unwrap();
    tick(&mut rt);
    assert!(rt.tracks[2].poly.voices.iter().any(|v| v.input
        == Some(InputKey::Midi {
            source: 2,
            ch: 1,
            note: 63
        })
        && matches!(v.env.stage, 1..=3)));
}
#[test]
fn emergency_output_ramps_then_stays_muted_after_explicit_ack_and_quiet_observation() {
    let (engine, mut rt) = fixture();
    rt.decks[0].playing = true;
    tick(&mut rt);
    engine.send(Command::SafetyStop(Safety::Silence)).unwrap();
    let mut block = [0.0; 256];
    rt.process(&mut block);
    assert!(block[192..].iter().all(|sample| *sample == 0.0));
    engine.send(Command::RecoverPerformance).unwrap();
    tick(&mut rt);
    assert!(!engine.cmd.performance().status().recovery);
    assert!(engine.cmd.performance().status().output_muted);
    // Even after the bounded observation completes, mute cannot self-clear.
    for _ in 0..760 {
        rt.process(&mut block);
    }
    let status = engine.cmd.performance().status();
    assert!(status.observation_complete && status.output_muted);
    assert!(block.iter().all(|sample| *sample == 0.0));
    let mut output = Output::default();
    output.stop(Safety::Silence, 48_000.0);
    output.observe([f32::NAN, 0.0], 48_000.0);
    assert!(output.nonfinite && output.quiet == 0);
}
#[test]
fn exclusive_jobs_and_optional_cancellation_close_mode_entry_races() {
    let handle = Handle::default();
    let permit = handle.audio_change().unwrap();
    assert!(permit.valid_for_audio(&handle));
    assert_eq!(handle.set_enabled(true), Err(Error::Changing));
    drop(permit);
    let work = handle.optional_work().unwrap();
    let cancel = work.cancel();
    handle.set_enabled(true).unwrap();
    assert!(cancel.load(Ordering::Acquire));
    assert!(matches!(handle.optional_work(), Err(Error::Protected)));
    assert!(matches!(handle.project_change(), Err(Error::Protected)));
    assert!(matches!(handle.audio_change(), Err(Error::Protected)));
    handle.request_safety(Safety::Stop);
    let request = handle.safety_request().unwrap().0;
    handle.stopped(request);
    let permit = handle.audio_change().unwrap();
    assert_eq!(handle.acknowledge_inputs_released(), Err(Error::Changing));
    drop(permit);
    handle.acknowledge_inputs_released().unwrap();
    let writer = handle.writer();
    assert!(!handle.try_recover(|| true));
    drop(writer);
    assert!(handle.try_recover(|| true));
    assert!(handle.status().protected);
}

#[test]
fn safety_mailbox_bypasses_full_owned_queue_and_retires_rejected_payloads_off_audio() {
    let (engine, mut rt) = fixture();
    let original = rt.decks[0].audio.clone().unwrap();
    let replacement = Arc::new(Sample {
        name: "rejected owned media".into(),
        path: String::new(),
        sr: 48_000,
        ch: 2,
        data: vec![0.5; 65_536],
        peaks: Arc::new(vec![[0.5; 3]; 512]),
        bpm: 120.0,
    });
    let weak = Arc::downgrade(&replacement);
    engine
        .send(Command::DeckAudio {
            deck: 0,
            audio: replacement,
        })
        .unwrap();
    engine
        .send(Command::SetNotes {
            track: 2,
            scene: 7,
            notes: vec![
                MidiNote {
                    channel:0,release_vel:64,source_timing:None, id: crate::engine::midi_edit::NoteId::new(), muted: false,
                    pitch: 62,
                    start: 0.0,
                    len: 1.0,
                    vel: 80
                };
                128
            ],
        })
        .unwrap();
    engine
        .send(Command::Gesture {
            id: 24,
            command: Box::new(Command::Master(0.1)),
        })
        .unwrap();
    while engine.send(Command::Tap(std::time::Instant::now())).is_ok() {}
    assert!(engine.cmd.len() > crate::engine::control::COMMANDS_PER_BLOCK);
    engine.send(Command::SafetyStop(Safety::Silence)).unwrap();
    let counts = test_alloc::measure(|| rt.process(&mut [0.0; 256]));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert!(engine.cmd.performance().status().stopped);
    assert!(engine.cmd.performance().status().output_muted);
    assert!(Arc::ptr_eq(&original, rt.decks[0].audio.as_ref().unwrap()));
    assert!(rt.tracks[2].clips[7].notes.is_empty());
    while !rt.cmd_rx.is_empty() {
        let counts = test_alloc::measure(|| rt.process(&mut [0.0; 256]));
        assert_eq!((counts.allocations, counts.frees), (0, 0));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while weak.strong_count() != 0 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
}

#[test]
fn optional_publication_rechecks_cancel_after_commit_claim() {
    let handle = Handle::default();
    let work = handle.optional_work().unwrap();
    let result = work.commit_after_check(|| {
        handle.set_enabled(true).unwrap();
        handle.set_enabled(false).unwrap();
    });
    assert!(matches!(result, Err(Error::Protected)));
    assert!(!handle.status().changing);
    assert!(matches!(work.commit(), Err(Error::Protected)));
}

#[test]
fn new_safety_request_cannot_reuse_old_recovery_cas_state() {
    let handle = Handle::default();
    handle.request_safety(Safety::Stop);
    handle.stopped(handle.safety_request().unwrap().0);
    handle.acknowledge_inputs_released().unwrap();
    assert!(!handle.try_recover(|| {
        handle.request_safety(Safety::Silence);
        handle.stopped(handle.safety_request().unwrap().0);
        handle.acknowledge_inputs_released().unwrap();
        true
    }));
    assert!(handle.status().recovery);
    assert!(handle.try_recover(|| true));
}

#[test]
fn protection_generation_rejects_publication_before_cancellation_flags_propagate() {
    let handle = Handle::default();
    let work = handle.optional_work().unwrap();
    // Pause mode entry after its admission-word linearization, before walking
    // the fixed cancellation slots; another operator can already leave mode.
    handle
        .0
        .admission
        .fetch_add(SAFETY_GENERATION, Ordering::AcqRel);
    assert!(!work.cancel().load(Ordering::Acquire));
    assert!(matches!(work.commit(), Err(Error::Protected)));
    assert!(!handle.status().changing);
}

#[test]
fn newer_safety_request_cannot_receive_an_older_stopped_publication_or_early_ack() {
    let handle = Handle::default();
    handle.request_safety(Safety::Stop);
    let old = handle.safety_request().unwrap().0;
    handle.stopped_after_check(old, || handle.request_safety(Safety::Silence));
    assert!(!handle.status().stopped);
    assert_eq!(
        handle.acknowledge_inputs_released(),
        Err(Error::PendingStop)
    );
    let new = handle.safety_request().unwrap().0;
    assert_ne!(new, old);
    handle.stopped(new);
    assert!(
        !handle.try_recover(|| true),
        "new stop needs its own deliberate acknowledgment"
    );
    handle.acknowledge_inputs_released().unwrap();
    assert!(handle.try_recover(|| true));
}

#[test]
fn protected_hybrid_render_matches_studio_during_twenty_seconds_of_mixing_and_recording() {
    use crate::engine::audio::OutputCallback;
    fn prepare() -> (Engine, OutputCallback) {
        let (engine, mut rt) = fixture();
        rt.quant = 0.0;
        rt.apply(Command::LaunchScene { scene: 0 });
        rt.apply(Command::ComposeArm { track: 2, scene: 7 });
        rt.apply(Command::Record);
        for deck in 0..2 {
            rt.apply(Command::DeckLoop { deck, beats: 16.0 });
            if !rt.decks[deck as usize].playing {
                rt.apply(Command::DeckPlay { deck });
            }
        }
        let mut callback = OutputCallback::new(rt, 2);
        callback.render(&mut [0.0f32; 1024]);
        (engine, callback)
    }
    let (protected, mut a) = prepare();
    let (studio, mut b) = prepare();
    protected.send(Command::PerformanceMode(true)).unwrap();
    let (mut out_a, mut out_b) = ([0.0f32; 1024], [0.0f32; 1024]);
    let mut energy = 0.0f64;
    for block in 0..2048 {
        for engine in [&protected, &studio] {
            if block % 8 == 0 {
                engine
                    .send(Command::TrackGain {
                        track: 2,
                        value: 0.2 + (block % 32) as f32 / 100.0,
                    })
                    .unwrap();
                engine
                    .send(Command::Xfader((block % 128) as f32 / 127.0))
                    .unwrap();
                engine
                    .send(Command::DeckPitch {
                        deck: 1,
                        value: 0.46 + (block % 64) as f32 / 800.0,
                    })
                    .unwrap();
                engine
                    .send(Command::DeckJog {
                        deck: 0,
                        delta: if block % 32 == 0 { -0.001 } else { 0.001 },
                    })
                    .unwrap();
            }
            let note = 57 + ((block / 16) % 12) as u8;
            for source in [41, 42] {
                if block % 16 == 0 {
                    engine
                        .send(Command::LiveNoteOn {
                            source,
                            ch: 0,
                            note,
                            vel: 80,
                        })
                        .unwrap();
                }
                if block % 16 == 8 {
                    engine
                        .send(Command::LiveNoteOff {
                            source,
                            ch: 0,
                            note,
                        })
                        .unwrap();
                }
            }
        }
        let counts = test_alloc::measure(|| a.render(&mut out_a));
        assert_eq!((counts.allocations, counts.frees), (0, 0), "block {block}");
        b.render(&mut out_b);
        assert_eq!(
            out_a, out_b,
            "protection altered valid performance audio at block {block}"
        );
        assert!(out_a.iter().all(|v| v.is_finite()));
        energy += out_a.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
        std::thread::yield_now();
    }
    assert!(energy > 1.0);
    let rt = a.renderer_for_test();
    assert!(rt.playing && rt.decks.iter().all(|d| d.playing));
    assert!(!rt.has_held_project_notes());
    let notes = &rt.tracks[2].clips[7].notes;
    assert_eq!(notes.len(), 256);
    let duration = 8.0 * 512.0 / 48_000.0 * rt.bpm / 60.0;
    for (index, note) in notes.iter().enumerate() {
        assert_eq!(note.pitch, 57 + ((index / 2) % 12) as u8);
        assert!((note.len - duration).abs() < 0.00001);
    }
    assert_eq!(protected.undo.view().failures, 0);
    assert!(protected.cmd.performance().status().protected);
}

#[test]
fn emergency_envelope_has_exact_bounded_endpoints_at_supported_rates() {
    for rate in [44_100.0f32, 48_000.0, 96_000.0] {
        let mut output = Output::default();
        output.stop(Safety::Silence, rate);
        let frames = (rate * 0.002).ceil() as usize;
        let mut previous = 1.0;
        for frame in 0..frames {
            let value = output.output([1.0, -1.0]);
            let expected = (frames - frame - 1) as f32 / (frames - 1) as f32;
            assert_eq!(value, [expected, -expected]);
            assert!(value[0] <= previous);
            previous = value[0];
        }
        assert_eq!(output.output([1.0, -1.0]), [0.0; 2]);
        output.stop(Safety::Stop, rate);
        assert_eq!(output.output([f32::NAN, f32::INFINITY]), [0.0; 2]);
    }
}

#[test]
fn combined_optional_commit_rechecks_each_generation_before_cancel_flags_arrive() {
    let handle = Handle::default();
    let old = handle.optional_work().unwrap();
    // Mode entry has advanced its generation, but has not yet walked flags.
    handle
        .0
        .admission
        .fetch_add(SAFETY_GENERATION, Ordering::AcqRel);
    let current = handle.optional_work().unwrap();
    let _commit = current.commit().unwrap();
    assert!(!old.cancel().load(Ordering::Acquire));
    assert!(
        old.cancelled(),
        "a different component's guard cannot revive old work"
    );
    assert!(!current.cancelled());
}

#[test]
fn manual_grid_edits_are_destructive_and_queued_protected_requests_acknowledge_rejection_without_heap() {
    use crate::engine::beatgrid::{Grid, GridEditAck, GridEditState};
    let (engine, mut rt) = fixture();
    let receipt = engine.initial_playback[0].clone().unwrap();
    let grid = Grid::new(1.25, 120.0).unwrap();
    let ack = GridEditAck::new();
    engine.send(Command::DeckGrid { deck: 0, grid: Some(grid), receipt: receipt.clone(), ack: ack.clone() }).unwrap();
    // Protection enters after admission but before this queued edit is applied.
    engine.cmd.performance().set_enabled(true).unwrap();
    let before = engine.undo.view().cursor;
    assert_eq!(test_alloc::measure(|| tick(&mut rt)), test_alloc::Counts::default());
    assert_eq!(ack.state(), GridEditState::Rejected);
    assert_eq!(rt.decks[0].grid, None);
    assert_eq!(receipt.preparation().unwrap().1.grid, None);
    assert_eq!(engine.undo.view().cursor, before);
    for grid in [Some(grid), None] {
        assert!(matches!(engine.send(Command::DeckGrid { deck: 0, grid, receipt: receipt.clone(), ack: GridEditAck::new() }), Err(crate::engine::SubmissionError::Performance(Error::Protected))));
    }
    engine.cmd.performance().set_enabled(false).unwrap();
    let ack = GridEditAck::new();
    engine.send(Command::DeckGrid { deck: 0, grid: Some(grid), receipt, ack: ack.clone() }).unwrap();
    assert_eq!(test_alloc::measure(|| tick(&mut rt)), test_alloc::Counts::default());
    assert_eq!(ack.state(), GridEditState::Applied);
    assert_eq!(rt.decks[0].grid, Some(grid));
}

#[test]
fn explicit_deck_lock_guards_studio_loads_and_ejects_without_callback_heap_or_audio_changes() {
    let (engine, mut rt) = fixture();
    rt.apply(Command::Stop);
    rt.legacy_gain_math = true;
    rt.xfader = 0.0;
    rt.master = 1.0;
    rt.apply(Command::DeckAudio { deck: 0, audio: Arc::new(Sample { name: "continuous source".into(), path: String::new(), sr: 48000, ch: 2,
        data: vec![0.25; 65536], peaks: Arc::new(Vec::new()), bpm: 120.0 }) });
    rt.apply(Command::DeckPlay { deck: 0 });
    tick(&mut rt);
    let original = rt.decks[0].audio.clone().unwrap();
    let position = rt.decks[0].pos;
    engine.cmd.performance().set_deck_load_lock(0, true).unwrap();
    assert!(!engine.cmd.performance().protected());
    let receipt = load_receipt::Receipt::new();
    for command in [Command::DeckAudio { deck: 0, audio: original.clone() },
        Command::DeckUnload { deck: 0 }, Command::LoadBuiltin { deck: 0, stem: 1 },
        Command::DeckLoadSelected { deck: 0 }, Command::DeckLoadRequested { deck: 0,
            media: load_receipt::Media::Builtin(1), receipt: receipt.clone() }] {
        assert!(matches!(engine.send(command), Err(crate::engine::SubmissionError::Performance(Error::PlayingDeck))));
    }
    let queued = load_receipt::Receipt::new();
    let command = Command::DeckLoadRequested { deck: 0, media: load_receipt::Media::Builtin(1), receipt: queued.clone() };
    let counts = test_alloc::measure(|| rt.apply(command));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(queued.state(), load_receipt::State::Protected);
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &original));
    assert!(rt.decks[0].playing);
    let mut output = [0.0; 128];
    rt.process(&mut output);
    assert!(output.iter().all(|sample| sample.is_finite() && sample.abs() > 0.01));
    assert!(rt.decks[0].pos > position);
}

#[test]
fn reviewed_override_belongs_to_its_owner_source_and_safety_generation() {
    let (engine, mut rt) = fixture();
    rt.apply(Command::DeckPlay { deck: 0 }); tick(&mut rt);
    let handle = engine.cmd.performance();
    handle.set_deck_load_lock(0, true).unwrap();
    let review = handle.approve_deck_load(0, handle.deck_load_word(0)).unwrap();
    let foreign = Handle::default();
    assert!(!review.valid(&foreign, 0));
    let receipt = load_receipt::Receipt::new().with_deck_approval(review.clone());
    engine.send(Command::DeckLoadRequested { deck: 0, media: load_receipt::Media::Builtin(1), receipt: receipt.clone() }).unwrap();
    tick(&mut rt);
    assert_eq!(receipt.state(), load_receipt::State::Current);
    let stale = load_receipt::Receipt::new().with_deck_approval(review);
    assert!(matches!(engine.send(Command::DeckLoadRequested { deck: 0, media: load_receipt::Media::Builtin(0), receipt: stale }),
        Err(crate::engine::SubmissionError::Performance(Error::PlayingDeck))));
    let review = handle.approve_deck_load(0, handle.deck_load_word(0)).unwrap();
    let wrong = load_receipt::Receipt::new().with_deck_approval(review.clone());
    assert!(matches!(engine.send(Command::DeckLoadRequested { deck: 1, media: load_receipt::Media::Builtin(0), receipt: wrong }),
        Err(crate::engine::SubmissionError::Performance(Error::PlayingDeck))));
    handle.request_safety(Safety::Stop);
    let stale = load_receipt::Receipt::new().with_deck_approval(review);
    assert!(handle.check(&Command::DeckLoadRequested { deck: 0, media: load_receipt::Media::Unload, receipt: stale }, None).is_err());
}

#[test]
fn lock_change_after_receipt_claim_refuses_a_reviewed_renderer_mutation() {
    let (engine, mut rt) = fixture();
    rt.apply(Command::DeckPlay { deck: 0 }); tick(&mut rt);
    let original = rt.decks[0].audio.clone().unwrap();
    let handle = engine.cmd.performance().clone();
    handle.set_deck_load_lock(0, true).unwrap();
    let approval = handle.approve_deck_load(0, handle.deck_load_word(0)).unwrap();
    let receipt = load_receipt::Receipt::new().with_deck_approval(approval);
    rt.load_test_hooks[1] = Some(Box::new(move || { handle.set_deck_load_lock(0, false).unwrap(); }));
    rt.apply(Command::DeckLoadRequested { deck: 0, media: load_receipt::Media::Builtin(1), receipt: receipt.clone() });
    assert_eq!(receipt.state(), load_receipt::State::Protected);
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &original));
    assert!(rt.decks[0].playing);
}

#[test]
fn lock_entering_after_ordinary_receipt_claim_preserves_the_live_source() {
    let (engine, mut rt) = fixture();
    rt.apply(Command::DeckPlay { deck: 0 }); tick(&mut rt);
    let original = rt.decks[0].audio.clone().unwrap();
    let handle = engine.cmd.performance().clone();
    let receipt = load_receipt::Receipt::new();
    rt.load_test_hooks[1] = Some(Box::new(move || { handle.set_deck_load_lock(0, true).unwrap(); }));
    rt.apply(Command::DeckLoadRequested { deck: 0, media: load_receipt::Media::Builtin(1), receipt: receipt.clone() });
    assert_eq!(receipt.state(), load_receipt::State::Protected);
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &original));
    assert!(rt.decks[0].playing);
}

#[test]
fn studio_lock_blocks_media_history_replay_until_the_deck_is_quiet() {
    let (engine, mut rt) = fixture();
    let initial = rt.decks[0].audio.clone().unwrap();
    rt.apply(Command::LoadBuiltin { deck: 0, stem: 1 });
    let current = rt.decks[0].audio.clone().unwrap();
    assert!(!Arc::ptr_eq(&initial, &current));
    rt.apply(Command::DeckPlay { deck: 0 }); tick(&mut rt);
    engine.cmd.performance().set_deck_load_lock(0, true).unwrap();
    let cursor = engine.undo.view().cursor;
    rt.apply(Command::Undo);
    assert_eq!(engine.undo.view().cursor, cursor);
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &current));
    rt.apply(Command::DeckPlay { deck: 0 }); tick(&mut rt); tick(&mut rt);
    rt.apply(Command::Undo);
    assert!(Arc::ptr_eq(rt.decks[0].audio.as_ref().unwrap(), &initial));
    assert!(engine.cmd.performance().status().deck_load_locked[0]);
}
