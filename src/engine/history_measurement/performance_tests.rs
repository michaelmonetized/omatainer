use crate::engine::keylock_showload_tests;

#[test]
#[ignore = "local release-profile active-history timing, never ordinary CI"]
fn active_history_show_budget() {
    assert!(!cfg!(debug_assertions), "run with --release");
    let mut reports = Vec::new();
    for rate in [44_100, 48_000, 96_000] { for frames in [128, 256] { for ratio in [0.5, 0.84, 1.5] { for repetition in 0..3 {
        let report = keylock_showload_tests::workload_observed(rate, frames, ratio, 2048, true);
        for (name, pass) in report["checks"].as_object().unwrap() { assert_eq!(pass, true, "{name}"); }
        for metric in ["allocations", "frees", "rejected_commands"] { assert_eq!(report["metrics"][metric], 0, "{metric}"); }
        let deadline = frames as u64 * 1_000_000_000 / rate as u64;
        for metric in ["callback_wall_ns", "render_cpu_ns", "full_callback_thread_cpu_ns"] {
            let mut times: Vec<u64> = report["samples"][metric].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
            times.sort_unstable();
            let p50 = times[(times.len() * 50).div_ceil(100) - 1];
            let p99 = times[(times.len() * 99).div_ceil(100) - 1];
            let maximum = *times.last().unwrap();
            println!("history-active rate={rate} frames={frames} ratio={ratio} {metric} p99={p99} max={maximum} deadline={deadline}");
            if metric == "callback_wall_ns" {
                assert!(p99 <= deadline && maximum <= deadline * 2 && p99 - p50 <= deadline / 2);
            } else { assert!(p99 <= deadline * 3 / 4); }
        }
        reports.push(serde_json::json!({"rate":rate,"frames":frames,"ratio":ratio,"repetition":repetition,"measurement":report}));
    }
    } } }
    if let Some(path) = std::env::var_os("OMATAINER_HISTORY_PROTOTYPE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    }
}

#[test]
#[ignore = "local release-profile four-source history with actual persistence worker"]
fn retiring_history_with_worker_show_budget() {
    use crate::{engine::{self, audio::OutputCallback, performance_workload_tests, test_alloc, Command, FxKind}, performance_history::{storage, worker::{Job, Worker}}};
    use std::time::{Duration, Instant};
    assert!(!cfg!(debug_assertions), "run with --release");
    let mut reports = Vec::new();
    for repetition in 0..3 {
        let root = std::env::temp_dir().join(format!("omat-history-show-{}", storage::new_id().unwrap()));
        let (engine, mut rt) = performance_workload_tests::prepared_at("hybrid", 96_000);
        rt.fx_wet = [0.5; 3]; rt.fx_kind = [FxKind::Echo, FxKind::Reverb, FxKind::Filter];
        let replacement = rt.decks[0].audio.clone().unwrap();
        for deck in &mut rt.decks { deck.keylock = true; deck.pitch_range = 2; deck.pitch = 0.34; deck.rate = 0.84; deck.target_rate = 0.84; deck.sync = false; }
        let mut callback = OutputCallback::new(rt, 2);
        let mut output = [0.0_f32; 256];
        for _ in 0..128 { callback.render(&mut output); }
        let mut worker = Worker::start(root.clone(), engine.performance_history.clone(), engine.cmd.performance().clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !worker.poll().ready { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(1)); }
        worker.submit(Job::Start).unwrap();
        while worker.poll().active.is_none() { assert!(Instant::now() < deadline); callback.render(&mut output); std::thread::sleep(Duration::from_millis(1)); }
        for deck in 0..2 {
            engine.send(Command::DeckAudio { deck, audio: replacement.clone() }).unwrap();
            engine.send(Command::DeckLoop { deck, beats: 16.0 }).unwrap();
            engine.send(Command::DeckPlay { deck }).unwrap();
        }
        let mut wall = Vec::with_capacity(2048); let mut cpu = Vec::with_capacity(2048); let mut full_cpu = Vec::with_capacity(2048);
        for _ in 0..2048 {
            let before = engine::audio_metrics::thread_cpu_ns().unwrap();
            let heap = test_alloc::measure(|| callback.render(&mut output));
            full_cpu.push(engine::audio_metrics::thread_cpu_ns().unwrap() - before);
            assert_eq!(heap, test_alloc::Counts::default());
            let measurement = engine.cmd.audio_metrics().last_callback.unwrap();
            wall.push(measurement.elapsed_ns); cpu.push(measurement.render_cpu_ns.unwrap());
            assert!(output.iter().all(|v| v.is_finite()));
            worker.poll(); std::thread::yield_now();
        }
        worker.submit(Job::End).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            callback.render(&mut output); let view = worker.poll();
            if view.active.is_none() && view.durable { break; }
            assert!(Instant::now() < deadline, "worker end/save timeout"); std::thread::sleep(Duration::from_millis(1));
        }
        let session = worker.view().selected.as_ref().unwrap();
        assert!(!session.incomplete && session.dropped_observation_frames == 0);
        assert_eq!(session.entries.iter().filter(|e| e.measured_seconds() > 0.0).count(), 4, "two retiring sources and two current sources must be measured");
        let deadline_ns = 128_u64 * 1_000_000_000 / 96_000;
        for (name, values) in [("callback_wall_ns", &wall), ("render_cpu_ns", &cpu), ("full_callback_thread_cpu_ns", &full_cpu)] {
            let mut times = values.clone(); times.sort_unstable();
            let p50 = times[(times.len() * 50).div_ceil(100) - 1]; let p99 = times[(times.len() * 99).div_ceil(100) - 1]; let max = *times.last().unwrap();
            println!("history-retiring repetition={repetition} {name} p99={p99} max={max} deadline={deadline_ns}");
            if name == "callback_wall_ns" { assert!(p99 <= deadline_ns && max <= deadline_ns * 2 && p99 - p50 <= deadline_ns / 2); }
            else { assert!(p99 <= deadline_ns * 3 / 4); }
        }
        reports.push(serde_json::json!({"repetition":repetition,"rate":96000,"frames":128,"wall":wall,"render_cpu":cpu,"full_cpu":full_cpu,
            "entries":session.entries.iter().map(|e|e.measured_seconds()).collect::<Vec<_>>(),"incomplete":session.incomplete,"durable":worker.view().durable}));
        drop(worker);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Ok(store) = storage::Store::open(root.clone()) { drop(store); break; }
            assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(1));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
    if let Some(path) = std::env::var_os("OMATAINER_HISTORY_WORKER_REPORT") { std::fs::write(path, serde_json::to_vec_pretty(&reports).unwrap()).unwrap(); }
}
