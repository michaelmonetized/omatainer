use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

#[test]
fn command_backlog_defers_note_off_without_reordering_or_dropping_it() {
    let (tx, rx) = crossbeam_channel::bounded(256);
    let snap = Arc::new(Mutex::new(Snapshot::default()));
    let mut engine = RtEngine::new(48_000.0, rx, snap.clone());
    engine.selected_track = 1;
    for _ in 0..control::COMMANDS_PER_BLOCK - 1 {
        tx.send(Command::Master(0.8)).unwrap();
    }
    tx.send(Command::LiveNoteOn {
        source: 0,
        ch: 0,
        note: 60,
        vel: 100,
    })
    .unwrap();
    tx.send(Command::LiveNoteOff { source: 0, ch: 0, note: 60 }).unwrap();
    let mut out = [0.0; 128];
    engine.process(&mut out);
    assert!(engine.tracks[1]
        .poly
        .voices
        .iter()
        .any(|v| v.note == 60 && (1..=3).contains(&v.env.stage)));
    assert_eq!(engine.command_stats.backlog, 1);
    assert_eq!(engine.command_stats.applied_last_block, 2);
    engine.publish_for_test();
    assert_eq!(snap.lock().commands.budget_exhaustions, 1);
    engine.process(&mut out);
    assert!(!engine.tracks[1]
        .poly
        .voices
        .iter()
        .any(|v| v.note == 60 && (1..=3).contains(&v.env.stage)));
    assert_eq!(engine.command_stats.received, 33);
    assert_eq!(engine.command_stats.applied, 3);
    assert_eq!(engine.command_stats.coalesced, 30);
    assert_eq!(engine.command_stats.backlog, 0);
}

#[test]
fn sustained_concurrent_producers_cannot_extend_the_callback_command_budget() {
    let (tx, rx) = crossbeam_channel::bounded(256);
    let mut engine = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    engine.selected_track = 1;
    engine.decks[0].playing = true;
    // The consumer can render far ahead while the producer is descheduled.
    // Keep the finite demo media looping and count distance across wraps;
    // final position alone can be near zero after any amount of progress.
    engine.decks[0].loop_on = true;
    engine.decks[0].loop_start = 0.0;
    engine.decks[0].loop_len = 8192.0;
    let mut rendered_distance = 0.0;
    for _ in 0..256 {
        tx.send(Command::Master(0.8)).unwrap();
    }
    let finished = Arc::new(AtomicBool::new(false));
    let worker_finished = finished.clone();
    let (ready_tx, ready_rx) = crossbeam_channel::bounded(0);
    let producer = std::thread::spawn(move || {
        ready_tx.send(()).unwrap();
        for i in 0..8192 {
            // Complete every on/off pair through the same FIFO, retrying
            // admission by blocking only this synthetic producer thread.
            let command = match i % 4 {
                0 => Command::LiveNoteOn {
                    source: 0,
                    ch: 0,
                    note: 60,
                    vel: 100,
                },
                1 => Command::Master(0.6),
                2 => Command::Master(0.8),
                _ => Command::LiveNoteOff { source: 0, ch: 0, note: 60 },
            };
            tx.send(command).unwrap();
        }
        tx.send(Command::LiveNoteOff { source: 0, ch: 0, note: 60 }).unwrap();
        worker_finished.store(true, Ordering::Release);
        8193_u64
    });
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut out = [0.0; 256];
    let mut maximum = Duration::ZERO;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut blocks = 0;
    while !finished.load(Ordering::Acquire) || !engine.cmd_rx.is_empty() {
        assert!(Instant::now() < deadline, "command stress did not finish");
        let before = engine.frames_done;
        let previous_position = engine.decks[0].pos;
        let started = Instant::now();
        engine.process(&mut out);
        let elapsed = started.elapsed();
        rendered_distance += (engine.decks[0].pos - previous_position).rem_euclid(8192.0);
        maximum = maximum.max(elapsed);
        // A generous liveness guard, not a device deadline qualification.
        assert!(
            elapsed < Duration::from_secs(1),
            "callback stalled: {elapsed:?}"
        );
        blocks += 1;
        assert!(engine.command_stats.received_last_block <= control::COMMANDS_PER_BLOCK);
        assert_eq!(engine.frames_done - before, 128);
        assert!(out.iter().all(|s| s.is_finite()));
    }
    let sent = producer.join().unwrap();
    assert_eq!(sent, 8193);
    assert!(blocks >= (256 + sent as usize).div_ceil(control::COMMANDS_PER_BLOCK));
    assert_eq!(engine.command_stats.received, 256 + sent);
    assert!(engine.command_stats.budget_exhaustions > 0);
    assert!(rendered_distance > 32_000.0, "deck must advance while commands drain");
    // No direct rescue command: the final release must have traversed the
    // saturated queue and actual callback consumer.
    assert!(!engine.tracks[1]
        .poly
        .voices
        .iter()
        .any(|v| (1..=3).contains(&v.env.stage)));
    eprintln!("concurrent producer: {sent} accepted events; max observed 128-frame callback {maximum:?}; dequeue limit {}", control::COMMANDS_PER_BLOCK);
}
