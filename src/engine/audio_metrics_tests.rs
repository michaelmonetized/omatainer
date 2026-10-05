use super::*;
use crate::engine::audio_metrics::{thread_cpu_ns, Telemetry};
use crate::engine::{test_alloc, Command, CommandPort, Snapshot};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn fixture(sr: u32, channels: usize) -> (OutputCallback, CommandPort) {
    let (commands, rx) = CommandPort::channel(256);
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    let rt = RtEngine::new(sr as f32, rx, snapshot);
    (OutputCallback::new(rt, channels), commands)
}

#[test]
fn totals_include_control_publication_and_conversion_delays_but_cpu_excludes_sleep() {
    let (mut callback, commands) = fixture(48000, 2);
    let mut out = [0i16; 960]; // 10 ms budget
    callback.render(&mut out);
    commands.send(Command::Play).unwrap();
    callback.rt.telemetry_delays[0] = Duration::from_millis(15);
    callback.rt.telemetry_delays[2] = Duration::from_millis(20);
    callback.conversion_delay = Duration::from_millis(10);
    callback.rt.frames_done = 48000 / 60 - 100;
    let observed = Instant::now();
    callback.render(&mut out);
    let outside = observed.elapsed();
    let metrics = commands.audio_metrics();
    let sample = metrics.last_callback.unwrap();
    assert!(
        sample.elapsed_ns >= 45_000_000,
        "missing phase delay: {sample:?}"
    );
    assert!(sample.elapsed_ns <= outside.as_nanos() as u64);
    assert_eq!(sample.budget_ns, 10_000_000);
    assert_eq!(sample.overrun_ns, sample.elapsed_ns - sample.budget_ns);
    assert_eq!(metrics.callbacks, 2);
    assert!(metrics.deadline_overruns >= 1);
    assert!(
        sample.render_cpu_ns.unwrap() < sample.elapsed_ns - 35_000_000,
        "CPU includes injected waits: {sample:?}"
    );
    assert!(callback.rt.playing);
    assert_eq!(callback.rt.command_stats.received, 1);
    assert_eq!(metrics.dropped_buffers, None);
    callback.rt.telemetry_delays = [Duration::ZERO; 3];
    callback.conversion_delay = Duration::ZERO;
    callback.rt.publish_for_test();
    assert_eq!(
        callback
            .rt
            .snap
            .lock()
            .audio
            .last_callback
            .unwrap()
            .elapsed_ns,
        sample.elapsed_ns
    );
}

#[test]
fn render_clock_counts_thread_cpu_not_render_wall_waits() {
    assert!(thread_cpu_ns().is_some());
    let (mut callback, commands) = fixture(48000, 2);
    let mut out = [0f32; 96];
    callback.render(&mut out);
    callback.rt.telemetry_delays[1] = Duration::from_millis(30);
    callback.render(&mut out);
    let sample = commands.audio_metrics().last_callback.unwrap();
    assert!(sample.elapsed_ns >= 30_000_000);
    assert!(
        sample.render_cpu_ns.unwrap() < sample.elapsed_ns - 20_000_000,
        "sleep counted as CPU: {sample:?}"
    );
}

#[test]
fn budget_uses_real_frames_rate_channels_and_all_output_formats() {
    for sr in [44100, 48000, 96000] {
        for channels in [1, 2, 6] {
            let (mut callback, commands) = fixture(sr, channels);
            let frames = 257;
            let mut floats = vec![0f32; frames * channels];
            let mut signed = vec![0i16; frames * channels];
            let mut unsigned = vec![0u16; frames * channels];
            callback.render(&mut floats);
            callback.render(&mut signed);
            callback.render(&mut unsigned);
            let metrics = commands.audio_metrics();
            assert_eq!(metrics.callbacks, 3);
            assert_eq!(
                metrics.last_callback.unwrap().budget_ns,
                (frames as u64 * 1_000_000_000) / sr as u64
            );
        }
    }
}

#[test]
fn fixed_metrics_and_warmed_callback_do_not_allocate_or_wait_for_readers() {
    let (mut callback, commands) = fixture(48000, 2);
    let mut out = [0f32; 512];
    callback.render(&mut out);
    callback.rt.publish_for_test();
    let snap = callback.rt.snap.clone();
    let held = snap.lock();
    // Cross many real periodic publication boundaries while snapshot worker is
    // stalled by a reader. Metrics remain accessible through the command port.
    let counts = test_alloc::measure(|| {
        for _ in 0..60 {
            callback.render(&mut out);
            let _ = commands.audio_metrics();
        }
    });
    drop(held);
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    assert_eq!(commands.audio_metrics().callbacks, 61);
}

#[test]
fn stream_errors_are_counted_without_logging_or_inventing_dropped_buffers() {
    let metrics = Telemetry::default();
    let device_lost = cpal::StreamError::DeviceNotAvailable;
    let other = cpal::StreamError::BackendSpecific {
        err: cpal::BackendSpecificError {
            description: "test backend failure".into(),
        },
    };
    let counts = test_alloc::measure(|| {
        metrics.error(&device_lost);
        metrics.error(&other);
    });
    assert_eq!((counts.allocations, counts.frees), (0, 0));
    let stats = metrics.read();
    assert_eq!(stats.backend_errors, 2);
    assert_eq!(stats.device_lost, 1);
    assert_eq!(stats.dropped_buffers, None);
    assert!(stats.last_callback.is_none());
}

#[test]
fn coherent_samples_and_monotonic_counters_survive_concurrent_readers() {
    let telemetry = Arc::new(Telemetry::default());
    let writer = telemetry.clone();
    let thread = std::thread::spawn(move || {
        for n in 1..=100_000u64 {
            writer.record(
                Duration::from_nanos(n * 3),
                n as usize,
                1_000_000_000,
                Some(n * 2),
            );
        }
    });
    let mut previous = 0;
    while !thread.is_finished() {
        let metrics = telemetry.read();
        assert!(metrics.callbacks >= previous);
        previous = metrics.callbacks;
        if let Some(sample) = metrics.last_callback {
            assert_eq!(sample.elapsed_ns, sample.budget_ns * 3);
            assert_eq!(sample.render_cpu_ns, Some(sample.budget_ns * 2));
            assert_eq!(sample.overrun_ns, sample.budget_ns * 2);
        }
    }
    thread.join().unwrap();
    let final_state = telemetry.read();
    assert_eq!(final_state.callbacks, 100_000);
    assert_eq!(final_state.deadline_overruns, 100_000);
    assert_eq!(final_state.max_elapsed_ns, 300_000);
    assert_eq!(final_state.max_overrun_ns, 200_000);
}

#[test]
fn live_engine_and_ipc_status_read_completed_metrics_without_waiting_for_snapshot_publication() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let (engine, rt) = crate::engine::Engine::headless_for_test(48000, 48);
    let mut callback = OutputCallback::new(rt, 2);
    callback.render(&mut [0f32; 128]); // far short of a periodic snapshot interval
    assert_eq!(engine.snapshot().audio.callbacks, 1);
    let command_port = engine.cmd.clone();
    let snapshot = engine.snap.clone();
    let (mut client, server) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let server = std::thread::spawn(move || crate::handle_client(server, command_port, snapshot));
    client.write_all(b"{\"op\":\"status\"}\n").unwrap();
    let mut client = BufReader::new(client);
    let mut response = String::new();
    client.read_line(&mut response).unwrap();
    let json: serde_json::Value = serde_json::from_str(&response).unwrap();
    assert_eq!(json["audio"]["callbacks"], 1);
    assert_eq!(json["audio"]["last_callback"]["budget_ns"], 1_333_333);
    assert!(json["audio"]["dropped_buffers"].is_null());
    drop(client);
    server.join().unwrap().unwrap();
}
