use super::*;
use crate::{
    engine::media_analysis::{
        tests::{wav, Files},
        Stage,
    },
    track_analysis::{Fields, Prepared},
};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn request(files: &Files) -> AnalysisRequest {
    AnalysisRequest {
        reference: files.source("analysis.wav", &wav(8000, 8000, 1, false)),
        fields: Fields::ALL,
    }
}
fn failed(reason: &str) -> AnalysisFailure {
    AnalysisFailure::Failed(reason.into())
}
fn wait_ready(loader: &Loader) -> AnalysisCompletion {
    let until = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(done) = loader.take_analysis_ready() {
            return done;
        }
        assert!(Instant::now() < until, "analysis worker did not complete");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn real_run(
    request: AnalysisRequest,
    token: &AnalysisToken,
    work: &performance::WorkPermit,
) -> Result<Prepared, AnalysisFailure> {
    media_analysis::run(request, token, work)
}

#[test]
fn foreground_admission_preempts_analysis_on_the_same_worker_for_decks_and_sampler() {
    for sampler in [false, true] {
        let files = Files::new();
        let (entered, seen) = mpsc::channel();
        let (resume, wait) = mpsc::channel();
        let (deck_calls, decks) = mpsc::channel();
        let loader = Loader::with_workers(
            move |_, _| {
                deck_calls.send(std::thread::current().id()).unwrap();
                Ok(super::tests::sample("foreground"))
            },
            move |_, token, work| {
                entered.send(std::thread::current().id()).unwrap();
                wait.recv_timeout(Duration::from_secs(3)).unwrap();
                token.check(work)?;
                Err(failed("should have been preempted"))
            },
            performance::Handle::default(),
        )
        .unwrap();
        let analysis = loader.request_analysis(request(&files)).unwrap();
        let worker_thread = seen.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(loader.request(255, "invalid".into()).is_err());
        assert_eq!(
            analysis.failure(),
            None,
            "failed foreground admission must not preempt"
        );
        let sampler_token = if sampler {
            let owner = assets::Owner::isolated_for_test(assets::Budget::limits());
            Some(
                loader
                    .request_sampler(
                        prepare::Request {
                            epoch: 1,
                            revision: 0,
                            sample_rate: 48000,
                            operation: prepare::Operation::Empty {
                                name: "foreground bank".into(),
                            },
                            catalog: Arc::new(crate::library::Catalog::default()),
                        },
                        owner,
                    )
                    .unwrap(),
            )
        } else {
            loader.request(0, "foreground".into()).unwrap();
            None
        };
        assert_eq!(analysis.failure(), Some(AnalysisFailure::Preempted));
        resume.send(()).unwrap();
        let done = wait_ready(&loader);
        assert_eq!(done.token.id, analysis.id);
        assert!(matches!(done.result, Err(AnalysisFailure::Preempted)));
        if let Some(token) = sampler_token {
            let until = Instant::now() + Duration::from_secs(3);
            loop {
                if let Some(done) = loader.take_sampler_ready() {
                    assert_eq!(done.token.id, token.id);
                    assert!(done.result.is_ok());
                    break;
                }
                assert!(Instant::now() < until);
                std::thread::sleep(Duration::from_millis(1));
            }
        } else {
            assert_eq!(
                decks.recv_timeout(Duration::from_secs(3)).unwrap(),
                worker_thread,
                "analysis must reuse the foreground worker, not create a decoder"
            );
        }
    }
}

#[test]
fn optional_admission_does_not_cancel_foreground_and_waits_behind_all_foreground_lanes() {
    let files = Files::new();
    let (entered, seen) = mpsc::channel();
    let (resume, wait) = mpsc::channel();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let deck_calls = calls.clone();
    let analysis_calls = calls.clone();
    let loader = Loader::with_workers(
        move |path, token| {
            deck_calls
                .lock()
                .unwrap()
                .push(path.to_string_lossy().to_string());
            if path == Path::new("A") {
                entered.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(3)).unwrap();
                assert!(
                    token.is_current(),
                    "analysis must not cancel explicit deck work"
                );
            }
            Ok(super::tests::sample("deck"))
        },
        move |request, token, work| {
            analysis_calls.lock().unwrap().push("analysis".into());
            real_run(request, token, work)
        },
        performance::Handle::default(),
    )
    .unwrap();
    loader.request(0, "A".into()).unwrap();
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    loader.request(1, "B".into()).unwrap();
    let analysis = loader.request_analysis(request(&files)).unwrap();
    assert_eq!(analysis.progress().stage, Stage::Queued);
    resume.send(()).unwrap();
    assert!(wait_ready(&loader).result.is_ok());
    assert_eq!(*calls.lock().unwrap(), ["A", "B", "analysis"]);
}

#[test]
fn analysis_pending_results_and_replacement_are_bounded_and_late_cancel_survives_take() {
    let files = Files::new();
    let request = request(&files);
    let (entered, seen) = mpsc::channel();
    let (resume, wait) = mpsc::channel();
    let loader = Loader::with_workers(
        move |_, _| {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(super::tests::sample("deck"))
        },
        real_run,
        performance::Handle::default(),
    )
    .unwrap();
    loader.request(0, "block".into()).unwrap();
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    let first = loader.request_analysis(request.clone()).unwrap();
    let mut latest = first.clone();
    for _ in 0..100 {
        latest = loader.request_analysis(request.clone()).unwrap();
    }
    assert!(!first.is_current());
    {
        let state = loader.shared.state.lock().unwrap();
        assert!(state.analysis_pending.is_some());
        assert!(state.analysis_ready.is_none());
        assert!(state.analysis_active.is_none());
    }
    resume.send(()).unwrap();
    let completion = wait_ready(&loader);
    assert_eq!(completion.token.id, latest.id);
    assert!(completion.result.is_ok());
    assert_eq!(latest.progress().stage, Stage::Ready);
    assert_eq!(latest.progress().millionths, Some(1_000_000));
    assert!(latest.cancel());
    let _commit = completion.work.commit().unwrap();
    assert_eq!(
        completion.token.claim_publication(),
        Err(AnalysisFailure::Cancelled)
    );
    assert!(loader.take_analysis_ready().is_none());
}

#[test]
fn protection_cancels_active_work_and_prepared_publication_and_drop_never_joins_io() {
    let files = Files::new();
    let performance = performance::Handle::default();
    let loader =
        Loader::with_workers(|_, _| unreachable!(), real_run, performance.clone()).unwrap();
    let token = loader.request_analysis(request(&files)).unwrap();
    let done = wait_ready(&loader);
    assert!(done.result.is_ok());
    performance.set_enabled(true).unwrap();
    performance.set_enabled(false).unwrap();
    assert!(done.work.commit().is_err());
    assert_eq!(token.check(&done.work), Err(AnalysisFailure::Protected));
    assert_eq!(token.claim_publication(), Err(AnalysisFailure::Protected));
    let (entered, seen) = mpsc::channel();
    let (resume, wait) = mpsc::channel();
    let (stopped, stop_seen) = mpsc::channel();
    let loader = Loader::with_workers(
        |_, _| unreachable!(),
        move |_, token, work| {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(token.check(work).is_err());
            stopped.send(()).unwrap();
            Err(AnalysisFailure::Cancelled)
        },
        performance.clone(),
    )
    .unwrap();
    let token = loader.request_analysis(request(&files)).unwrap();
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    let start = Instant::now();
    drop(loader);
    assert!(start.elapsed() < Duration::from_millis(100));
    assert!(!token.is_current());
    resume.send(()).unwrap();
    stop_seen.recv_timeout(Duration::from_secs(3)).unwrap();
    performance.set_enabled(true).unwrap();
    let loader = Loader::start_with_performance(performance).unwrap();
    assert!(loader.request_analysis(request(&files)).is_err());
}

#[test]
fn analysis_and_long_source_work_leave_actual_callback_output_and_heap_unchanged() {
    use crate::engine::{audio::OutputCallback, test_alloc, Command, Engine};
    let files = Files::new();
    let reference = files.source("eight-seconds.wav", &wav(384_000, 48000, 2, false));
    let (entered, seen) = mpsc::channel();
    let (resume, wait) = mpsc::channel();
    let loader = Loader::with_workers(
        |_, _| unreachable!(),
        move |request, token, work| {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            real_run(request, token, work)
        },
        performance::Handle::default(),
    )
    .unwrap();
    let (_engine, mut rt) = Engine::headless_for_test(48_000, 256);
    let (_reference_engine, mut reference_rt) = Engine::headless_for_test(48_000, 256);
    for rt in [&mut rt, &mut reference_rt] {
        rt.apply(Command::DeckPlay { deck: 0 });
        rt.apply(Command::DeckLoop {
            deck: 0,
            beats: 8.0,
        });
    }
    let mut callback = OutputCallback::new(rt, 2);
    let mut reference_callback = OutputCallback::new(reference_rt, 2);
    let mut actual = [0.0f32; 256];
    let mut expected = [0.0f32; 256];
    for _ in 0..32 {
        callback.render(&mut actual);
        reference_callback.render(&mut expected);
    }
    loader
        .request_analysis(AnalysisRequest {
            reference,
            fields: Fields::ALL,
        })
        .unwrap();
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    resume.send(()).unwrap();
    let mut energy = 0.0f64;
    for _ in 0..256 {
        let heap = test_alloc::measure(|| callback.render(&mut actual));
        assert_eq!((heap.allocations, heap.frees), (0, 0));
        reference_callback.render(&mut expected);
        assert_eq!(actual, expected);
        energy += actual.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>();
        std::thread::yield_now();
    }
    assert!(energy > 0.0);
    let result = wait_ready(&loader).result.unwrap();
    assert_eq!(result.duration, 8.0);
    assert_eq!(result.waveform.unwrap().frames, 384_000);
}

#[test]
fn concurrent_analysis_admission_orders_ids_and_only_latest_result_survives() {
    let files = Files::new();
    let request = request(&files);
    let (entered, seen) = mpsc::channel();
    let (resume, wait) = mpsc::channel();
    let loader = Arc::new(
        Loader::with_workers(
            move |_, _| {
                entered.send(()).unwrap();
                wait.recv_timeout(Duration::from_secs(3)).unwrap();
                Ok(super::tests::sample("deck"))
            },
            real_run,
            performance::Handle::default(),
        )
        .unwrap(),
    );
    loader.request(0, "block".into()).unwrap();
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    let barrier = Arc::new(std::sync::Barrier::new(9));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let loader = loader.clone();
            let barrier = barrier.clone();
            let request = request.clone();
            std::thread::spawn(move || {
                barrier.wait();
                loader.request_analysis(request).unwrap()
            })
        })
        .collect();
    barrier.wait();
    let tokens: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    let latest = tokens.iter().max_by_key(|token| token.id).unwrap();
    assert_eq!(tokens.iter().filter(|token| token.is_current()).count(), 1);
    assert!(latest.is_current());
    resume.send(()).unwrap();
    let done = wait_ready(&loader);
    assert_eq!(done.token.id, latest.id);
    assert!(done.result.is_ok());
}

#[test]
fn active_analysis_stays_cancelled_after_a_quick_protection_cycle() {
    let files = Files::new();
    let performance = performance::Handle::default();
    let (entered, seen) = mpsc::channel();
    let (resume, wait) = mpsc::channel();
    let loader = Loader::with_workers(
        |_, _| unreachable!(),
        move |request, token, work| {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            real_run(request, token, work)
        },
        performance.clone(),
    )
    .unwrap();
    let token = loader.request_analysis(request(&files)).unwrap();
    seen.recv_timeout(Duration::from_secs(3)).unwrap();
    performance.set_enabled(true).unwrap();
    performance.set_enabled(false).unwrap();
    resume.send(()).unwrap();
    let done = wait_ready(&loader);
    assert!(matches!(done.result, Err(AnalysisFailure::Protected)));
    assert_eq!(token.failure(), Some(AnalysisFailure::Protected));
    assert!(done.work.commit().is_err());
    loader.invalidate_analysis();
    assert!(!token.is_current());
}
