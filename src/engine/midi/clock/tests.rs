use super::*;
use crate::engine::{test_alloc, Command, Engine};
use std::sync::Mutex;

fn configured() -> (Arc<Shared>, Runtime, crossbeam_channel::Receiver<Event>) {
    let shared = Arc::new(Shared::default());
    let receiver = shared.receiver.lock().take().unwrap();
    shared.generation.store(1, Release);
    shared.enabled.store(true, Release);
    let runtime = Runtime::new(shared.clone());
    (shared, runtime, receiver)
}
#[test]
fn sample_deadlines_follow_independent_rational_tempo_oracles_at_every_rate_without_heap_work() {
    for rate in [8000, 44100, 48000, 96000] {
        for bpm in [40_u32, 73, 120, 137, 180, 240] {
            for compensation in [-50, 0, 50] {
                let (shared, mut runtime, events) = configured();
                shared.compensation_ms.store(compensation, Release);
                let frames = (u64::from(rate) * 180 / u64::from(bpm)) as usize;
                let base = 1_000_000_000_u64;
                runtime.begin_at(rate, frames, base, true);
                assert_eq!(
                    test_alloc::measure(|| {
                        for frame in 0..frames {
                            let beat = frame as f64 * f64::from(bpm) / (f64::from(rate) * 60.0);
                            let next =
                                (frame + 1) as f64 * f64::from(bpm) / (f64::from(rate) * 60.0);
                            runtime.frame(beat, next, true, 0, frame);
                        }
                    }),
                    Default::default()
                );
                let messages: Vec<_> = events.try_iter().collect();
                assert_eq!(messages[0].message, Message::Start);
                let clocks: Vec<_> = messages
                    .iter()
                    .filter(|event| event.message == Message::Clock)
                    .collect();
                assert_eq!(clocks.len(), 72);
                for (tick, event) in clocks.iter().enumerate() {
                    let numerator = tick as u64 * u64::from(rate) * 60;
                    let denominator = u64::from(bpm) * 24;
                    let frame = numerator.div_ceil(denominator);
                    let expected = base.saturating_add_signed(i64::from(compensation) * 1_000_000)
                        + frame * 1_000_000_000 / u64::from(rate);
                    assert_eq!(
                        event.deadline_ns, expected,
                        "rate {rate}, bpm {bpm}, tick {tick}"
                    );
                }
                assert!(shared.counters().backend_timing);
                assert_eq!(shared.counters().scheduled, 72);
            }
        }
    }
}
#[test]
fn start_continue_seek_stop_count_in_and_invalid_positions_have_exact_order_and_no_callback_heap() {
    let (shared, mut runtime, events) = configured();
    runtime.begin_at(48000, 100, 1_000_000_000, true);
    runtime.frame(0.0, 0.0, false, 0, 0);
    assert!(events.try_recv().is_err());
    assert_eq!(
        test_alloc::measure(|| {
            runtime.frame(0.0, 1.0 / 24000.0, true, 0, 1);
            runtime.frame(1.0 / 24000.0, 1.0 / 24000.0, false, 1, 2);
            runtime.frame(5.3, 5.3 + 1.0 / 24000.0, true, 2, 3);
            runtime.frame(1.0, 1.0 + 1.0 / 24000.0, true, 3, 4);
            runtime.frame(1.0 + 1.0 / 24000.0, 1.0 + 1.0 / 24000.0, false, 4, 5);
        }),
        Default::default()
    );
    let messages: Vec<_> = events.try_iter().map(|event| event.message).collect();
    assert_eq!(
        messages,
        [
            Message::Start,
            Message::Clock,
            Message::Stop,
            Message::Position(21),
            Message::Continue,
            Message::Stop,
            Message::Position(4),
            Message::Continue,
            Message::Clock,
            Message::Stop
        ]
    );
    runtime.frame(5000.0, 5000.01, true, 5, 6);
    assert_eq!(shared.counters().error, Some(Fault::Position.text()));
    assert!(!shared.counters().enabled);
}
#[test]
fn full_queue_faults_without_callback_waiting_and_a_running_long_song_does_not_need_an_unrepresentable_position(
) {
    let (shared, mut runtime, events) = configured();
    runtime.begin_at(48000, 1, 10_000_000, true);
    for _ in 0..8192 {
        shared
            .events
            .try_send(Event {
                message: Message::Clock,
                deadline_ns: 0,
                generation: 1,
            })
            .unwrap();
    }
    assert_eq!(
        test_alloc::measure(|| runtime.frame(0.0, 0.001, true, 0, 0)),
        Default::default()
    );
    assert_eq!(shared.counters().error, Some(Fault::QueueFull.text()));
    assert_eq!(shared.counters().overflow, 1);
    assert_eq!(events.len(), 8192);
    let (shared, mut runtime, events) = configured();
    runtime.begin_at(48000, 2, 10_000_000, true);
    runtime.frame(4095.75, 4095.75004, true, 0, 0);
    runtime.frame(4095.75004, 4095.75008, true, 0, 1);
    assert!(shared.counters().enabled);
    assert_eq!(shared.counters().error, None);
    assert_eq!(
        events.try_iter().next().unwrap().message,
        Message::Position(16383)
    );
}
#[test]
fn clock_preferences_roundtrip_and_migrate_navigation_profiles_while_legacy_headers_refuse_injection(
) {
    let mut preferences =
        crate::preferences::Preferences::defaults(std::path::Path::new("/home/clock-fixture"));
    let config = Config {
        enabled: true,
        ports: vec![Endpoint {
            name: "drum machine".into(),
            id: Some("42:0".into()),
        }],
        compensation_ms: -12,
    };
    preferences.profiles.get_mut("Studio").unwrap().midi_clock = config.clone();
    let bytes = serde_json::to_vec(&preferences).unwrap();
    let (reopened, migrated) = crate::preferences::storage::decode(&bytes).unwrap();
    assert!(!migrated);
    assert_eq!(reopened.current().unwrap().midi_clock, config);
    let mut raw = serde_json::to_value(preferences).unwrap();
    raw["version"] = 20.into();
    assert!(crate::preferences::storage::decode(&serde_json::to_vec(&raw).unwrap()).is_err());
    raw["profiles"]["Studio"]["midi_clock"] = serde_json::Value::Null;
    assert!(crate::preferences::storage::decode(&serde_json::to_vec(&raw).unwrap()).is_err());
    for profile in raw["profiles"].as_object_mut().unwrap().values_mut() {
        profile.as_object_mut().unwrap().remove("midi_clock");
    }
    let (reopened, migrated) =
        crate::preferences::storage::decode(&serde_json::to_vec(&raw).unwrap()).unwrap();
    assert!(migrated);
    assert!(reopened.current().unwrap().midi_clock.is_default());
    for config in [
        Config {
            enabled: true,
            ..Default::default()
        },
        Config {
            compensation_ms: 501,
            ..Default::default()
        },
        Config {
            ports: vec![Endpoint {
                name: " ".into(),
                id: None,
            }],
            ..Default::default()
        },
    ] {
        assert!(config.validate().is_err());
    }
}

#[derive(Clone)]
struct Backend {
    capture: Arc<Mutex<Vec<(usize, Vec<u8>, Instant)>>>,
    fail: Arc<AtomicBool>,
    discovery: Arc<
        Mutex<
            Option<(
                crossbeam_channel::Sender<()>,
                crossbeam_channel::Receiver<()>,
            )>,
        >,
    >,
    connections: Arc<AtomicU64>,
    clock_delay_ns: Arc<AtomicU64>,
}
impl output::Backend for Backend {
    type Port = usize;
    type Connection = usize;
    fn discover(&mut self) -> Result<Vec<output::Port<usize>>, String> {
        let gate = self.discovery.lock().unwrap().take();
        if let Some((entered, released)) = gate {
            entered.send(()).unwrap();
            released.recv().unwrap();
        }
        Ok((0..3)
            .map(|port| output::Port {
                endpoint: Endpoint {
                    name: format!("fake instrument {port}"),
                    id: Some(format!("{}:0", port + 20)),
                },
                port,
            })
            .collect())
    }
    fn connect(&mut self, port: &usize) -> Result<usize, String> {
        self.connections.fetch_add(1, Relaxed);
        Ok(*port)
    }
    fn send(&mut self, connection: &mut usize, bytes: &[u8]) -> Result<(), String> {
        if self.fail.load(Acquire) && bytes == [0xf8] {
            return Err("injected output failure".into());
        }
        if bytes == [0xf8] {
            std::thread::sleep(Duration::from_nanos(self.clock_delay_ns.load(Relaxed)));
        }
        self.capture
            .lock()
            .unwrap()
            .push((*connection, bytes.to_vec(), Instant::now()));
        Ok(())
    }
}
fn wait(mut condition: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < end, "clock owner did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn fake() -> Backend {
    Backend {
        capture: Arc::new(Mutex::new(Vec::new())),
        fail: Arc::new(AtomicBool::new(false)),
        discovery: Arc::new(Mutex::new(None)),
        connections: Arc::new(AtomicU64::new(0)),
        clock_delay_ns: Arc::new(AtomicU64::new(0)),
    }
}
fn policy() -> Config {
    Config {
        enabled: true,
        ports: vec![Endpoint {
            name: "fake instrument 1".into(),
            id: Some("21:0".into()),
        }],
        compensation_ms: 0,
    }
}
#[test]
fn actual_audio_callback_drives_only_selected_outputs_and_echoed_transport_preserves_the_song() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.quantize = false;
    let backend = fake();
    let captured = backend.capture.clone();
    let manager = output::Manager::start_backend(engine.cmd.clone(), policy(), backend).unwrap();
    wait(|| !manager.status().pending);
    assert!(manager.status().error.is_none());
    rt.apply(Command::Play);
    let mut callback = crate::engine::audio::OutputCallback::new(rt, 2);
    let mut data = [0_f32; 960];
    callback.render_timed(&mut data, Some(Duration::from_millis(30)));
    wait(|| engine.cmd.clock_output().counters().sent >= 1);
    assert!(captured
        .lock()
        .unwrap()
        .iter()
        .all(|(port, _, _)| *port == 1));
    assert_eq!(captured.lock().unwrap()[0].1, [0xfa]);
    let hub = crate::engine::midi::MidiHub::without_devices();
    let map = crate::engine::midi::MidiMap {
        name: "Clock keyboard".into(),
        matchers: vec![],
        bindings: vec![],
        unmapped_notes: crate::engine::midi::UnmappedNotes::Live,
    };
    let mut echo = hub.open_for_test(&engine.cmd, 555, map, "another port", "21:7");
    echo.push(&[0xfc]);
    callback.render_timed(&mut data, Some(Duration::from_millis(30)));
    assert!(callback.renderer_for_test().playing);
    echo.push(&[0xf8]);
    callback.render_timed(&mut data, Some(Duration::from_millis(30)));
    assert_eq!(callback.renderer_for_test().midi_clock.ticks, 1);
    callback.renderer_mut_for_test().apply(Command::Stop);
    callback.render_timed(&mut data, Some(Duration::from_millis(30)));
    wait(|| {
        captured
            .lock()
            .unwrap()
            .iter()
            .any(|(_, bytes, _)| bytes.as_slice() == [0xfc])
    });
    assert!(!engine.cmd.clock_output().counters().running);
}
#[test]
fn output_failure_stops_all_selected_clock_ports_and_requires_explicit_retry() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let backend = fake();
    let failure = backend.fail.clone();
    let captured = backend.capture.clone();
    let manager = output::Manager::start_backend(engine.cmd.clone(), policy(), backend).unwrap();
    wait(|| !manager.status().pending);
    failure.store(true, Release);
    rt.apply(Command::Play);
    let mut callback = crate::engine::audio::OutputCallback::new(rt, 2);
    callback.render_timed(&mut [0_f32; 960], Some(Duration::from_millis(10)));
    wait(|| engine.cmd.clock_output().counters().error.is_some());
    wait(|| {
        captured
            .lock()
            .unwrap()
            .iter()
            .any(|(_, bytes, _)| bytes.as_slice() == [0xfc])
    });
    assert_eq!(
        engine.cmd.clock_output().counters().error,
        Some(Fault::Send.text())
    );
    failure.store(false, Release);
    callback.renderer_mut_for_test().apply(Command::Stop);
    callback.render_timed(&mut [0_f32; 960], Some(Duration::from_millis(10)));
    manager.configure(policy()).unwrap();
    wait(|| !manager.status().pending);
    assert_eq!(engine.cmd.clock_output().counters().error, None);
}

#[test]
fn held_discovery_cancellation_and_missing_ports_preserve_the_applied_policy_without_connecting() {
    let (engine, _rt) = Engine::headless_for_test(48000, 256);
    let backend = fake();
    let gate = backend.discovery.clone();
    let connections = backend.connections.clone();
    let manager =
        output::Manager::start_backend(engine.cmd.clone(), Config::default(), backend).unwrap();
    wait(|| !manager.status().pending);
    let (entered, ready) = crossbeam_channel::bounded(1);
    let (release, released) = crossbeam_channel::bounded(1);
    *gate.lock().unwrap() = Some((entered, released));
    manager.configure(policy()).unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(manager.cancel());
    release.send(()).unwrap();
    wait(|| !manager.status().pending);
    assert!(manager
        .status()
        .error
        .as_ref()
        .unwrap()
        .contains("cancelled"));
    assert!(!manager.status().applied.enabled);
    assert_eq!(connections.load(Relaxed), 0);
    let missing = Config {
        enabled: true,
        ports: vec![Endpoint {
            name: "missing exact port".into(),
            id: None,
        }],
        compensation_ms: 0,
    };
    manager.configure(missing).unwrap();
    wait(|| !manager.status().pending);
    assert!(manager.status().error.as_ref().unwrap().contains("missing"));
    assert!(!manager.status().applied.enabled);
    assert_eq!(connections.load(Relaxed), 0);
    manager.configure(policy()).unwrap();
    wait(|| !manager.status().pending);
    assert!(manager.status().applied.enabled);
    assert_eq!(connections.load(Relaxed), 1);
}

#[test]
fn actual_song_renderer_follows_an_independent_ramp_and_step_oracle_across_callback_sizes_without_heap(
) {
    use crate::engine::midi_data::{Conductor, Meter, Tempo, TimingSettings};
    let timestamp = |beat: f64| {
        2.0 * (1.0 + beat.min(2.0) / 2.0).ln()
            + (beat - 2.0).clamp(0.0, 2.0) * 0.5
            + (beat - 4.0).max(0.0) * 0.75
    };
    for rate in [8000, 44100, 48000, 96000] {
        for block_size in [31, 257] {
            let (engine, mut rt) = Engine::headless_for_test(rate, 256);
            let shared = engine.cmd.clock_output().clone();
            let events = shared.receiver.lock().take().unwrap();
            shared.generation.store(1, Release);
            shared.enabled.store(true, Release);
            rt.quantize = false;
            rt.metronome = false;
            rt.conductor = Some(
                Conductor::native(
                    960,
                    vec![
                        Tempo::new(0, 60.0, true).unwrap(),
                        Tempo::new(1920, 120.0, false).unwrap(),
                        Tempo::new(3840, 80.0, false).unwrap(),
                    ],
                    vec![Meter {
                        tick: 0,
                        numerator: 7,
                        denominator_power: 3,
                        clocks: 24,
                        thirty_seconds: 8,
                    }],
                    TimingSettings::default(),
                )
                .unwrap(),
            );
            rt.apply(Command::Play);
            let frames = (timestamp(6.0) * f64::from(rate)).ceil() as usize;
            let base = 1_000_000_000_u64;
            let mut data = vec![0.0; block_size * 2];
            assert_eq!(
                test_alloc::measure(|| {
                    let mut rendered = 0;
                    while rendered < frames {
                        let count = (frames - rendered).min(block_size);
                        rt.clock_output.begin_at(
                            rate,
                            count,
                            base + rendered as u64 * 1_000_000_000 / u64::from(rate),
                            true,
                        );
                        rt.process(&mut data[..count * 2]);
                        rendered += count;
                    }
                }),
                Default::default()
            );
            let actual: Vec<_> = events.try_iter().collect();
            assert_eq!(actual[0].message, Message::Start);
            assert!(actual[1..]
                .iter()
                .all(|event| event.message == Message::Clock));
            assert_eq!(actual.len(), 145);
            for (tick, event) in actual[1..].iter().enumerate() {
                let frame = (timestamp(tick as f64 / 24.0) * f64::from(rate)).ceil() as u64;
                let expected = base + frame * 1_000_000_000 / u64::from(rate);
                assert!(
                    event.deadline_ns.abs_diff(expected)
                        <= 1_000_000_000_u64.div_ceil(u64::from(rate)),
                    "rate {rate}, callback {block_size}, clock {tick}: {} versus {expected}",
                    event.deadline_ns
                );
            }
            assert_eq!(shared.counters().error, None);
        }
    }
}

#[test]
fn all_selected_ports_receive_ordered_compensated_transport_and_report_slow_backend_completion() {
    let (engine, _rt) = Engine::headless_for_test(48000, 256);
    let backend = fake();
    let captured = backend.capture.clone();
    backend.clock_delay_ns.store(8_000_000, Relaxed);
    let mut config = policy();
    config.ports.push(Endpoint {
        name: "fake instrument 2".into(),
        id: Some("22:0".into()),
    });
    config.compensation_ms = 40;
    let manager =
        output::Manager::start_backend(engine.cmd.clone(), config.clone(), backend).unwrap();
    wait(|| !manager.status().pending);
    let shared = engine.cmd.clock_output().clone();
    let mut runtime = Runtime::new(shared.clone());
    let base = shared.now_ns() + 20_000_000;
    runtime.begin_at(48000, 2, base, true);
    runtime.frame(0.0, 1.0 / 24000.0, true, 0, 0);
    manager.configure(policy()).unwrap();
    wait(|| !manager.status().pending);
    assert!(manager
        .status()
        .error
        .as_ref()
        .unwrap()
        .contains("Stop the song"));
    assert_eq!(manager.status().applied.as_ref(), &config);
    assert!(shared.counters().enabled);
    wait(|| shared.counters().sent == 1);
    assert!(shared.counters().late >= 1);
    assert!(shared.counters().max_late_ns >= 16_000_000);
    runtime.frame(1.0 / 24000.0, 1.0 / 24000.0, false, 1, 1);
    wait(|| {
        captured
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, bytes, _)| bytes.as_slice() == [0xfc])
            .count()
            == 2
    });
    let records = captured.lock().unwrap();
    assert!(records.iter().all(|(port, _, _)| [1, 2].contains(port)));
    for port in [1, 2] {
        let records: Vec<_> = records
            .iter()
            .filter(|(actual, _, _)| *actual == port)
            .collect();
        assert_eq!(
            records
                .iter()
                .map(|(_, bytes, _)| bytes.as_slice())
                .collect::<Vec<_>>(),
            [&[0xfa][..], &[0xf8][..], &[0xfc][..]]
        );
        assert!(records
            .iter()
            .all(|(_, _, time)| ns(time.duration_since(shared.origin)) >= base + 40_000_000));
    }
}

#[test]
fn lost_audio_callbacks_and_emergency_requests_stop_clock_without_another_audio_callback() {
    for emergency in [false, true] {
        let (engine, mut rt) = Engine::headless_for_test(48000, 256);
        rt.quantize = false;
        let backend = fake();
        let captured = backend.capture.clone();
        let manager =
            output::Manager::start_backend(engine.cmd.clone(), policy(), backend).unwrap();
        wait(|| !manager.status().pending);
        rt.apply(Command::Play);
        let mut callback = crate::engine::audio::OutputCallback::new(rt, 2);
        callback.render_timed(&mut [0.0_f32; 960], Some(Duration::from_millis(10)));
        let shared = engine.cmd.clock_output().clone();
        wait(|| shared.counters().sent >= 1);
        if emergency {
            engine
                .cmd
                .performance()
                .request_safety(crate::engine::performance::Safety::Silence);
        }
        wait(|| shared.counters().error.is_some());
        assert_eq!(
            shared.counters().error,
            Some(if emergency {
                Fault::Safety.text()
            } else {
                Fault::Callback.text()
            })
        );
        wait(|| {
            captured
                .lock()
                .unwrap()
                .iter()
                .any(|(_, bytes, _)| bytes.as_slice() == [0xfc])
        });
        assert!(!shared.counters().enabled && !shared.counters().running);
        let records = captured.lock().unwrap();
        assert_eq!(records.last().unwrap().1, [0xfc]);
    }
    let (engine, _rt) = Engine::headless_for_test(48000, 256);
    let manager =
        output::Manager::start_backend(engine.cmd.clone(), Config::default(), fake()).unwrap();
    wait(|| !manager.status().pending);
    engine
        .cmd
        .performance()
        .request_safety(crate::engine::performance::Safety::Stop);
    std::thread::sleep(Duration::from_millis(10));
    assert_eq!(engine.cmd.clock_output().counters().error, None);
}

struct Load {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<u64>>,
}
impl Drop for Load {
    fn drop(&mut self) {
        self.stop.store(true, Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[test]
fn actual_output_deadlines_remain_bounded_under_real_decode_and_egui_load_at_several_tempos() {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::spawn(move || {
        let context = egui::Context::default();
        let start = Instant::now();
        let mut iterations = 0;
        while !flag.load(Acquire) {
            let decoded = crate::engine::decode::decode_audio(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/audio/tone.flac")
                    .as_path(),
            )
            .unwrap();
            let _ = context.run(
                egui::RawInput {
                    time: Some(start.elapsed().as_secs_f64()),
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::Vec2::new(1200.0, 800.0),
                    )),
                    ..Default::default()
                },
                |context| {
                    egui::CentralPanel::default().show(context, |ui| {
                        ui.label(&decoded.sample.name);
                        for index in 0..64 {
                            let mut value = decoded.sample.data[index];
                            ui.add(
                                egui::Slider::new(&mut value, -1.0..=1.0)
                                    .text(format!("Decode/load meter {index}")),
                            );
                        }
                    });
                },
            );
            iterations += 1;
        }
        iterations
    });
    let mut load = Load {
        stop,
        thread: Some(thread),
    };
    let mut reports = Vec::new();
    for bpm in [40_u32, 73, 120, 137, 180, 240] {
        let (engine, _rt) = Engine::headless_for_test(48000, 256);
        let backend = fake();
        let capture = backend.capture.clone();
        let manager =
            output::Manager::start_backend(engine.cmd.clone(), policy(), backend).unwrap();
        wait(|| !manager.status().pending);
        let shared = engine.cmd.clock_output().clone();
        let mut runtime = Runtime::new(shared.clone());
        let rate = 48000_u32;
        let frames = (u64::from(rate) * 60 / u64::from(bpm)) as usize;
        let base = shared.now_ns() + 60_000_000;
        runtime.begin_at(rate, frames + 1, base, true);
        for frame in 0..frames {
            let beat = frame as f64 * f64::from(bpm) / (f64::from(rate) * 60.0);
            let next = (frame + 1) as f64 * f64::from(bpm) / (f64::from(rate) * 60.0);
            runtime.frame(beat, next, true, 0, frame);
        }
        runtime.frame(
            frames as f64 * f64::from(bpm) / (f64::from(rate) * 60.0),
            frames as f64 * f64::from(bpm) / (f64::from(rate) * 60.0),
            false,
            1,
            frames,
        );
        wait(|| shared.counters().sent == 24);
        let timestamps: Vec<_> = capture
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, bytes, _)| bytes.as_slice() == [0xf8])
            .map(|(_, _, time)| ns(time.duration_since(shared.origin)))
            .collect();
        assert_eq!(timestamps.len(), 24);
        let mut delays = Vec::new();
        for (tick, time) in timestamps.into_iter().enumerate() {
            let frame = (tick as u64 * u64::from(rate) * 60).div_ceil(u64::from(bpm) * 24);
            let expected = base + frame * 1_000_000_000 / u64::from(rate);
            assert!(time >= expected);
            delays.push(time - expected);
        }
        let maximum = *delays.iter().max().unwrap();
        assert!(
            maximum < 5_000_000,
            "clock delay {maximum} ns at {bpm} BPM under decode/UI load"
        );
        reports.push(
            serde_json::json!({"bpm":bpm,"clocks":24,"delay_ns":delays,"max_delay_ns":maximum}),
        );
    }
    load.stop.store(true, Release);
    let iterations = load.thread.take().unwrap().join().unwrap();
    assert!(iterations > 0);
    if let Some(path) = std::env::var_os("OMATAINER_CLOCK_TIMING_RECEIPT") {
        let path = std::path::PathBuf::from(path);
        assert!(path.is_absolute() && path.starts_with("/home"));
        std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"real_decode_egui_iterations":iterations,"physical_devices_opened":false,"tempos":reports})).unwrap()).unwrap();
    }
}
#[test]
fn actual_scene_tempo_and_meter_changes_keep_clock_continuous_at_the_shared_launch_sample() {
    use crate::engine::{scene::{Properties, Signature, Empty}, clip_launch::Grid};
    for rate in [8000, 44100, 48000, 96000] { for block_size in [31,257] {
        let (engine,mut rt)=Engine::headless_for_test(rate,256);
        let shared=engine.cmd.clock_output().clone();let events=shared.receiver.lock().take().unwrap();shared.generation.store(1,Release);shared.enabled.store(true,Release);
        rt.session.scenes[0].scene=Properties{tempo_micros:Some(500000),meter:Some(Signature{numerator:7,denominator_power:3}),grid:Grid::Immediate,empty:Empty::Keep};
        rt.apply(Command::LaunchScene{scene:0});rt.apply(Command::Play);
        rt.session.scenes[1].scene=Properties{tempo_micros:Some(750000),meter:Some(Signature::default()),grid:Grid::Bar,empty:Empty::Keep};
        let base=1_000_000_000u64;let boundary=(f64::from(rate)*1.75).round()as usize;let frames=boundary+(f64::from(rate)*0.75).round()as usize;let mut data=vec![0.0;block_size*2];
        let render=|rt:&mut crate::engine::RtEngine,begin:usize,end:usize,data:&mut[f32]|{let mut rendered=begin;while rendered<end{let count=(end-rendered).min(block_size);rt.clock_output.begin_at(rate,count,base+rendered as u64*1_000_000_000/u64::from(rate),true);rt.process(&mut data[..count*2]);rendered+=count;}};
        assert_eq!(test_alloc::measure(||{render(&mut rt,0,100,&mut data);rt.apply(Command::LaunchScene{scene:1});render(&mut rt,100,frames,&mut data);}),Default::default());
        assert!(rt.scenes.pending.is_none());assert_eq!(rt.scenes.timing.unwrap().signature,Signature::default());
        let actual:Vec<_>=events.try_iter().collect();assert_eq!(actual[0].message,Message::Start);assert!(actual[1..].iter().all(|e|e.message==Message::Clock));assert_eq!(actual.len(),109);
        for(tick,event)in actual[1..].iter().enumerate(){let beat=tick as f64/24.0;let seconds=beat.min(3.5)*0.5+(beat-3.5).max(0.0)*0.75;let expected=base+(seconds*f64::from(rate)).ceil()as u64*1_000_000_000/u64::from(rate);assert!(event.deadline_ns.abs_diff(expected)<=1_000_000_000u64.div_ceil(u64::from(rate)),"rate{rate} block{block_size} tick{tick}:{} vs {expected}",event.deadline_ns);}
        assert_eq!(shared.counters().error,None);
    }}
}
