use super::*;
use super::control::{Action, Outcome};
use crate::engine::*;

#[test]
fn only_real_output_can_start_and_exact_session_end_is_renderer_acknowledged() {
    let (_, rx) = crossbeam_channel::bounded(64);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    let history = rt.history_measurement.as_mut().unwrap();
    let handle = history.handle();
    let observations = history.take_receiver().unwrap();
    let denied = handle.submit(Action::Start(17)).unwrap();
    assert!(handle.poll(denied).unwrap().is_none());
    rt.process(&mut []);
    assert_eq!(handle.poll(denied).unwrap().unwrap().outcome, Outcome::NoOutput);
    assert!(observations.is_empty());
    let mut callback = audio::OutputCallback::new(rt, 4);
    let start = handle.submit(Action::Start(18)).unwrap();
    assert!(handle.submit(Action::Start(19)).is_err());
    callback.render(&mut [0.0_f32; 512]);
    assert!(handle.submit(Action::End(18)).is_err(), "receipt must be consumed before reuse");
    let ack = handle.poll(start).unwrap().unwrap();
    assert_eq!((ack.session, ack.outcome, ack.frame, ack.rate), (18, Outcome::Started, 0, 48_000));
    assert!(ack.wall_ns > 0);
    assert_eq!(handle.status().0, 18);
    let wrong = handle.submit(Action::End(17)).unwrap();
    callback.render(&mut [0.0_f32; 512]);
    assert_eq!(handle.poll(wrong).unwrap().unwrap().outcome, Outcome::WrongSession);
    assert_eq!(handle.status().0, 18);
    let duplicate = handle.submit(Action::Start(20)).unwrap();
    callback.render(&mut [0.0_f32; 512]);
    assert_eq!(handle.poll(duplicate).unwrap().unwrap().outcome, Outcome::AlreadyActive);
    let end = handle.submit(Action::End(18)).unwrap();
    // An offline retained renderer can close a session without pretending it
    // rendered another buffer. This also supports coordinated app shutdown.
    callback.renderer_mut_for_test().process(&mut []);
    let ended = handle.poll(end).unwrap().unwrap();
    assert_eq!((ended.session, ended.outcome, ended.frame), (18, Outcome::Ended, 384));
    assert!(ended.wall_ns >= ack.wall_ns);
    assert_eq!(handle.status().0, 0);
    assert!(handle.poll(end).is_err());
    drop(callback);
    assert!(!handle.status().3);
    assert!(handle.submit(Action::Start(21)).is_err());
}

#[test]
fn session_receipt_and_observations_keep_identity_under_protection_without_callback_heap() {
    let (_, rx) = crossbeam_channel::bounded(64);
    let mut rt = RtEngine::new(48_000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
    rt.apply(Command::DeckPlay { deck: 0 });
    let history = rt.history_measurement.as_mut().unwrap();
    let handle = history.handle(); let observations = history.take_receiver().unwrap();
    let mut callback = audio::OutputCallback::new(rt, 2);
    let mut data = [0.0_f32; 1024];
    callback.render(&mut data);
    callback.renderer_for_test().performance.set_enabled(true).unwrap();
    let start = handle.submit(Action::Start(471)).unwrap();
    let counts = test_alloc::measure(|| callback.render(&mut data));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    let ack = handle.poll(start).unwrap().unwrap();
    assert_eq!(ack.outcome, Outcome::Started);
    let end = handle.submit(Action::End(471)).unwrap();
    let counts = test_alloc::measure(|| callback.render(&mut data));
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(handle.poll(end).unwrap().unwrap().outcome, Outcome::Ended);
    let all: Vec<_> = observations.try_iter().collect();
    assert!(!all.is_empty());
    assert!(all.iter().all(|o| o.session == 471 && o.wall_ns >= ack.wall_ns));
    assert_eq!(all.iter().map(|o| u64::from(o.frames)).sum::<u64>(), 512);
}
