use super::*;
use crate::engine::{Engine, Command, audio::OutputCallback};

struct Files(PathBuf);
impl Files { fn new() -> Self { Self(std::env::temp_dir().join(format!("omatainer-history-worker-{}", storage::new_id().unwrap()))) } }
impl Drop for Files { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); } }
fn source() -> Source { Source::Catalog { track_id: format!("{:032x}", 1), version: 0, title: "Drums".into(), artist: "Factory".into() } }

fn wait(worker: &mut Worker, callback: &mut OutputCallback, predicate: impl Fn(&View) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        callback.render(&mut [0.0_f32; 256]);
        if predicate(worker.poll()) { return; }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("history worker timeout: {:?}", worker.view());
}
fn receipt(worker: &mut Worker, callback: &mut OutputCallback, id: u64) {
    wait(worker, callback, |view| view.receipt.as_ref().is_some_and(|r| r.id == id));
    assert!(worker.view().receipt.as_ref().unwrap().applied, "{:?}", worker.view());
}

#[test]
fn actual_worker_records_edits_ends_saves_exports_and_reopens_a_performance_session() {
    let files = Files::new();
    let (engine, rt) = Engine::headless_for_test(48_000, 128);
    let key = engine.initial_playback[0].as_ref().unwrap().history_key();
    engine.send(Command::DeckPlay { deck: 0 }).unwrap();
    let mut callback = OutputCallback::new(rt, 4);
    let mut worker = Worker::start(files.0.clone(), engine.performance_history.clone(), engine.cmd.performance().clone()).unwrap();
    worker.register(key, source()).unwrap();
    wait(&mut worker, &mut callback, |view| view.ready);
    let start = worker.submit(Job::Start).unwrap(); receipt(&mut worker, &mut callback, start);
    let id = worker.view().active.clone().unwrap();
    for _ in 0..90 { callback.render(&mut [0.0_f32; 512]); }
    wait(&mut worker, &mut callback, |view| view.selected.as_ref().is_some_and(|s| s.entries.iter().any(|e| e.measured_seconds() > 0.0)));
    let session = worker.view().selected.as_ref().unwrap();
    let entry = session.entries.iter().find(|entry| entry.load_key == Some(key)).unwrap().id;
    let revision = session.edit_revision;
    let mark = worker.submit(Job::Mark { session: id.clone(), revision, entry, played: Some(false) }).unwrap();
    receipt(&mut worker, &mut callback, mark);
    let revision = worker.view().selected.as_ref().unwrap().edit_revision;
    let external = worker.submit(Job::External { session: id.clone(), revision, title: "External vinyl".into(), artist: "Guest".into() }).unwrap();
    receipt(&mut worker, &mut callback, external);
    // Accepted automatic history and End remain essential while protected.
    engine.cmd.performance().set_enabled(true).unwrap();
    let end = worker.submit(Job::End).unwrap(); receipt(&mut worker, &mut callback, end);
    assert!(worker.view().active.is_none() && worker.view().durable);
    let ended = worker.view().selected.as_ref().unwrap().clone();
    assert_eq!(ended.state, State::Ended); ended.validate().unwrap();
    let tracked = ended.entries.iter().find(|e| e.load_key == Some(key)).unwrap();
    assert_eq!(tracked.source, source()); assert!(!tracked.played()); assert!(tracked.measured_seconds() > 0.0);
    assert!(ended.entries.iter().any(|e| matches!(e.source, Source::External { .. }) && e.measured_seconds() == 0.0));
    engine.cmd.performance().set_enabled(false).unwrap();
    let destination = files.0.with_extension("export.json");
    let export = worker.submit(Job::Export { session: id.clone(), path: destination.clone() }).unwrap();
    receipt(&mut worker, &mut callback, export);
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&destination).unwrap()).unwrap();
    assert_eq!(json["session_id"], id); assert_eq!(json["state"], "ended");
    std::fs::remove_file(destination).unwrap();
    drop(worker);
    let deadline = Instant::now() + Duration::from_secs(5);
    let reopened = loop {
        if let Ok(store) = Store::open(files.0.clone()) { break store; }
        assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(reopened.sessions[&id], ended);
}

#[test]
fn offline_start_has_no_session_file_and_cancelled_close_allows_a_later_real_start() {
    let files = Files::new(); let (engine, mut rt) = Engine::headless_for_test(48_000, 64);
    let handle = engine.performance_history.clone().unwrap();
    let receiver = handle.take_observations().unwrap();
    let mut owner = Owner::new(files.0.clone(), Some(handle), Some(receiver));
    owner.request(Request { id: 1, job: Job::Start, permit: None });
    rt.process(&mut []); owner.collect();
    assert!(!owner.receipt.as_ref().unwrap().applied);
    assert!(owner.store.as_ref().unwrap().sessions.is_empty());
    owner.request(Request { id: 2, job: Job::Close, permit: None });
    assert!(owner.closing);
    owner.request(Request { id: 3, job: Job::KeepWorking, permit: None });
    assert!(!owner.closing);
    let mut callback = OutputCallback::new(rt, 2);
    owner.request(Request { id: 4, job: Job::Start, permit: None });
    callback.render(&mut [0.0_f32; 256]); owner.collect();
    assert!(owner.receipt.as_ref().unwrap().applied && owner.active.is_some());
    owner.request(Request { id: 5, job: Job::Close, permit: None });
    callback.render(&mut [0.0_f32; 256]); owner.collect();
    assert!(owner.active.is_none() && owner.dirty.is_empty());
}

#[test]
fn durable_prefix_and_end_include_all_queued_events_and_report_queue_overflow() {
    let files = Files::new(); let (engine, rt) = Engine::headless_for_test(48_000, 64);
    for deck in 0..2 { engine.send(Command::DeckLoop { deck, beats: 16.0 }).unwrap(); engine.send(Command::DeckPlay { deck }).unwrap(); }
    let handle = engine.performance_history.clone().unwrap(); let receiver = handle.take_observations().unwrap();
    let mut owner = Owner::new(files.0.clone(), Some(handle), Some(receiver));
    let mut callback = OutputCallback::new(rt, 2);
    for _ in 0..20 { callback.render(&mut [0.0_f32; 256]); }
    owner.request(Request { id: 1, job: Job::Start, permit: None });
    callback.render(&mut [0.0_f32; 256]); owner.collect();
    let id = owner.active.clone().unwrap();
    for _ in 0..9000 { callback.render(&mut [0.0_f32; 256]); }
    owner.collect();
    let session = &owner.store.as_ref().unwrap().sessions[&id];
    assert!(session.incomplete && session.dropped_observation_frames > 0);
    assert!(owner.staged.len() <= 1);
    owner.request(Request { id: 2, job: Job::End, permit: None });
    callback.render(&mut [0.0_f32; 256]); owner.collect();
    let ended = &owner.store.as_ref().unwrap().sessions[&id];
    ended.validate().unwrap();
    assert_eq!(ended.state, State::Ended); assert!(ended.incomplete);
    assert!(owner.observations.as_ref().unwrap().is_empty());
    assert!(owner.staged.is_empty()); assert!(owner.dirty.is_empty());
}

#[test]
fn ended_ack_survives_graph_loss_and_late_catalog_qualification() {
    let files = Files::new(); let (engine, rt) = Engine::headless_for_test(48_000, 64);
    let key = engine.initial_playback[0].as_ref().unwrap().history_key();
    let handle = engine.performance_history.clone().unwrap(); let observations = handle.take_observations().unwrap();
    let mut owner = Owner::new(files.0.clone(), Some(handle), Some(observations));
    let mut callback = OutputCallback::new(rt, 2);
    owner.request(Request { id: 1, job: Job::Start, permit: None }); callback.render(&mut [0.0_f32; 256]); owner.collect();
    let id = owner.active.clone().unwrap();
    owner.request(Request { id: 2, job: Job::End, permit: None }); callback.render(&mut [0.0_f32; 256]);
    drop(callback); // The terminal receipt is authoritative even after disconnect.
    owner.collect(); assert!(owner.receipt.as_ref().unwrap().applied);
    assert_eq!(owner.store.as_ref().unwrap().sessions[&id].state, State::Ended);
    owner.register(key, source()); owner.save_due(true);
    assert_eq!(owner.store.as_ref().unwrap().sessions[&id].entries[0].source, source());
    drop(owner);
    let reopened = Store::open(files.0.clone()).unwrap();
    assert_eq!(reopened.sessions[&id].entries[0].source, source());
    assert_eq!(reopened.sessions[&id].state, State::Ended);
}

#[test]
fn failed_end_save_retains_actual_ended_session_until_retry_can_persist_it() {
    let files = Files::new(); let (engine, rt) = Engine::headless_for_test(48_000, 64);
    let handle = engine.performance_history.clone().unwrap(); let observations = handle.take_observations().unwrap();
    let mut owner = Owner::new(files.0.clone(), Some(handle), Some(observations));
    let mut callback = OutputCallback::new(rt, 2);
    owner.request(Request { id: 1, job: Job::Start, permit: None }); callback.render(&mut [0.0_f32; 256]); owner.collect(); owner.save_due(true);
    let id = owner.active.clone().unwrap(); let displaced = files.0.with_extension("saved");
    std::fs::rename(&files.0, &displaced).unwrap(); std::fs::write(&files.0, b"temporary obstruction").unwrap();
    owner.request(Request { id: 2, job: Job::Close, permit: None }); callback.render(&mut [0.0_f32; 256]); owner.collect();
    assert!(owner.receipt.as_ref().unwrap().applied, "actual end cannot be reported rejected after save failure");
    assert!(owner.active.is_none() && !owner.view().durable);
    assert_eq!(owner.store.as_ref().unwrap().sessions[&id].state, State::Ended);
    assert!(owner.receipt.as_ref().unwrap().message.contains("save remains pending"));
    let persisted: Session = serde_json::from_slice(&std::fs::read(displaced.join(format!("{id}.json"))).unwrap()).unwrap();
    assert_eq!(persisted.state, State::Active, "last durable prefix is still recoverable");
    std::fs::remove_file(&files.0).unwrap(); std::fs::rename(&displaced, &files.0).unwrap();
    owner.request(Request { id: 3, job: Job::Retry, permit: None }); assert!(owner.view().durable);
    owner.request(Request { id: 4, job: Job::Close, permit: None }); assert!(owner.receipt.as_ref().unwrap().applied);
    drop(owner); assert_eq!(Store::open(files.0.clone()).unwrap().sessions[&id].state, State::Ended);
}
