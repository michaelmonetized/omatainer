use super::*;
use crate::engine::{Command, CommandPort, RtEngine, Snapshot};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn sample(name: &str) -> DecodedAudio {
    DecodedAudio {
        sample: crate::engine::dsp::Sample {
            name: name.into(),
            sr: 48000,
            ch: 1,
            data: vec![0.25; 128],
            peaks: Vec::new(),
            bpm: 120.0,
            path: name.into(),
        },
        diagnostics: Default::default(),
    }
}
fn ready(loader: &Loader, deck: usize) -> Completion {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(done) = loader.take_ready()[deck].take() {
            return done;
        }
        assert!(Instant::now() < until, "decoder failed to complete");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn newer_selection_and_unload_discard_late_success_and_failure() {
    for fail_old in [false, true] {
        let (started, seen) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let loader = Loader::with_decoder(move |path, _| {
            if path == Path::new("A") {
                started.send(()).unwrap();
                wait.recv().unwrap(); // deliberately ignore cancellation until return
                if fail_old {
                    return Err(failure("obsolete failure"));
                }
            }
            Ok(sample(path.to_str().unwrap()))
        })
        .unwrap();
        let old = loader.request(0, "A".into()).unwrap();
        seen.recv_timeout(Duration::from_secs(3)).unwrap();
        let new = loader.request(0, "B".into()).unwrap();
        assert!(new.id > old.id && !old.is_current());
        release.send(()).unwrap();
        let done = ready(&loader, 0);
        assert_eq!(done.token.id, new.id);
        assert_eq!(done.result.unwrap().sample.name, "B");
        loader.invalidate(0).unwrap();
        assert!(!done.token.is_current());
        assert!(loader.take_ready().iter().all(Option::is_none));
    }
    let (started, seen) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let (finished, finished_rx) = mpsc::channel();
    let loader = Loader::with_decoder(move |_, _| {
        started.send(()).unwrap();
        wait.recv().unwrap();
        finished.send(()).unwrap();
        Ok(sample("A"))
    })
    .unwrap();
    let old = loader.request(1, "A".into()).unwrap();
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    loader.invalidate(1).unwrap();
    release.send(()).unwrap();
    finished_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(!old.is_current());
    // Synchronizing the state mutex cannot expose a stale result, even if the
    // decoder has not yet reached its final current-token check.
    assert!(loader.take_ready().iter().all(Option::is_none));
}

#[test]
fn pending_work_and_completions_are_bounded_and_decks_remain_independent() {
    let (started, seen) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let loader = Loader::with_decoder(move |path, _| {
        recorded.lock().unwrap().push(path.to_path_buf());
        if path == Path::new("blocked") {
            started.send(()).unwrap();
            wait.recv().unwrap();
        }
        Ok(sample(path.to_str().unwrap()))
    })
    .unwrap();
    loader.request(0, "blocked".into()).unwrap();
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    for n in 0..1000 {
        loader.request(0, format!("superseded-{n}").into()).unwrap();
    }
    let last_a = loader.request(0, "latest A".into()).unwrap();
    let last_b = loader.request(1, "latest B".into()).unwrap();
    {
        let state = loader.shared.state.lock().unwrap();
        assert_eq!(state.pending.iter().flatten().count(), DECKS);
        assert_eq!(state.ready.iter().flatten().count(), 0);
    }
    release.send(()).unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        if loader
            .shared
            .state
            .lock()
            .unwrap()
            .ready
            .iter()
            .all(Option::is_some)
        {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut results = loader.take_ready();
    assert_eq!(results[0].take().unwrap().token.id, last_a.id);
    assert_eq!(results[1].take().unwrap().token.id, last_b.id);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            PathBuf::from("blocked"),
            "latest B".into(),
            "latest A".into()
        ]
    );
}

#[test]
fn cancellation_is_observable_during_decode_and_drop_does_not_wait_for_io() {
    let (started, seen) = mpsc::channel();
    let (cancelled, observed) = mpsc::channel();
    let loader = Loader::with_decoder(move |_, token| {
        started.send(()).unwrap();
        while token.is_current() {
            std::thread::sleep(Duration::from_millis(1));
        }
        cancelled.send(()).unwrap();
        Err(failure("cancelled"))
    })
    .unwrap();
    loader.request(0, "slow".into()).unwrap();
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    let start = Instant::now();
    drop(loader);
    assert!(start.elapsed() < Duration::from_millis(100));
    observed.recv_timeout(Duration::from_secs(3)).unwrap();
}

#[test]
fn completion_already_queued_to_engine_cannot_overwrite_newer_selection_or_unload() {
    let loader = Loader::with_decoder(|path, _| Ok(sample(path.to_str().unwrap()))).unwrap();
    let (commands, receiver) = CommandPort::channel(32);
    let mut rt = RtEngine::new(
        48000.0,
        receiver,
        Arc::new(parking_lot::Mutex::new(Snapshot::default())),
    );
    loader.request(0, "old".into()).unwrap();
    let old = ready(&loader, 0);
    commands
        .send(Command::DeckDecoded {
            request: old.token,
            audio: Arc::new(old.result.unwrap().sample),
        })
        .unwrap();
    loader.invalidate(0).unwrap();
    commands.send(Command::DeckUnload { deck: 0 }).unwrap();
    rt.process(&mut [0.0; 2]);
    assert!(rt.decks[0].audio.is_none());
    loader.request(0, "new".into()).unwrap();
    let new = ready(&loader, 0);
    rt.apply(Command::DeckDecoded {
        request: new.token,
        audio: Arc::new(new.result.unwrap().sample),
    });
    assert_eq!(rt.decks[0].title, "new");
    loader.request(0, "unwanted".into()).unwrap();
    let unwanted = ready(&loader, 0);
    commands
        .send(Command::DeckDecoded {
            request: unwanted.token,
            audio: Arc::new(unwanted.result.unwrap().sample),
        })
        .unwrap();
    loader.request(0, "newest".into()).unwrap();
    rt.process(&mut [0.0; 2]);
    assert_eq!(rt.decks[0].title, "new");
    let newest = ready(&loader, 0);
    rt.apply(Command::DeckDecoded {
        request: newest.token,
        audio: Arc::new(newest.result.unwrap().sample),
    });
    assert_eq!(rt.decks[0].title, "newest");
}

#[test]
fn concurrent_requests_have_unique_ordered_identities_and_only_latest_pending_work_survives() {
    let (started, seen) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let loader = Arc::new(
        Loader::with_decoder(move |path, _| {
            if path == Path::new("blocked") {
                started.send(()).unwrap();
                wait.recv().unwrap();
            }
            Ok(sample(path.to_str().unwrap()))
        })
        .unwrap(),
    );
    loader.request(0, "blocked".into()).unwrap();
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(17));
    let threads: Vec<_> = (0..16)
        .map(|index| {
            let loader = loader.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                let path = format!("file-{index}");
                (loader.request(0, path.clone().into()).unwrap(), path)
            })
        })
        .collect();
    barrier.wait();
    let requests: Vec<_> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();
    let mut ids: Vec<_> = requests.iter().map(|(token, _)| token.id).collect();
    ids.sort_unstable();
    assert_eq!(ids, (2..18).collect::<Vec<_>>());
    let (latest, path) = requests.iter().max_by_key(|(token, _)| token.id).unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|(token, _)| token.is_current())
            .count(),
        1
    );
    assert!(latest.is_current());
    release.send(()).unwrap();
    let done = ready(&loader, 0);
    assert_eq!(done.token.id, latest.id);
    assert_eq!(done.result.unwrap().sample.name, *path);
    assert!(loader.request(u8::MAX, "invalid".into()).is_err());
    assert!(loader.invalidate(u8::MAX).is_err());
    assert!(
        latest.is_current(),
        "invalid addresses must not affect a valid deck"
    );
}

fn failure(detail: &str) -> DecodeFailure {
    DecodeFailure {
        kind: super::super::decode::DecodeFailureKind::Io,
        stage: super::super::decode::DecodeStage::ReadPacket,
        diagnostics: Default::default(),
        detail: detail.into(),
    }
}
