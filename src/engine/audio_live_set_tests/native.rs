use super::*;
use cpal::traits::{DeviceTrait, StreamTrait};

struct Capture {
    samples: Vec<f32>,
    stamps: Vec<(usize, u64)>,
    returned: crossbeam_channel::Sender<(Vec<f32>, Vec<(usize, u64)>)>,
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.returned.try_send((std::mem::take(&mut self.samples), std::mem::take(&mut self.stamps)));
    }
}

fn carrier(frequencies: [f64; 2]) -> Arc<crate::engine::dsp::Sample> {
    let data = (0..4 * 1024 * 1024).flat_map(|frame| frequencies.map(|frequency|
        (std::f64::consts::TAU * frequency * frame as f64 / 48000.0).sin() as f32 * 0.04)).collect();
    Arc::new(crate::engine::dsp::Sample { spectrum: None, name: "Private continuous transition carrier".into(),
        sr: 48000, ch: 2, data, peaks: Arc::new(vec![]), bpm: 120.0, path: String::new() })
}

fn wait(engine: &crate::engine::Engine, output: &AudioOut, directory: &std::path::Path, phase: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !ready() {
        let status = output.handle.status();
        if status.phase == owner::Phase::Offline || Instant::now() >= deadline {
            let receipt = serde_json::json!({"status":"failed","waiting_for":phase,"output_phase":format!("{:?}",status.phase),
                "message":status.message,"audio":engine.cmd.audio_metrics(),"playing":engine.snapshot().playing});
            std::fs::write(directory.join(format!("failure-{phase}.json")), serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
            panic!("Native transition acknowledgment failed: {receipt}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn tone(samples: &[f32], channel: usize, frequency: f64) -> f64 {
    let frames = samples.len() / 2;
    let mut real = 0.0;
    let mut imaginary = 0.0;
    for (frame, pair) in samples.chunks_exact(2).enumerate() {
        let angle = std::f64::consts::TAU * frequency * frame as f64 / 44100.0;
        real += f64::from(pair[channel]) * angle.cos();
        imaginary += f64::from(pair[channel]) * angle.sin();
    }
    2.0 * real.hypot(imaginary) / frames.max(1) as f64
}

fn windows(samples: &[f32]) -> Vec<f64> {
    samples.chunks_exact(882 * 2).map(|window|
        (window.iter().map(|sample| f64::from(*sample).powi(2)).sum::<f64>() / window.len() as f64).sqrt()).collect()
}

fn workers() -> Vec<serde_json::Value> {
    use std::io::Read;
    std::fs::read_dir("/proc/self/task").unwrap().take(256).filter_map(|entry| {
        let entry = entry.ok()?;
        let mut text = String::new();
        std::fs::File::open(entry.path().join("status")).ok()?.take(16 * 1024).read_to_string(&mut text).ok()?;
        let name = text.lines().find_map(|line| line.strip_prefix("Name:\t"))?;
        if !name.starts_with("cpal_alsa_") { return None; }
        let cpus = text.lines().find_map(|line| line.strip_prefix("Cpus_allowed_list:\t"))?;
        Some(serde_json::json!({"tid":entry.file_name().to_string_lossy(),"name":name,"allowed_cpus":cpus}))
    }).collect()
}

#[test]
fn continuous_capture_check_refuses_a_missing_block_and_requires_each_carrier_channel() {
    let samples: Vec<f32> = (0..44100).flat_map(|frame| [381.0, 583.0].map(|frequency|
        (std::f64::consts::TAU * frequency * frame as f64 / 44100.0).sin() as f32 * 0.01)).collect();
    let baseline = windows(&samples);
    assert!(baseline.iter().all(|rms| *rms > 0.005));
    assert!(tone(&samples, 0, 381.0) > 0.0099 && tone(&samples, 1, 583.0) > 0.0099);
    assert!(tone(&samples, 1, 381.0) < 1e-6 && tone(&samples, 0, 583.0) < 1e-6);
    let mut broken = samples;
    broken[22050..22050 + 3528].fill(0.0);
    assert!(windows(&broken).iter().any(|rms| *rms < 0.001));
}

#[test]
#[ignore = "Requires exclusive original NS7 master XLR outputs wired to Peavey 7/8 and OMATAINER_NATIVE_TRANSITION_DIR under the project; emits quiet continuous carriers"]
fn native_ns7_large_live_set_transition_has_captured_master_audio() {
    native_transition(false);
}

#[test]
#[ignore = "Requires exclusive NS7 master XLR outputs wired to Peavey 7/8 and OMATAINER_NATIVE_TRANSITION_DIR under the project; verifies routed 20 ms graph delay and emits quiet carriers"]
fn native_ns7_routed_large_live_set_transition_has_captured_master_audio() {
    native_transition(true);
}

fn native_transition(routed: bool) {
    let directory = std::path::PathBuf::from(std::env::var_os("OMATAINER_NATIVE_TRANSITION_DIR").expect("Choose a private receipt directory"));
    assert!(directory.is_absolute() && directory.starts_with(env!("CARGO_MANIFEST_DIR")));
    std::fs::create_dir(&directory).unwrap();
    let baseline_memory = memory();
    let mut state = large_state();
    if routed {
        use crate::engine::audio::routing::model::*;
        state.routing = Some(Arc::new(Model { latency: Some(LatencyConfiguration { reports: vec![LatencyReport {
            group: Group::Deck(0), external_micros: 0, processing_micros: 20_000,
        }], ..Default::default() }), ..Default::default() }));
    }
    state.master = 0.5;
    state.xfader = 0.0;
    for track in &mut state.tracks { track.gain = 0.002; }
    let mut first = Prepared::from_state(state.clone(), vec![carrier([381.0, 583.0])], 44100).unwrap();
    first.rt.playing = true;
    first.rt.resume_project_clips();
    for deck in &mut first.rt.decks { deck.playing = true; }
    let (engine, mut rt) = crate::engine::Engine::headless_for_test(44100, 256);
    first.swap_into(&mut rt);
    drop(first);
    let input_plan = config::Plan { backend: "ALSA".into(), graph: Default::default(), device: "hw:CARD=CODEC,DEV=0".into(),
        channels: 2, rate: 44100, format: cpal::SampleFormat::I16, buffer: Some(512), warning: None };
    let input_device = config::select_input_exact(&input_plan).unwrap();
    let (returned, receive) = crossbeam_channel::bounded(1);
    let mut capture = Capture { samples: Vec::with_capacity(44100 * 2 * 40), stamps: Vec::with_capacity(20000), returned };
    let origin = Instant::now();
    let input_fault = Arc::new(AtomicBool::new(false));
    let fault = input_fault.clone();
    let input = input_device.build_input_stream(input_plan.config(), move |data: &[i16], _| {
        if data.len() % 2 != 0 || capture.stamps.len() == capture.stamps.capacity() { fault.store(true, Ordering::Release); return; }
        let remaining = capture.samples.capacity() - capture.samples.len();
        if data.len() > remaining { fault.store(true, Ordering::Release); return; }
        capture.stamps.push((capture.samples.len() / 2, origin.elapsed().as_nanos() as u64));
        capture.samples.extend(data.iter().map(|sample| f32::from(*sample) / 32768.0));
    }, {
        let fault = input_fault.clone();
        move |error| { if error.kind() != cpal::ErrorKind::RealtimeDenied { fault.store(true, Ordering::Release); } }
    }, None).unwrap();
    input.play().unwrap();
    let output_settings = crate::preferences::Audio { backend: Some("ALSA".into()), device: Some("hw:CARD=NS7,DEV=0".into()),
        sample_rate: Some(44100), channels: Some(4), format: Some(crate::preferences::AudioFormat::I32), buffer_frames: Some(512), ..Default::default() };
    let output = start_with_settings(rt, &output_settings).unwrap();
    wait(&engine, &output, &directory, "warmup", || engine.cmd.audio_metrics().callbacks >= 100);
    let warmed = engine.cmd.audio_metrics();
    let native_workers = workers();
    let measured_start = origin.elapsed().as_nanos() as u64;
    let mut trials = Vec::new();
    let mut process_memory = Vec::new();
    for trial in 0..3 {
        let preload_start = origin.elapsed().as_nanos() as u64;
        let frequencies = if trial % 2 == 0 { [473.0, 689.0] } else { [381.0, 583.0] };
        let next = Prepared::from_state(state.clone(), vec![carrier(frequencies)], 44100).unwrap();
        process_memory.push(memory());
        let handle = engine.project.live_sets();
        let namespace = engine.snapshot().session.as_ref().unwrap().namespace;
        let control = handle.stage(handle.reserve().unwrap(), next, namespace, Arc::new(AtomicBool::new(false))).unwrap();
        wait(&engine, &output, &directory, "preload", || control.ready());
        let prepared_at = origin.elapsed().as_nanos() as u64;
        assert!(control.cue_available.load(Ordering::Acquire));
        let initial_priming_seconds = control.priming_seconds(44100);
        if routed {
            assert_eq!(initial_priming_seconds, 0.02);
            assert!(control.transition(&engine.project, engine.project.revision(), 0.01).unwrap_err().contains("processing history"));
            assert!(control.ready());
        }
        control.preview.store(true, Ordering::Release);
        std::thread::sleep(Duration::from_secs(1));
        wait(&engine, &output, &directory, "cue-priming", || control.priming_seconds(44100) == 0.0);
        let cue_audio = engine.cmd.audio_metrics();
        control.preview.store(false, Ordering::Release);
        std::thread::sleep(Duration::from_millis(100));
        let transition_at = origin.elapsed().as_nanos() as u64;
        if let Err(error) = control.transition(&engine.project, engine.project.revision(), 1.0) {
            let status = output.handle.status();
            let receipt = serde_json::json!({"status":"failed","trial":trial,"waiting_for":"fade admission","error":error,
                "output_phase":format!("{:?}",status.phase),"message":status.message,"audio":engine.cmd.audio_metrics(),
                "playing":engine.snapshot().playing,"performance":engine.cmd.performance().status()});
            std::fs::write(directory.join(format!("failure-fade-admission-{trial}.json")),serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
            panic!("Native fade admission failed: {receipt}");
        }
        let mut applied = false;
        let mut retired = None;
        wait(&engine, &output, &directory, "retirement", || {
            applied |= handle.applied().is_some();
            if retired.is_none() { retired = handle.retire(); }
            applied && retired.is_some()
        });
        assert!(retired.unwrap().is_ok());
        let settled_at = origin.elapsed().as_nanos() as u64;
        let settled_audio = engine.cmd.audio_metrics();
        std::thread::sleep(Duration::from_secs(1));
        trials.push(serde_json::json!({"trial":trial,"incoming_hz":frequencies,"preload_start_ns":preload_start,
            "ready_ns":prepared_at,"transition_requested_ns":transition_at,"settled_ns":settled_at,"fade_seconds":1.0,
            "cue_admitted":true,"initial_priming_seconds":initial_priming_seconds,"short_unprimed_fade_refused":routed,"applied":applied,"worker_retired":true,
            "cue_audio":cue_audio,"settled_audio":settled_audio}));
    }
    let measured_end = origin.elapsed().as_nanos() as u64;
    let metrics = engine.cmd.audio_metrics();
    drop(output);
    owner::finish_shutdown();
    drop(input);
    let (samples, stamps) = receive.recv_timeout(Duration::from_secs(5)).unwrap();
    let frame_at = |time: u64| stamps.iter().find(|(_, ns)| *ns >= time).map(|(frame, _)| *frame).unwrap_or(samples.len() / 2);
    let first_frame = frame_at(measured_start + 100_000_000);
    let last_frame = frame_at(measured_end);
    let rms = windows(&samples[first_frame * 2..last_frame * 2]);
    let mut sorted = rms.clone();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let minimum = rms.iter().copied().fold(f64::INFINITY, f64::min);
    let final_start = frame_at(measured_end.saturating_sub(900_000_000));
    let final_samples = &samples[final_start * 2..last_frame * 2];
    let final_tones = [tone(final_samples, 0, 473.0), tone(final_samples, 1, 689.0)];
    let wrong_tones = [tone(final_samples, 1, 473.0), tone(final_samples, 0, 689.0)];
    let input_failed = input_fault.load(Ordering::Acquire);
    let gaps = rms.iter().filter(|rms| **rms < median * 0.1).count();
    let qualified = !input_failed && gaps == 0 && median > 0.0001 && samples.iter().all(|value| value.is_finite() && value.abs() < 0.999)
        && final_tones.iter().all(|value| *value > 0.0001) && (0..2).all(|channel| final_tones[channel] > wrong_tones[channel] * 5.0)
        && metrics.backend_errors == warmed.backend_errors && metrics.device_lost == warmed.device_lost
        && metrics.xruns == warmed.xruns && metrics.deadline_overruns == warmed.deadline_overruns;
    let receipt = serde_json::json!({"schema":1,"status":if qualified {"pass"} else {"failed"},"device":"original NS7, four-channel ALSA, 44.1 kHz, 512-frame requested buffer",
        "physical_return":"Master XLR L/R to owner-restored Peavey PV8 USB 7/8, stereo CODEC input","routed":routed,"reported_deck_processing_micros":if routed {20000} else {0},"stored_tracks":64,"stored_notes_per_graph":32768,
        "launched_clip_tracks_per_graph":8,"loaded_decks_per_graph":2,"resident_pcm_bytes_per_graph":32*1024*1024,
        "baseline_memory":baseline_memory,"preloaded_memory":process_memory,"final_memory":memory(),"trials":trials,
        "warmup_audio":warmed,"final_audio":metrics,"native_workers":native_workers,"carrier_peak":0.04,"capture_frames":samples.len()/2,"input_failed":input_failed,
        "measured_ns":measured_end-measured_start,"capture_window_frames":882,"quiet_windows":gaps,"median_window_rms":median,"minimum_window_rms":minimum,
        "final_expected_tone_amplitudes":final_tones,"final_opposite_tone_amplitudes":wrong_tones,
        "scope":"Actual production native output owner and three complete graph fades, independent cue admission, captured stereo master continuity in 20 ms windows. Startup metrics retained separately. No physical headphone cue capture, listening, display timing, all-track maximum-polyphony or stage-duration claim."});
    std::fs::write(directory.join("receipt.json"), serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    std::fs::write(directory.join("peavey-return.f32le"), samples.iter().flat_map(|value| value.to_le_bytes()).collect::<Vec<_>>()).unwrap();
    assert!(qualified, "Native transition and captured return results retained: {receipt}");
}
