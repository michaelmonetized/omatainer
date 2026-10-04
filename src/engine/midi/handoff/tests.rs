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
        Arc::new(Mutex::new(None)),
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
    let (cmd, _receiver) = CommandPort::channel(16);
    let log = Arc::new(Mutex::new(Vec::new()));
    let learn = Arc::new(Mutex::new(None));
    let counters = Arc::new(InputCounters::default());
    let (mut sink, guard) = start(
        71,
        map(),
        cmd.clone(),
        log.clone(),
        learn.clone(),
        "blocked worker".into(),
        counters.clone(),
    )
    .unwrap();
    let log_guard = log.lock();
    let learn_guard = learn.lock();
    let mut result = None;
    cmd.with_admission_held_for_test(|| {
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
    });
    drop(learn_guard);
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
        let (engine, mut rt) = Engine::headless_for_test(48_000, 16);
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
    let (cmd, receiver) = CommandPort::channel(16);
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
    let (cmd, receiver) = CommandPort::channel(16);
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
    let (mut callback, worker) = start(72,profile,engine.cmd.clone(),Arc::new(Mutex::new(Vec::new())),Arc::new(Mutex::new(None)),"synthetic browser".into(),Arc::new(InputCounters::default())).unwrap();
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
