use super::*;
use crate::engine::{live_set, project::{self, Prepared}, session, test_alloc, ClipKind, MidiNote};
use std::time::{Duration, Instant};

fn source(value: f32) -> Arc<crate::engine::dsp::Sample> {
    Arc::new(crate::engine::dsp::Sample { name: "resident 32 MiB".into(), sr: 48_000, ch: 2,
        data: vec![value; 8 * 1024 * 1024], peaks: Arc::new(vec![]), bpm: 120.0, path: String::new() })
}

fn large_state() -> project::State {
    let mut seed = Prepared::empty(48_000).unwrap().rt;
    let owner = seed.project.clone();
    let worker = std::thread::spawn(move || owner.capture(&AtomicBool::new(false)).unwrap());
    let deadline = Instant::now() + Duration::from_secs(10);
    while !worker.is_finished() {
        seed.process(&mut []);
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    let mut state = worker.join().unwrap().state;
    let track = state.tracks[0].clone();
    state.tracks = (0..64).map(|index| {
        let mut track = track.clone();
        track.name = format!("Track {index}");
        track.drums = [0; 6];
        track.kind = 1;
        track.launch = (index < 8).then_some(project::Launch { scene: 0, start_beat: 0.0, looping: true });
        track.clips[0].kind = ClipKind::Midi;
        track.clips[0].bars = 32.0;
        track.clips[0].notes = (0..512).map(|note| MidiNote { id: crate::engine::midi_edit::NoteId::new(),
            muted: false, pitch: 48 + (note % 24) as u8, start: 0.0, len: 64.0, vel: 40,
            channel: 0, release_vel: 64, source_timing: None }).collect();
        track
    }).collect();
    state.session = Some(session::Layout::fresh(state.tracks.iter().map(|track| track.name.clone()), state.scene_fx.len()));
    for bank in &mut state.banks {
        bank.media = [None; 16];
        bank.settings.as_mut().unwrap().slots = std::array::from_fn(|_| crate::sampler_bank::Slot::default());
    }
    state.builtin = [None; 2];
    for deck in &mut state.decks { deck.audio = Some(0); }
    state.master = 0.02;
    state
}

fn memory() -> serde_json::Value {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let value = |name: &str| status.lines().find_map(|line| line.strip_prefix(name))
        .and_then(|line| line.split_whitespace().next()).and_then(|text| text.parse::<u64>().ok());
    serde_json::json!({"resident_kib":value("VmRSS:"),"process_high_water_kib":value("VmHWM:")})
}

#[test]
#[ignore = "Explicit optimized transition timing and process-memory qualification; writes an immutable receipt"]
fn large_live_set_transition_qualification() {
    assert!(!cfg!(debug_assertions), "Use the retained optimized test binary");
    let report_path = std::env::var_os("OMATAINER_LIVE_SET_REPORT").expect("Choose an explicit project-owned receipt path");
    let path = std::path::PathBuf::from(report_path);
    assert!(path.is_absolute() && path.starts_with(env!("CARGO_MANIFEST_DIR")));
    let baseline_memory = memory();
    let state = large_state();
    let mut trials = Vec::new();
    for frames in [128usize, 256, 1024] {
        for trial in 0..3 {
            let prepare_started = Instant::now();
            let mut current = Prepared::from_state(state.clone(), vec![source(0.1)], 48_000).unwrap();
            current.rt.playing = true;
            current.rt.resume_project_clips();
            for deck in &mut current.rt.decks { deck.playing = true; }
            let next = Prepared::from_state(state.clone(), vec![source(-0.1)], 48_000).unwrap();
            let preparation_wall_ns = prepare_started.elapsed().as_nanos() as u64;
            let handle = current.rt.project.live_sets();
            let control = handle.stage(handle.reserve().unwrap(), next, current.rt.session.namespace, Arc::new(AtomicBool::new(false))).unwrap();
            let (returned, retired) = crossbeam_channel::bounded(1);
            let mut callback = OutputCallback::managed(current.rt, 4, returned, Arc::new(AtomicBool::new(true)), Arc::new(AtomicBool::new(false)));
            let mut output = vec![0.0f32; frames * 4];
            for _ in 0..64 { callback.render(&mut output); }
            assert!(control.ready());
            let owner = callback.renderer_for_test().project.clone();
            control.transition(&owner, owner.revision(), 30.0).unwrap();
            let mut wall = Vec::with_capacity(256);
            let mut cpu = Vec::with_capacity(256);
            let mut allocations = 0;
            let mut frees = 0;
            let mut finite = true;
            let mut peak = 0.0f32;
            for _ in 0..256 {
                let start = Instant::now();
                let cpu_start = crate::engine::audio_metrics::thread_cpu_ns().unwrap();
                let counts = test_alloc::measure(|| callback.render(&mut output));
                cpu.push(crate::engine::audio_metrics::thread_cpu_ns().unwrap() - cpu_start);
                wall.push(start.elapsed().as_nanos() as u64);
                allocations += counts.allocations;
                frees += counts.frees;
                for value in &output { finite &= value.is_finite(); peak = peak.max(value.abs()); }
            }
            let budget = frames as u64 * 1_000_000_000 / 48_000;
            let maximum_wall = *wall.iter().max().unwrap();
            let maximum_cpu = *cpu.iter().max().unwrap();
            let process_memory = memory();
            callback.renderer_mut_for_test().performance.request_safety(crate::engine::performance::Safety::Silence);
            callback.render(&mut output);
            assert!(handle.applied().is_some());
            assert!(matches!(handle.retire(), Some(Err(project::Error::Protected(crate::engine::performance::Error::Recovery)))));
            drop(callback);
            drop(retired.try_recv().unwrap());
            trials.push(serde_json::json!({"frames":frames,"trial":trial,"callbacks":256,"channels":4,
                "budget_ns":budget,"preparation_wall_ns":preparation_wall_ns,"maximum_wall_ns":maximum_wall,
                "maximum_cpu_ns":maximum_cpu,"callback_wall_ns":wall,"callback_cpu_ns":cpu,
                "rust_allocations":allocations,"rust_frees":frees,"finite":finite,"peak":peak,
                "process_memory":process_memory,"deadline_pass":maximum_wall<=budget && maximum_cpu<=budget,
                "heap_pass":allocations==0 && frees==0}));
        }
    }
    let deadline_pass = trials.iter().all(|trial| trial["deadline_pass"] == true);
    let heap_pass = trials.iter().all(|trial| trial["heap_pass"] == true);
    let report = serde_json::json!({"schema":1,"scope":"Actual production four-channel OutputCallback, two complete graphs; local optimized timing only",
        "stored_tracks":64,"stored_notes_per_graph":32768,"active_clip_tracks_per_graph":8,"loaded_decks_per_graph":2,
        "distinct_resident_pcm_bytes_per_graph":32*1024*1024,"warmup_callbacks":64,"measured_callbacks_per_trial":256,
        "hardware_xruns":null,"physical_hardware_test":false,"baseline_process_memory":baseline_memory,
        "final_process_memory":memory(),"deadline_pass":deadline_pass,"heap_pass":heap_pass,"trials":trials,
        "limits":"No maximum-polyphony or all-64-tracks-active qualification. Process high water includes setup and prior trials; preparation timing excludes container file I/O. No hardware or listening claim."});
    let file = std::fs::OpenOptions::new().create_new(true).write(true).open(path).unwrap();
    serde_json::to_writer_pretty(file, &report).unwrap();
    assert!(heap_pass, "Callback Rust heap activity is retained in the receipt");
    assert!(deadline_pass, "Callback deadline failures are retained in the receipt");
}
