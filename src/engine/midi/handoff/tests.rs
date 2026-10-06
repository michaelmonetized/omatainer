use super::*;
use crate::engine::midi::{cbind, nbind, rbind, RelativeSpec, UnmappedNotes};
use crate::engine::{dsp::InputKey, Engine, RtEngine};
use std::time::Instant;

fn map() -> MidiMap {
    MidiMap {
        name: "handoff test".into(),
        matchers: vec![],
        unmapped_notes: UnmappedNotes::Live,
        bindings: vec![
            nbind(0, 20, Action::DeckJogTouch, 0, 0),
            nbind(0, 30, Action::Shift, 0, 0),
            nbind(0, 31, Action::Stop, 0, 0),
            cbind(0, 7, Action::TrackFader, 0, 0),
            rbind(0, 8, Action::DeckJog, 0, 0, RelativeSpec::PIONEER_JOG),
        ],
    }
}

fn input(capacity: usize, source: u64, cmd: &CommandPort) -> (InputSink, InputWorker) {
    channel(
        capacity,
        source,
        map(),
        cmd.clone(),
        Arc::new(Mutex::new(Vec::new())),
        "test device".into(),
        "test device".into(),
        Arc::new(InputCounters::default()),
    ).unwrap()
}

fn drain(worker: &mut InputWorker) {
    while worker.step() {}
}
fn render(rt: &mut RtEngine) {
    while !rt.cmd_rx.is_empty() {
        rt.process(&mut []);
    }
}

#[test]
fn learned_beat_jump_pads_keep_note_edges_ordered_and_target_both_decks_without_heap_work() {
    use crate::engine::deck_controls::{Control, BEAT_JUMP_SIZES};
    use crate::engine::midi::{Binding, learn::{Config, Endpoint, Mapping}};
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    let (mut sink, mut worker) = input(64, 1401, &engine.cmd);
    let actions = [Action::DeckBeatJumpBack, Action::DeckBeatJumpForward, Action::DeckBeatJumpSmaller, Action::DeckBeatJumpLarger];
    let mappings: Vec<_> = (0..2).flat_map(|deck| actions.into_iter().enumerate().map(move |(i, action)| Mapping {
        endpoint: Endpoint { name: "test device".into(), id: "test device".into() },
        binding: nbind(0, 60 + deck * 4 + i as u8, action, deck, 0),
    })).collect();
    engine.cmd.midi_learn().configure(Config { mappings: mappings.clone() }).unwrap();
    for deck in 0..2 {
        let audio = rt.decks[deck].audio.as_ref().unwrap().clone();
        for (i, action) in actions.into_iter().enumerate() {
            rt.decks[deck].pos = audio.frames() as f64 * 0.5;
            let old = rt.decks[deck].pos; let other = rt.decks[1 - deck].pos;
            let size = rt.decks[deck].controls.status().beat_jump_size;
            let note = 60 + deck as u8 * 4 + i as u8;
            let counts = crate::engine::test_alloc::measure(|| { sink.push(&[0x90, note, 100]); sink.push(&[0x80, note, 0]); sink.push(&[0x90, note, 0]); });
            assert_eq!(counts, crate::engine::test_alloc::Counts::default()); drain(&mut worker);
            let actual = rt.cmd_rx.try_recv().unwrap();
            assert!(matches!(actual, Command::DeckControl { source: 1401, deck: d, .. } if usize::from(d) == deck));
            assert!(rt.cmd_rx.is_empty(), "note releases cannot duplicate a jump");
            assert_eq!(crate::engine::test_alloc::measure(|| rt.apply(actual)), crate::engine::test_alloc::Counts::default());
            assert_eq!(rt.decks[1 - deck].pos, other); assert!(!rt.decks[deck].playing);
            match action {
                Action::DeckBeatJumpBack => assert!(rt.decks[deck].pos < old),
                Action::DeckBeatJumpForward => assert!(rt.decks[deck].pos > old),
                Action::DeckBeatJumpSmaller => assert_eq!(rt.decks[deck].controls.status().beat_jump_size, size - 1),
                Action::DeckBeatJumpLarger => assert_eq!(rt.decks[deck].controls.status().beat_jump_size, size + 1),
                _ => unreachable!(),
            }
            assert!((rt.decks[deck].controls.status().beat_jump_size as usize) < BEAT_JUMP_SIZES.len());
        }
    }
    for mapping in mappings {
        for invalid in [Binding { deck: 2, ..mapping.binding }, Binding { extra: 1, ..mapping.binding }, Binding { kind: MsgKind::Cc, ..mapping.binding }] {
            assert!(crate::engine::midi::learn::validate_binding(&invalid).is_err());
        }
    }
    assert!(!Control::BeatJumpSize { index: 10 }.valid());
}

#[test]
fn learned_cue_holds_follow_note_edges_play_latches_and_worker_retirement_without_heap_work() {
    use crate::engine::midi::{Binding, learn::{Config, Endpoint, Mapping}};
    let (engine, mut rt) = Engine::headless_for_test(48_000, 64);
    let (mut sink, mut worker) = input(16, 1501, &engine.cmd);
    let mappings: Vec<_> = (0..2).flat_map(|deck| [Action::DeckCueHold, Action::DeckPlay].into_iter().enumerate().map(move |(i,action)| Mapping {
        endpoint: Endpoint { name:"test device".into(), id:"test device".into() }, binding: nbind(0,70+deck*2+i as u8,action,deck,0),
    })).collect();
    engine.cmd.midi_learn().configure(Config { mappings:mappings.clone() }).unwrap();
    for deck in 0..2usize {
        rt.apply(Command::DeckSeek {deck:deck as u8,frac:0.25}); let cue=rt.decks[deck].pos; let other=rt.decks[1-deck].pos;
        let note=70+deck as u8*2;
        assert_eq!(crate::engine::test_alloc::measure(|| sink.push(&[0x90,note,100])),crate::engine::test_alloc::Counts::default());
        drain(&mut worker); assert_eq!(crate::engine::test_alloc::measure(|| render(&mut rt)),crate::engine::test_alloc::Counts::default());
        assert!(rt.decks[deck].controls.status().cue_held); assert!(!rt.decks[deck].playing); assert!(rt.decks[deck].preview_position.is_some());
        let mut energy=0.0; for _ in 0..1024 {let (l,r)=rt.render_deck(deck);energy+=l*l+r*r;} assert!(energy>0.01);
        sink.push(&[0x90,note+1,100,0x80,note+1,0,0x90,note,0]); drain(&mut worker); render(&mut rt);
        assert!(rt.decks[deck].playing); assert!(!rt.decks[deck].controls.status().cue_held); assert!(rt.decks[deck].preview_position.is_none()); assert!(rt.decks[deck].pos>cue); assert_eq!(rt.decks[1-deck].pos,other);
        sink.push(&[0x90,note,100,0x80,note,0]); drain(&mut worker); render(&mut rt);
        assert!(!rt.decks[deck].playing); assert_eq!(rt.decks[deck].pos,cue);
        sink.push(&[0x90,note,100]); drain(&mut worker); render(&mut rt);
        sink.push(&[0x80,note,0]); drain(&mut worker); render(&mut rt); assert!(rt.decks[deck].preview_position.is_none());
    }
    for mapping in mappings.iter().filter(|m|m.binding.action==Action::DeckCueHold) {
        for invalid in [Binding {deck:2,..mapping.binding},Binding {extra:1,..mapping.binding},Binding {kind:MsgKind::Cc,..mapping.binding}] {assert!(crate::engine::midi::learn::validate_binding(&invalid).is_err());}
    }
    sink.push(&[0x90,70,100,0x90,72,100]); drain(&mut worker); render(&mut rt);
    assert!(rt.decks.iter().all(|d|d.preview_position.is_some()));
    drop(worker); render(&mut rt); assert!(rt.decks.iter().all(|d|d.preview_position.is_none()&&!d.controls.status().cue_held));
}
#[test]
fn portable_presets_retarget_real_workers_keep_factory_input_and_retire_holds_without_callback_heap_work() {
    use crate::engine::midi::{presets::Preset,learn::{Config,Endpoint,Mapping}};
    let (engine,mut rt)=Engine::headless_for_test(48_000,256);
    let origin=Endpoint{name:"test device".into(),id:"another machine".into()};
    let target=Endpoint{name:"test device".into(),id:"test device".into()};
    let config=Config{mappings:vec![Mapping{endpoint:origin.clone(),binding:nbind(0,20,Action::DeckCueHold,0,0)},Mapping{endpoint:origin.clone(),binding:rbind(0,8,Action::DeckJog,1,0,RelativeSpec{encoding:super::super::RelativeEncoding::OffsetBinary,scale:0.5})}]};
    let preset=Preset::capture("Imported".into(),String::new(),&origin,&config).unwrap();
    let preset=Preset::decode(&serde_json::to_vec(&preset).unwrap()).unwrap();
    let (mut sink,mut worker)=input(64,1651,&engine.cmd);
    let view=engine.cmd.midi_learn().view();let applied=preset.target(&target,&view.config).unwrap();
    engine.cmd.midi_learn().configure_reviewed(view.revision,1651,&target,applied).unwrap();drain(&mut worker);render(&mut rt);
    assert_eq!(crate::engine::test_alloc::measure(||sink.push(&[0x90,20,100])),crate::engine::test_alloc::Counts::default());drain(&mut worker);
    assert_eq!(crate::engine::test_alloc::measure(||render(&mut rt)),crate::engine::test_alloc::Counts::default());assert!(rt.decks[0].controls.status().cue_held);
    sink.push(&[0x90,20,0]);drain(&mut worker);render(&mut rt);assert!(!rt.decks[0].controls.status().cue_held);
    sink.push(&[0xb0,8,65]);drain(&mut worker);let command=rt.cmd_rx.try_recv().unwrap();assert!(matches!(command,Command::DeckJog{deck:1,delta} if delta==0.5));rt.apply(command);
    sink.push(&[0xb0,7,80]);drain(&mut worker);let commands:Vec<_>=rt.cmd_rx.try_iter().collect();assert!(!commands.is_empty());for command in commands {rt.apply(command);}assert_eq!(rt.tracks[0].gain,80.0/127.0,"factory input must retain its identity-qualified renderer effect");
    sink.push(&[0x90,20,100]);drain(&mut worker);render(&mut rt);assert!(rt.decks[0].controls.status().cue_held);
    let view=engine.cmd.midi_learn().view();engine.cmd.midi_learn().configure_reviewed(view.revision,1651,&target,crate::engine::midi::presets::defaults(&target,&view.config)).unwrap();drain(&mut worker);
    assert_eq!(crate::engine::test_alloc::measure(||render(&mut rt)),crate::engine::test_alloc::Counts::default());assert!(!rt.decks[0].controls.status().cue_held);
    sink.push(&[0x90,20,100]);drain(&mut worker);render(&mut rt);assert!(rt.decks[0].touching,"restoring defaults returns the original factory action");
}

fn held(rt: &RtEngine, source: u64, note: u8) -> bool {
    rt.tracks
        .iter()
        .flat_map(|track| &track.poly.voices)
        .any(|voice| {
            voice.input
                == Some(InputKey::Midi {
                    source,
                    ch: 0,
                    note,
                })
                && matches!(voice.env.stage, 1..=3)
        })
}

#[test]
fn saturated_callback_never_touches_admission_log_or_learn_locks_or_heap() {
    let (cmd, _receiver) = CommandPort::channel(32);
    let log = Arc::new(Mutex::new(Vec::new()));
    let counters = Arc::new(InputCounters::default());
    let (mut sink, guard) = start(
        71,
        map(),
        cmd.clone(),
        log.clone(),
        "blocked worker".into(),
        counters.clone(),
    )
    .unwrap();
    let log_guard = log.lock();
    let mut result = None;
    cmd.midi_learn().with_editor_lock_for_test(|| cmd.with_admission_held_for_test(|| {
        let (done, receive) = std::sync::mpsc::channel();
        let callback = std::thread::spawn(move || {
            let began = Instant::now();
            let allocations = crate::engine::test_alloc::measure(|| {
                for _ in 0..20_000 {
                    sink.push(&[0x90, 60, 100]);
                }
            });
            done.send((began.elapsed(), allocations)).unwrap();
            sink
        });
        // Locks remain held until this receive completes. A callback taking
        // any of them cannot satisfy this deliberately broad 500ms bound.
        result = Some((receive.recv_timeout(Duration::from_millis(500)), callback));
    }));
    drop(log_guard);
    let (result, callback) = result.unwrap();
    let sink = callback.join().unwrap();
    let (elapsed, allocations) = result.expect("callback blocked on a non-callback lock");
    assert_eq!(allocations.allocations, 0);
    assert_eq!(allocations.frees, 0);
    assert!(counters.snapshot().dropped > 0);
    assert_eq!(counters.snapshot().received, 20_000);
    eprintln!(
        "20,000 callbacks with blocked worker/full input queue: {elapsed:?}, {allocations:?}"
    );
    drop(sink);
    drop(guard);
}

#[test]
fn overflow_releases_only_its_source_notes_touches_and_shift_even_when_engine_is_full() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    rt.selected_track = 1;
    let (mut a, mut aw) = input(2, 11, &engine.cmd);
    let (mut b, mut bw) = input(2, 22, &engine.cmd);
    for bytes in [[0x90, 60, 100], [0x90, 20, 100], [0x90, 30, 100]] {
        a.push(&bytes);
        drain(&mut aw);
        b.push(&bytes);
        drain(&mut bw);
    }
    engine
        .cmd
        .send(Command::DeckTouch { deck: 0, on: true })
        .unwrap();
    render(&mut rt);
    assert!(held(&rt, 11, 60) && held(&rt, 22, 60));
    assert!(rt.decks[0].touching);
    while engine.cmd.send(Command::Tap(Instant::now())).is_ok() {}
    a.push(&[0x90, 61, 100]);
    a.push(&[0x90, 62, 100]);
    a.push(&[0x80, 60, 0]); // Ordered overflow: stale onsets must be discarded.
    drain(&mut aw);
    assert!(!aw.shift.lock()[0]);
    assert!(bw.shift.lock()[0]);
    render(&mut rt);
    assert!(!held(&rt, 11, 60));
    assert!(!held(&rt, 11, 61) && !held(&rt, 11, 62));
    assert!(held(&rt, 22, 60));
    assert!(rt.decks[0].touching);
    // GUI and the other controller retain independent touches.
    engine
        .cmd
        .send(Command::DeckTouch { deck: 0, on: false })
        .unwrap();
    render(&mut rt);
    assert!(rt.decks[0].touching);
    for bytes in [[0x80, 60, 0], [0x80, 20, 0], [0x80, 30, 0]] {
        b.push(&bytes);
        drain(&mut bw);
    }
    render(&mut rt);
    assert!(!held(&rt, 22, 60) && !rt.decks[0].touching);
    assert!(!bw.shift.lock()[0]);
    a.push(&[0x90, 63, 100]);
    drain(&mut aw);
    render(&mut rt);
    assert!(held(&rt, 11, 63), "new epoch did not resume input");
}

#[test]
fn overflow_preserves_mapped_and_realtime_stops_without_replaying_old_onsets() {
    for stop in [
        &[0x90, 31, 100][..],
        &[0xfc][..],
        &[0x90, 0xf8, 31, 0xf8, 100][..],
    ] {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 32);
        rt.playing = true;
        let (mut sink, mut worker) = input(1, 11, &engine.cmd);
        sink.push(&[0x90, 60, 100]);
        sink.push(stop);
        drain(&mut worker);
        render(&mut rt);
        assert!(!rt.playing && !held(&rt, 11, 60));
        assert!(sink.shared.counters.snapshot().resets > 0);
    }
}

#[test]
fn only_adjacent_absolute_cc_values_coalesce_and_barriers_keep_order() {
    let (cmd, receiver) = CommandPort::channel(256);
    let (mut sink, mut worker) = input(3, 11, &cmd);
    for value in [10, 20, 30, 40, 50] {
        sink.push(&[0xb0, 7, value]);
    }
    assert_eq!(sink.shared.counters.snapshot().coalesced, 1);
    worker.step();
    worker.step(); // Two slots free for the pending CC + note.
    sink.push(&[0x90, 60, 100]);
    drain(&mut worker);
    let commands: Vec<_> = receiver.try_iter().collect();
    assert_eq!(commands.len(), 5);
    for (command, expected) in commands[..4].iter().zip([10, 20, 30, 50]) {
        assert!(
            matches!(command, Command::TrackGain {track:0,value} if *value == expected as f32 / 127.0)
        );
    }
    assert!(matches!(
        commands[4],
        Command::LiveNoteOn {
            source: 11,
            note: 60,
            ..
        }
    ));
    assert_eq!(sink.shared.counters.snapshot().dropped, 0);
    let (mut sink, mut worker) = input(1, 12, &cmd);
    sink.push(&[0xb0, 8, 1]);
    sink.push(&[0xb0, 8, 2]);
    drain(&mut worker);
    assert_eq!(sink.shared.counters.snapshot().coalesced, 0);
    assert!(
        receiver.is_empty(),
        "relative jog was replayed after ordered overflow"
    );
}

#[test]
fn oversized_and_disconnected_input_are_bounded_and_observable() {
    let (cmd, receiver) = CommandPort::channel(32);
    let (mut sink, mut worker) = input(1, 11, &cmd);
    sink.push(&[0x90, 60, 100]);
    drain(&mut worker);
    sink.push(&[0; EVENT_BYTES + 1]);
    drain(&mut worker);
    let commands: Vec<_> = receiver.try_iter().collect();
    assert!(commands.iter().any(|cmd| matches!(
        cmd,
        Command::LiveNoteOff {
            source: 11,
            note: 60,
            ..
        }
    )));
    assert!(commands
        .iter()
        .any(|cmd| matches!(cmd, Command::ReservedStop { .. })));
    assert_eq!(sink.shared.counters.snapshot().oversized, 1);
    drop(worker);
    let allocation = crate::engine::test_alloc::measure(|| sink.push(&[0x80, 60, 0]));
    assert_eq!(allocation.allocations, 0);
    assert_eq!(sink.shared.counters.snapshot().disconnected, 1);
}

#[test]
fn renderer_touch_owner_capacity_covers_every_admitted_gate_without_heap_work() {
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    let mut admitted = 0;
    for source in 1..=256 {
        if engine
            .cmd
            .send(Command::MidiDeckTouch {
                source,
                deck: 0,
                on: true,
            })
            .is_err()
        {
            break;
        }
        rt.process(&mut []);
        admitted += 1;
    }
    assert!(admitted > 200);
    assert_eq!(rt.decks[0].touch_sources.iter().flatten().count(), admitted);
    for source in 1..=admitted as u64 {
        engine.cmd.release_midi_source(source);
        let allocation = crate::engine::test_alloc::measure(|| rt.process(&mut []));
        assert_eq!(allocation.allocations, 0);
        assert_eq!(rt.decks[0].touching, source < admitted as u64);
    }
}

#[test]
fn disconnect_releases_owned_gates_and_keeps_queued_stop_without_new_notes() {
    let (cmd, receiver) = CommandPort::channel(32);
    let (mut sink, mut worker) = input(4, 11, &cmd);
    sink.push(&[0x90, 60, 100]);
    drain(&mut worker);
    sink.push(&[0x90, 20, 100]);
    drain(&mut worker);
    receiver.try_iter().for_each(drop);
    sink.push(&[0x90, 61, 100]);
    sink.push(&[0x90, 31, 100]);
    drop(sink);
    worker.finish();
    let commands: Vec<_> = receiver.try_iter().collect();
    assert!(commands.iter().any(|cmd| matches!(
        cmd,
        Command::LiveNoteOff {
            source: 11,
            note: 60,
            ..
        }
    )));
    assert!(commands.iter().any(|cmd| matches!(
        cmd,
        Command::MidiDeckTouch {
            source: 11,
            on: false,
            ..
        }
    )));
    assert!(commands
        .iter()
        .any(|cmd| matches!(cmd, Command::ReservedStop { .. })));
    assert!(!commands
        .iter()
        .any(|cmd| matches!(cmd, Command::LiveNoteOn { .. })));
}

#[test]
fn cc_published_after_empty_fifo_probe_cannot_overtake_new_ordered_events() {
    let (cmd, receiver) = CommandPort::channel(256);
    let (mut sink, mut worker) = input(2, 11, &cmd);
    // Pause at the precise old race: FIFO was observed empty, then callback
    // publishes ordered events and an overflow CC before the worker proceeds.
    let none = worker.next_event_after_probe(|| {
        sink.push(&[0x90, 60, 100]);
        sink.push(&[0xb0, 8, 1]);
        sink.push(&[0xb0, 7, 30]);
        sink.push(&[0xb0, 7, 50]);
    });
    assert!(none.is_none());
    drain(&mut worker);
    let commands: Vec<_> = receiver.try_iter().collect();
    assert!(matches!(
        commands.as_slice(),
        [
            Command::LiveNoteOn { .. },
            Command::DeckJog { .. },
            Command::TrackGain { .. }
        ]
    ));
}

#[test]
fn mailbox_keeps_full_event_fields_coherent_under_concurrent_replacement() {
    let (mut writer, mut reader) = latest::channel();
    let done = Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let thread = std::thread::spawn(move || {
        for sequence in 1..=100_000u64 {
            writer.publish(Event::new(
                sequence ^ 0x1234_5678,
                sequence,
                &[sequence as u8; EVENT_BYTES],
            ));
        }
        finished.store(true, Release);
    });
    let mut previous = 0;
    loop {
        if let Some(event) = reader.take() {
            assert!(event.sequence > previous);
            assert_eq!(event.epoch, event.sequence ^ 0x1234_5678);
            assert_eq!(event.bytes(), &[event.sequence as u8; EVENT_BYTES]);
            previous = event.sequence;
        } else if done.load(Acquire) {
            break;
        }
    }
    thread.join().unwrap();
    // A last publication may race the empty observation before done becomes
    // visible, so inspect once more after joining before checking its value.
    if let Some(event) = reader.take() {
        previous = event.sequence;
    }
    assert_eq!(previous, 100_000);
}

#[test]
fn browse_callback_stays_allocation_free_while_gui_selection_lock_is_held() {
    let (engine, _rt) = Engine::headless_for_test(48000, 256);
    engine.ui_requests.publish_selection(Some(Arc::new(crate::engine::media_source::Selection {
        title: "captured".into(), source: crate::engine::media_source::LibSource::Builtin(crate::engine::media_source::BuiltinStem::Drums), fingerprint: None, })));
    let mut profile = map();
    profile.bindings = vec![
        rbind(0,17,Action::Browse,0,0,RelativeSpec {encoding:crate::engine::midi::RelativeEncoding::OffsetBinary,scale:1.0}),
        nbind(0,2,Action::DeckLoad,0,0),
    ];
    profile.validate().unwrap();
    let (mut callback, worker) = start(72,profile,engine.cmd.clone(),Arc::new(Mutex::new(Vec::new())),"synthetic browser".into(),Arc::new(InputCounters::default())).unwrap();
    engine.ui_requests.with_navigation_held_for_test(|| {
        let counts = crate::engine::test_alloc::measure(|| {
            for _ in 0..1000 { callback.push(&[0xb0,17,65,0x90,2,127]); }
        });
        assert_eq!(counts.allocations,0);
        assert_eq!(counts.frees,0);
    });
    drop(callback);
    drop(worker);
}

#[test]
fn performance_recovery_discards_raw_packets_before_stop_and_during_recovery() {
    use crate::engine::performance::Safety;
    let (engine, mut rt) = Engine::headless_for_test(48_000, 256);
    rt.selected_track = 1;
    let (mut sink, mut worker) = input(16, 42, &engine.cmd);
    sink.push(&[0x90, 60, 100]);
    drain(&mut worker); render(&mut rt);
    assert!(held(&rt, 42, 60));
    sink.push(&[0x90, 61, 100]); // not yet parsed at the safety boundary
    engine.send(Command::SafetyStop(Safety::Stop)).unwrap();
    rt.process(&mut []);
    sink.push(&[0x90, 62, 100]); // physically still held during recovery
    engine.send(Command::RecoverPerformance).unwrap();
    rt.process(&mut []);
    assert!(!engine.cmd.performance().status().recovery);
    drain(&mut worker); render(&mut rt);
    assert!(!held(&rt, 42, 60) && !held(&rt, 42, 61) && !held(&rt, 42, 62));
    let counts = crate::engine::test_alloc::measure(|| sink.push(&[0x90, 63, 100]));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    drain(&mut worker); render(&mut rt);
    assert!(held(&rt, 42, 63));
    assert_eq!(sink.shared.counters.snapshot().dropped, 2);
}


#[test]
fn learned_relative_cc_stays_ordered_and_old_capture_messages_are_fenced() {
    let (engine,mut rt)=Engine::headless_for_test(48_000,256);
    rt.selected_track=1;
    let (mut sink,mut worker)=input(8,1101,&engine.cmd);
    let endpoint=super::super::learn::Endpoint{name:"test device".into(),id:"test device".into()};
    let binding=rbind(0,7,Action::DeckJog,0,0,RelativeSpec{encoding:super::super::relative::RelativeEncoding::OffsetBinary,scale:1.0});
    engine.cmd.midi_learn().configure(super::super::learn::Config{mappings:vec![super::super::learn::Mapping{endpoint,binding}]}).unwrap();
    let allocations=crate::engine::test_alloc::measure(||{sink.push(&[0xb0,7,65]);sink.push(&[0xb0,7,66]);});
    assert_eq!((allocations.allocations,allocations.frees),(0,0));
    drain(&mut worker);
    assert!(matches!(rt.cmd_rx.try_recv().unwrap(),Command::DeckJog{delta:1.0,..}));
    assert!(matches!(rt.cmd_rx.try_recv().unwrap(),Command::DeckJog{delta:2.0,..}));
    assert!(rt.cmd_rx.try_recv().is_err());
    sink.push(&[0x90,60,100]);
    engine.cmd.midi_learn().begin(nbind(0,0,Action::DeckPlay,0,0),None).unwrap();
    drain(&mut worker);render(&mut rt);assert!(!held(&rt,1101,60));assert!(engine.cmd.midi_learn().view().capture.is_none());
    sink.push(&[0x90,61,100]);drain(&mut worker);assert_eq!(engine.cmd.midi_learn().view().capture.unwrap().mapping.binding.data,61);
    engine.cmd.midi_learn().cancel();sink.push(&[0x80,61,0]);sink.push(&[0x90,62,100]);drain(&mut worker);render(&mut rt);assert!(held(&rt,1101,62));
}

#[test]
fn capture_revision_releases_prior_source_gates_and_preserves_stale_safety_stop() {
    let (engine,mut rt)=Engine::headless_for_test(48_000,256);
    rt.selected_track=1;
    let (mut sink,mut worker)=input(8,1102,&engine.cmd);
    sink.push(&[0x90,60,100]);sink.push(&[0x90,20,100]);drain(&mut worker);render(&mut rt);
    assert!(held(&rt,1102,60));assert!(rt.decks[0].touching);
    rt.playing=true;sink.push(&[0xfc]);
    engine.cmd.midi_learn().begin(nbind(0,0,Action::DeckPlay,0,0),None).unwrap();
    drain(&mut worker);render(&mut rt);
    assert!(!held(&rt,1102,60));assert!(!rt.decks[0].touching);assert!(!rt.playing);
    engine.cmd.midi_learn().cancel();sink.push(&[0x90,60,100]);drain(&mut worker);render(&mut rt);assert!(held(&rt,1102,60));
    drop(worker);render(&mut rt);assert!(!held(&rt,1102,60));assert!(engine.cmd.midi_learn().view().devices.is_empty());
}

#[test]
fn cancel_fences_queued_capture_gestures_and_accepts_the_next_performance_note() {
    let (engine,mut rt)=Engine::headless_for_test(48_000,256);rt.selected_track=1;
    let (mut sink,mut worker)=input(8,1103,&engine.cmd);
    engine.cmd.midi_learn().begin(nbind(0,0,Action::DeckPlay,0,0),None).unwrap();sink.push(&[0x90,63,100]);
    engine.cmd.midi_learn().cancel();drain(&mut worker);render(&mut rt);assert!(!held(&rt,1103,63));assert!(engine.cmd.midi_learn().view().capture.is_none());
    sink.push(&[0x90,64,100]);drain(&mut worker);render(&mut rt);assert!(held(&rt,1103,64));
}

#[test]
fn surfaces_apc_and_mpd_stop_survive_callback_overflow() {
    for (map, stop) in [(crate::engine::midi::akai_apc40_mk2(), vec![0x90, 0x5c, 127]),
        (crate::engine::midi::akai_mpd232(), vec![0xb4, 117, 127]),
        (crate::engine::midi::akai_mpd232(), vec![0xf0, 0x7f, 0x7f, 6, 1, 0xf7])] {
        let (engine, mut rt) = Engine::headless_for_test(48000, 32);
        rt.playing = true;
        let (mut sink, mut worker) = channel(1, 98, map, engine.cmd.clone(), Arc::new(Mutex::new(Vec::new())),
            "surface stop test".into(), "fixture:stop".into(), Arc::new(InputCounters::default())).unwrap();
        sink.push(&[0x90, 60, 100]); sink.push(&stop);
        drain(&mut worker); render(&mut rt);
        assert!(!rt.playing && sink.shared.counters.snapshot().resets > 0);
    }
}
