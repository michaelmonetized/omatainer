use super::*;
use std::sync::mpsc;

fn queue(scheduler: &Scheduler, kind: Kind, key: &str, bytes: u64) -> Ticket {
    scheduler
        .request(
            kind,
            key.into(),
            bytes,
            Cancellation::Flag(Arc::new(AtomicBool::new(false))),
        )
        .unwrap()
}
#[test]
fn equivalent_requests_capacity_and_memory_refuse_before_start_and_ids_never_reuse() {
    let scheduler = Scheduler::default();
    let first = queue(&scheduler, Kind::Index, "same", MIB);
    assert!(scheduler
        .request(
            Kind::Index,
            "same".into(),
            MIB,
            Cancellation::Flag(Arc::new(AtomicBool::new(false)))
        )
        .is_err());
    assert!(scheduler
        .request(
            Kind::Index,
            "too-large".into(),
            MEMORY_BYTES + 1,
            Cancellation::Flag(Arc::new(AtomicBool::new(false)))
        )
        .is_err());
    let old = first.0.record.id;
    drop(first);
    let mut tickets = Vec::new();
    for id in 0..RECORDS {
        tickets.push(queue(&scheduler, Kind::Index, &format!("key-{id}"), MIB));
    }
    assert!(scheduler
        .request(
            Kind::Index,
            "full".into(),
            MIB,
            Cancellation::Flag(Arc::new(AtomicBool::new(false)))
        )
        .is_err());
    assert!(tickets.iter().all(|ticket| ticket.0.record.id > old));
    assert!(!scheduler.cancel(old));
    assert_eq!(scheduler.snapshot().running, 0);
}
#[test]
fn optional_workers_wait_then_cancel_without_leaking_a_turn_or_reservation() {
    let scheduler = Scheduler::default();
    let first = queue(&scheduler, Kind::Index, "first", MIB);
    let second = queue(&scheduler, Kind::Download, "second", MIB);
    let a = first.enter(|| false).unwrap();
    let b = second.enter(|| false).unwrap();
    let third = queue(&scheduler, Kind::Render, "third", MIB);
    let id = third.0.record.id;
    let (done, finished) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        let result = third.enter(|| false);
        done.send(result.is_err()).unwrap();
    });
    assert!(finished.recv_timeout(Duration::from_millis(50)).is_err());
    assert_eq!(scheduler.snapshot().running, 2);
    assert_eq!(scheduler.snapshot().reserved, 2 * MIB);
    assert!(scheduler.cancel(id));
    assert!(finished.recv_timeout(Duration::from_secs(2)).unwrap());
    worker.join().unwrap();
    drop(a);
    drop(b);
    assert_eq!(scheduler.snapshot().reserved, 0);
    assert_eq!(scheduler.snapshot().running, 0);
}
#[test]
fn foreground_admission_preempts_a_full_optional_budget_and_keeps_the_next_identity() {
    let scheduler = Scheduler::default();
    let flag = Arc::new(AtomicBool::new(false));
    let optional = scheduler
        .request(
            Kind::Analysis,
            "analysis".into(),
            MEMORY_BYTES,
            Cancellation::Flag(flag.clone()),
        )
        .unwrap();
    let running = optional.enter(|| false).unwrap();
    let generation = Arc::new(AtomicU64::new(7));
    let decode = scheduler
        .request(
            Kind::Decode,
            "deck".into(),
            MIB,
            Cancellation::Generation {
                current: generation.clone(),
                id: 7,
            },
        )
        .unwrap();
    assert!(flag.load(Ordering::Acquire));
    drop(running);
    let active = decode.enter(|| false).unwrap();
    let id = decode.0.record.id;
    generation.store(9, Ordering::Release);
    assert!(scheduler.cancel(id));
    assert_eq!(generation.load(Ordering::Acquire), 9);
    drop(active);
    assert_eq!(scheduler.snapshot().reserved, 0);
}
#[test]
fn progress_and_native_priorities_are_measured_only_on_the_owned_worker() {
    let parent = unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) };
    let parent_policy = unsafe { libc::sched_getscheduler(0) };
    let scheduler = Scheduler::default();
    let worker_scheduler = scheduler.clone();
    std::thread::spawn(move || {
        let ticket = queue(&worker_scheduler, Kind::Index, "observed", MIB);
        let running = ticket.enter(|| false).unwrap();
        ticket.progress(13, None);
        let row = worker_scheduler.snapshot().rows.pop().unwrap();
        assert_eq!((row.done, row.total), (13, None));
        assert!(row.nice.unwrap() >= 10);
        assert_eq!(
            unsafe { libc::syscall(libc::SYS_ioprio_get, 1, 0) },
            3 << 13
        );
        assert_eq!(unsafe { libc::sched_getscheduler(0) }, libc::SCHED_OTHER);
        ticket.progress(30, Some(20));
        let row = worker_scheduler.snapshot().rows.pop().unwrap();
        assert_eq!((row.done, row.total), (20, Some(20)));
        drop(running);
    })
    .join()
    .unwrap();
    assert_eq!(unsafe { libc::getpriority(libc::PRIO_PROCESS, 0) }, parent);
    assert_eq!(unsafe { libc::sched_getscheduler(0) }, parent_policy);
    assert_eq!(scheduler.snapshot().reserved, 0);
}
#[test]
fn retired_scheduler_records_cannot_pin_or_cancel_reused_optional_flags() {
    let handle = crate::engine::performance::Handle::default();
    let mut old_ids = Vec::new();
    for id in 0..96 {
        let permit = handle.optional_work().unwrap();
        let ticket = permit
            .background(Kind::Index, format!("flags-{id}"), MIB)
            .unwrap();
        old_ids.push(ticket.0.record.id);
        let running = ticket.enter(|| permit.cancelled()).unwrap();
        drop(running);
        drop(ticket);
        drop(permit);
    }
    assert_eq!(handle.status().optional_active, 0);
    let permit = handle.optional_work().unwrap();
    let ticket = permit
        .background(Kind::Index, "current".into(), MIB)
        .unwrap();
    for id in old_ids {
        assert!(!handle.jobs().cancel(id));
    }
    assert!(!permit.cancelled());
    drop(ticket);
    drop(permit);
}

#[test]
#[ignore = "Controlled optimized callback qualification: taskset -c 6, --exact, --nocapture"]
fn actual_decode_io_pressure_cancellation_supersession_and_exhaustion_preserve_callback_deadlines()
{
    use crate::engine::media_analysis::tests::{wav, Files};
    use crate::engine::{
        audio::OutputCallback, dsp::Sample, media_load::Loader, test_alloc, Command, Engine,
    };
    use std::time::Instant;
    assert!(
        !cfg!(debug_assertions),
        "Use the optimized qualification binary"
    );
    let files = Files::new();
    let first = files.source("first.wav", &wav(8000, 8000, 1, false));
    let second = files.source("second.wav", &wav(8000, 8000, 1, false));
    let path = files.0.join("first.wav");
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let sample = Arc::new(Sample {
        name: "continuous qualification signal".into(),
        path: String::new(),
        sr: 48000,
        ch: 2,
        data: vec![0.25; 48000 * 8],
        peaks: Arc::new(Vec::new()),
        bpm: 120.0,
    });
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: sample,
    });
    rt.apply(Command::DeckPlay { deck: 0 });
    let original = rt.decks[0].audio.clone().unwrap();
    let scheduler = engine.cmd.performance().jobs().clone();
    let loader = Loader::start_with_performance(engine.cmd.performance().clone()).unwrap();
    let (entered, seen) = mpsc::channel();
    let mut workers = Vec::new();
    for (kind, bytes) in [(Kind::Index, MEMORY_BYTES), (Kind::Download, 128 * MIB)] {
        let flag = Arc::new(AtomicBool::new(false));
        let ticket = scheduler
            .request(
                kind,
                format!("pressure-{kind:?}"),
                bytes,
                Cancellation::Flag(flag.clone()),
            )
            .unwrap();
        let entered = entered.clone();
        let path = path.clone();
        workers.push(std::thread::spawn(move || {
            let running = ticket.enter(|| false).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            std::hint::black_box(Sha256::digest(&bytes));
            let mut processed = bytes.len() as u64;
            ticket.progress(processed, None);
            entered.send((ticket.0.record.id, flag.clone())).unwrap();
            while !flag.load(Ordering::Acquire) {
                let bytes = std::fs::read(&path).unwrap();
                let hash = Sha256::digest(&bytes);
                std::hint::black_box(hash);
                processed += bytes.len() as u64;
                ticket.progress(processed, None);
            }
            drop(running);
            processed
        }));
        if kind == Kind::Index {
            seen.recv_timeout(Duration::from_secs(3)).unwrap();
        }
    }
    let mut callback = OutputCallback::new(rt, 2);
    let mut output = [0.0f32; 256];
    callback.render(&mut output);
    let mut timings = Vec::with_capacity(768);
    let mut hash = Sha256::new();
    let mut capacity_refusals = 0;
    let mut queued = Vec::new();
    let mut optional_cancelled = false;
    let mut decoded = 0;
    let mut last = [0; 2];
    let start = Instant::now();
    let period = Duration::from_nanos(128 * 1_000_000_000 / 48000);
    for block in 0..768 {
        if block == 0 {
            let token = loader.request_source(1, first.source.clone()).unwrap();
            last[1] = token.id;
        }
        if block == 32 {
            let (id, _) = seen.recv_timeout(Duration::from_secs(3)).unwrap();
            assert!(scheduler.cancel(id));
            optional_cancelled = true;
            loop {
                match scheduler.request(
                    Kind::Render,
                    format!("capacity-{}", queued.len()),
                    MIB,
                    Cancellation::Flag(Arc::new(AtomicBool::new(false))),
                ) {
                    Ok(ticket) => queued.push(ticket),
                    Err(_) => {
                        capacity_refusals += 1;
                        break;
                    }
                }
            }
            assert_eq!(scheduler.snapshot().rows.len(), RECORDS);
            assert!(!queued.is_empty());
        }
        if block == 48 {
            for ticket in &queued {
                assert!(scheduler.cancel(ticket.0.record.id));
            }
            queued.clear();
        }
        if block == 64 || block == 128 {
            for request in 0..100 {
                let source = if request % 2 == 0 {
                    first.source.clone()
                } else {
                    second.source.clone()
                };
                let token = loader.request_source(1, source).unwrap();
                last[1] = token.id;
            }
        }
        if block == 256 {
            assert!(scheduler
                .request(
                    Kind::Render,
                    "over-budget".into(),
                    MEMORY_BYTES + 1,
                    Cancellation::Flag(Arc::new(AtomicBool::new(false)))
                )
                .is_err());
            capacity_refusals += 1;
        }
        for ready in loader.take_ready().into_iter().flatten() {
            assert_eq!(ready.token.id, last[ready.token.deck as usize]);
            assert!(ready.result.is_ok());
            decoded += 1;
        }
        let before = Instant::now();
        let counts = test_alloc::measure(|| callback.render(&mut output));
        timings.push(before.elapsed().as_nanos() as u64);
        assert_eq!((counts.allocations, counts.frees), (0, 0));
        assert!(output.iter().all(|v| v.is_finite()));
        assert!(output.iter().any(|v| v.abs() > 0.01));
        for value in &output {
            hash.update(value.to_le_bytes());
        }
        let due = start + period * (block + 1);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
    }
    let processed: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert!(optional_cancelled && processed.iter().all(|bytes| *bytes > 0));
    assert!(decoded >= 2);
    assert_eq!(capacity_refusals, 2);
    assert!(Arc::ptr_eq(
        callback.renderer_for_test().decks[0]
            .audio
            .as_ref()
            .unwrap(),
        &original
    ));
    assert!(callback.renderer_for_test().decks[0].playing);
    timings.sort_unstable();
    let metrics = engine.cmd.audio_metrics();
    println!(
        "BACKGROUND_CALLBACK_QUALIFICATION {}",
        serde_json::json!({"platform":"Linux aarch64 software callback","blocks":timings.len(),"frames":128,"sample_rate":48000,"deadline_ns":period.as_nanos(),"p99_wall_ns":timings[timings.len()*99/100],"max_wall_ns":timings.last(),"deadline_overruns":metrics.deadline_overruns,"callback_allocations":0,"callback_frees":0,"processed_io_bytes":processed,"completed_current_decodes":decoded,"capacity_refusals":capacity_refusals,"audio_sha256":format!("{:x}",hash.finalize()),"scope":"Actual production file decoding, controlled admitted file-read/hash pressure and full request/memory refusal; no physical backend dropout claim"})
    );
    assert_eq!(
        metrics.deadline_overruns, 0,
        "Controlled callbacks missed their native block budget"
    );
    assert!(*timings.last().unwrap() < period.as_nanos() as u64);
    assert_eq!(scheduler.snapshot().reserved, 0);
}
