use super::{test_support::*, *};
use crate::engine::{test_alloc, Command, Engine};
use std::time::{Duration, Instant};

#[test]
fn injected_discovery_connect_failure_retry_and_duplicate_names_are_truthful() {
    let (mut engine, mut rt) = Engine::headless_for_test(48_000, 80);
    rt.selected_track = 1;
    let control = install(&mut engine);
    assert!(engine.snapshot().midi[0].contains("[connected] keyboard + mouse"));
    assert_eq!(engine.midi.retry_connections(), Retry::AlreadyRunning);
    control.discover(&[("1:0", "same keyboard"), ("2:0", "same keyboard")]);
    let Attempt::Connect { id, .. } = control.next() else {
        panic!()
    };
    assert_eq!(id, "1:0");
    let labels = engine.snapshot().midi;
    assert!(labels[1].starts_with("[connecting]"));
    assert!(labels[2].starts_with("[discovered]"));
    control
        .replies
        .send(Reply::Connected(Err("permission denied".into())))
        .unwrap();
    let second = control.connect("2:0", Ok(()));
    until(|| !engine.midi.connections_busy());
    let labels = engine.snapshot().midi;
    assert!(labels[0].starts_with("[connected]"));
    assert!(
        labels[1].contains("[failed] same keyboard") && labels[1].contains("permission denied")
    );
    assert!(labels[2].starts_with("[connected]"));
    rt.publish_for_test();
    assert_eq!(engine.snapshot().midi, labels);

    assert_eq!(engine.midi.retry_connections(), Retry::Queued);
    for _ in 0..1000 {
        assert_eq!(engine.midi.retry_connections(), Retry::AlreadyRunning);
    }
    control.discover(&[("1:0", "same keyboard"), ("2:0", "same keyboard")]);
    let first = control.connect("1:0", Ok(()));
    until(|| !engine.midi.connections_busy());
    assert!(
        control.attempts.try_recv().is_err(),
        "connected port was reopened"
    );
    assert!(engine
        .snapshot()
        .midi
        .iter()
        .all(|label| label.starts_with("[connected]")));
    let counts = test_alloc::measure(|| {
        first.push(&[0x90, 60, 100]);
        second.push(&[0x90, 60, 100]);
    });
    assert_eq!(counts, test_alloc::Counts::default());
    until(|| engine.midi.input_stats().dispatched >= 2);
    rt.process(&mut [0.0; 128]);
    assert_eq!(
        rt.tracks[1]
            .poly
            .voices
            .iter()
            .filter(|voice| voice.input.is_some() && (1..=3).contains(&voice.env.stage))
            .count(),
        2
    );
    assert_eq!(engine.midi.retry_connections(), Retry::Queued);
    control.discover(&[("2:0", "same keyboard")]);
    until(|| !engine.midi.connections_busy());
    assert!(control.attempts.try_recv().is_err());
    rt.process(&mut [0.0; 128]);
    assert!(first.is_closed());
    assert!(!second.is_closed());
    assert_eq!(
        rt.tracks[1]
            .poly
            .voices
            .iter()
            .filter(|voice| voice.input.is_some() && (1..=3).contains(&voice.env.stage))
            .count(),
        1,
        "removing one device must preserve the other same-pitch gate"
    );
}

#[test]
fn discovery_failure_keeps_fallback_and_retry_recovers_without_duplicate_rows() {
    let (mut engine, _rt) = Engine::headless_for_test(48_000, 48);
    let control = install(&mut engine);
    assert!(matches!(control.next(), Attempt::Discover));
    control
        .replies
        .send(Reply::Ports(Err("backend permission denied".into())))
        .unwrap();
    until(|| !engine.midi.connections_busy());
    let labels = engine.snapshot().midi;
    assert!(labels[0].contains("[connected] keyboard + mouse"));
    assert!(labels[1].contains("[failed]") && labels[1].contains("backend permission denied"));
    assert_eq!(engine.midi.retry_connections(), Retry::Queued);
    control.discover(&[("1", "MIDI keyboard")]);
    control.connect("1", Err("port is busy"));
    until(|| !engine.midi.connections_busy());
    assert!(engine.snapshot().midi[2].contains("port is busy"));
    assert_eq!(engine.midi.retry_connections(), Retry::Queued);
    control.discover(&[("1", "MIDI keyboard")]);
    control.connect("1", Ok(()));
    until(|| !engine.midi.connections_busy());
    let labels = engine.snapshot().midi;
    assert_eq!(labels.len(), 3);
    assert!(labels.iter().all(|label| label.starts_with("[connected]")));
}

#[test]
fn retry_detects_missing_ports_releases_their_notes_and_reconnects_them() {
    let (mut engine, mut rt) = Engine::headless_for_test(48_000, 48);
    rt.selected_track = 1;
    let control = install(&mut engine);
    control.discover(&[("1", "MIDI keyboard")]);
    let input = control.connect("1", Ok(()));
    until(|| !engine.midi.connections_busy());
    input.push(&[0x90, 60, 100]);
    until(|| engine.midi.input_stats().dispatched == 1);
    rt.process(&mut [0.0; 128]);
    assert_eq!(
        rt.tracks[1]
            .poly
            .voices
            .iter()
            .filter(|voice| voice.input.is_some() && (1..=3).contains(&voice.env.stage))
            .count(),
        1
    );
    assert_eq!(engine.midi.retry_connections(), Retry::Queued);
    control.discover(&[]);
    until(|| !engine.midi.connections_busy());
    rt.process(&mut [0.0; 128]);
    assert_eq!(
        rt.tracks[1]
            .poly
            .voices
            .iter()
            .filter(|voice| voice.input.is_some() && (1..=3).contains(&voice.env.stage))
            .count(),
        0
    );
    assert!(input.is_closed());
    assert!(engine.snapshot().midi[1].starts_with("[disconnected]"));
    assert_eq!(engine.midi.retry_connections(), Retry::Queued);
    control.discover(&[("1", "MIDI keyboard")]);
    control.connect("1", Ok(()));
    until(|| !engine.midi.connections_busy());
    assert_eq!(engine.snapshot().midi.len(), 2);
    assert!(engine.snapshot().midi[1].starts_with("[connected]"));
}

#[test]
fn slow_backend_never_blocks_control_renderer_or_manager_drop() {
    let (mut engine, mut rt) = Engine::headless_for_test(48_000, 48);
    let control = install(&mut engine);
    control.discover(&[("1", "MIDI keyboard")]);
    let Attempt::Connect { input, .. } = control.next() else {
        panic!()
    };
    let started = Instant::now();
    for i in 0..100 {
        assert_eq!(engine.midi.retry_connections(), Retry::AlreadyRunning);
        engine.cmd.send(Command::Master(i as f32 / 100.0)).unwrap();
        rt.process(&mut [0.0; 128]);
    }
    assert!(started.elapsed() < Duration::from_secs(1));
    let started = Instant::now();
    engine.midi = super::super::MidiHub::without_devices();
    assert!(started.elapsed() < Duration::from_millis(100));
    control.replies.send(Reply::Connected(Ok(()))).unwrap();
    until(|| input.is_closed());
    until(|| engine.snapshot().midi[1].starts_with("[disconnected]"));
}

#[test]
fn simultaneous_retry_admission_has_one_request_and_no_followup_backlog() {
    let (mut engine, _rt) = Engine::headless_for_test(48_000, 48);
    let control = install(&mut engine);
    control.discover(&[]);
    until(|| !engine.midi.connections_busy());
    let barrier = std::sync::Barrier::new(9);
    let manager = engine.midi.connections.as_ref().unwrap();
    let replies = std::thread::scope(|scope| {
        let callers: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    manager.retry()
                })
            })
            .collect();
        barrier.wait();
        callers
            .into_iter()
            .map(|caller| caller.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        replies
            .iter()
            .filter(|reply| **reply == Retry::Queued)
            .count(),
        1
    );
    assert_eq!(
        replies
            .iter()
            .filter(|reply| **reply == Retry::AlreadyRunning)
            .count(),
        7
    );
    control.discover(&[]);
    until(|| !manager.busy());
    assert!(control.attempts.try_recv().is_err());
}

#[test]
fn performance_protects_retry_and_pending_rediscovery_prevents_mode_entry() {
    let (mut engine, _rt) = Engine::headless_for_test(48_000, 80);
    let control = install(&mut engine);
    assert!(matches!(
        engine.send(Command::PerformanceMode(true)),
        Err(crate::engine::SubmissionError::Performance(
            crate::engine::performance::Error::Changing
        ))
    ));
    control.discover(&[]);
    until(|| !engine.midi.connections_busy());
    engine.send(Command::PerformanceMode(true)).unwrap();
    assert_eq!(
        engine.midi.retry_connections(),
        Retry::Performance(crate::engine::performance::Error::Protected)
    );
    assert!(control.attempts.try_recv().is_err());
    engine.send(Command::PerformanceMode(false)).unwrap();
    assert_eq!(engine.midi.retry_connections(), Retry::Queued);
    assert!(matches!(control.next(), Attempt::Discover));
    assert!(matches!(
        engine.send(Command::PerformanceMode(true)),
        Err(crate::engine::SubmissionError::Performance(
            crate::engine::performance::Error::Changing
        ))
    ));
    control
        .replies
        .send(Reply::Ports(Err("injected discovery failure".into())))
        .unwrap();
    until(|| !engine.midi.connections_busy());
    engine.send(Command::PerformanceMode(true)).unwrap();
    assert!(engine.cmd.performance().status().protected);
}

#[test]
fn protected_presence_retirement_releases_only_lost_sources_and_defers_new_connections() {
    let (mut engine, mut rt) = Engine::headless_for_test(48000, 80);
    rt.selected_track = 1;
    let control = install(&mut engine);
    control.discover(&[("1", "A"), ("2", "B")]);
    let a = control.connect("1", Ok(()));
    let b = control.connect("2", Ok(()));
    until(|| !engine.midi.connections_busy());
    a.push(&[0x90, 60, 100]);
    b.push(&[0x90, 60, 100]);
    until(|| engine.midi.input_stats().dispatched == 2);
    rt.process(&mut [0.0; 128]);
    engine.cmd.performance().set_enabled(true).unwrap();
    *control.presence.lock() = Some(vec![("2".into(), "2".into()), ("3".into(), "3".into())]);
    until(|| a.is_closed());
    assert!(!b.is_closed());
    assert!(control.attempts.try_recv().is_err());
    rt.process(&mut [0.0; 128]);
    assert_eq!(
        rt.tracks[1]
            .poly
            .voices
            .iter()
            .filter(|v| v.input.is_some() && (1..=3).contains(&v.env.stage))
            .count(),
        1
    );
    assert!(!rt.playing);
    assert!(engine.snapshot().midi[1].contains("disconnected"));
    engine.cmd.performance().set_enabled(false).unwrap();
    control.discover(&[("2", "B"), ("3", "C")]);
    control.connect("3", Ok(()));
    until(|| !engine.midi.connections_busy());
    assert!(!b.is_closed());
}
