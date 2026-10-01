use crate::engine::keylock_showload_tests;

#[test]
#[ignore = "local release-profile active-history timing, never ordinary CI"]
fn prototype_active_history_show_budget() {
    assert!(!cfg!(debug_assertions), "run with --release");
    let mut reports = Vec::new();
    for (rate, frames, ratio) in [(44_100, 128, 0.5), (48_000, 128, 0.84), (96_000, 128, 1.5)] {
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
        reports.push(serde_json::json!({"rate":rate,"frames":frames,"ratio":ratio,"measurement":report}));
    }
    if let Some(path) = std::env::var_os("OMATAINER_HISTORY_PROTOTYPE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
    }
}
