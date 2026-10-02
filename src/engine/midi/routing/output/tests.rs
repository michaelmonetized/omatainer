use super::*;
use crate::engine::midi::routing::{Filter, Input, Route};
use crate::engine::midi::{builtin_maps, handoff, pick_map};
use crate::engine::{test_alloc, Command, Engine, RtEngine};
use parking_lot::Mutex;
use std::time::Instant;

#[derive(Default)]
struct Fixture {
    trace: Mutex<Vec<Vec<u8>>>,
    opens: AtomicU64,
    closes: AtomicU64,
    blocked: AtomicBool,
    entered: AtomicBool,
    fail: AtomicBool,
    ambiguous: AtomicBool,
    missing: AtomicBool,
    send_blocked: AtomicBool,
    send_entered: AtomicBool,
}
struct Fake(Arc<Fixture>);
struct Connection(Arc<Fixture>);
pub(crate) fn install_for_test(engine: &mut Engine, config: Routing) {
    engine.midi.routing = Some(
        Manager::start_backend(
            engine.cmd.clone(),
            config,
            Fake(Arc::new(Fixture::default())),
        )
        .unwrap(),
    );
    until(|| !engine.midi.routing_status().unwrap().pending);
}
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.closes.fetch_add(1, Relaxed);
    }
}
fn endpoint(name: &str, id: &str) -> Endpoint {
    Endpoint {
        name: name.into(),
        id: Some(id.into()),
    }
}
impl Backend for Fake {
    type Port = ();
    type Connection = Connection;
    fn discover(&mut self) -> Result<Inventory<()>, String> {
        let mut outputs = vec![Port {
            endpoint: endpoint("Synth", "300:0"),
            port: (),
        }];
        if self.0.missing.load(Acquire) {
            outputs.clear();
        }
        if self.0.ambiguous.load(Acquire) {
            outputs.push(Port {
                endpoint: endpoint("Synth", "301:0"),
                port: (),
            });
        }
        Ok(Inventory {
            inputs: vec![
                endpoint("Keyboard A", "100:0"),
                endpoint("Keyboard B", "101:0"),
            ],
            outputs,
        })
    }
    fn connect(&mut self, _: &()) -> Result<Connection, String> {
        self.0.entered.store(true, Release);
        while self.0.blocked.load(Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.0.opens.fetch_add(1, Relaxed);
        Ok(Connection(self.0.clone()))
    }
    fn send(&mut self, _: &mut Connection, bytes: &[u8]) -> Result<(), String> {
        self.0.send_entered.store(true, Release);
        while self.0.send_blocked.load(Acquire) {
            std::thread::sleep(Duration::from_millis(1));
        }
        if self.0.fail.load(Acquire) {
            return Err("fixture send failure".into());
        }
        self.0.trace.lock().push(bytes.to_vec());
        Ok(())
    }
}
fn until(mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(5);
    while !f() {
        assert!(Instant::now() < end, "MIDI route owner did not settle");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn config() -> Routing {
    Routing {
        enabled: true,
        routes: (0..2)
            .map(|i| Route {
                track: 2 + i,
                inputs: vec![Input {
                    port: endpoint(
                        if i == 0 { "Keyboard A" } else { "Keyboard B" },
                        if i == 0 { "100:0" } else { "101:0" },
                    ),
                    channels: 1,
                }],
                output: Some(endpoint("Synth", "300:0")),
                output_channel: Some(4 + i),
                monitor: true,
                thru: true,
                filter: Filter {
                    sysex: true,
                    ..Default::default()
                },
            })
            .collect(),
    }
}
fn input(
    engine: &Engine,
    source: u64,
    name: &str,
    id: &str,
) -> (
    handoff::InputSink,
    handoff::InputGuard,
    Arc<handoff::InputCounters>,
) {
    let counters = Arc::new(handoff::InputCounters::default());
    let (sink, guard) = handoff::start_on_port(
        source,
        pick_map(&builtin_maps().unwrap(), "Generic Keyboard"),
        engine.cmd.clone(),
        engine.midi.log.clone(),
        engine.midi.learn.clone(),
        name.into(),
        id.into(),
        counters.clone(),
        true,
        || {},
    )
    .unwrap();
    (sink, guard, counters)
}
fn held(rt: &RtEngine, track: usize) -> usize {
    rt.tracks[track]
        .poly
        .voices
        .iter()
        .filter(|v| matches!(v.env.stage, 1..=3))
        .count()
}
#[test]
fn simultaneous_controllers_route_all_types_and_release_original_track() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let fixture = Arc::new(Fixture::default());
    let manager =
        Manager::start_backend(engine.cmd.clone(), config(), Fake(fixture.clone())).unwrap();
    until(|| !manager.status().pending);
    assert!(manager.status().error.is_none());
    assert_eq!(
        fixture.opens.load(Relaxed),
        1,
        "shared multitimbral port opened twice"
    );
    let (mut a, ga, ca) = input(&engine, 31, "Keyboard A", "100:0");
    let (mut b, gb, cb) = input(&engine, 32, "Keyboard B", "101:0");
    engine
        .cmd
        .send(Command::Select { track: 7, scene: 0 })
        .unwrap();
    let counts = test_alloc::measure(|| {
        a.push(&[0x90, 60, 100]);
        b.push(&[0x90, 60, 90]);
    });
    assert_eq!(counts, test_alloc::Counts::default());
    until(|| ca.snapshot().dispatched == 1 && cb.snapshot().dispatched == 1);
    let counts = test_alloc::measure(|| rt.process(&mut [0.0; 128]));
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(held(&rt, 2), 1);
    assert_eq!(held(&rt, 3), 1);
    assert_eq!(held(&rt, 7), 0);
    a.push(&[0x80, 60, 37]);
    until(|| ca.snapshot().dispatched == 2);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt, 2), 0);
    assert_eq!(held(&rt, 3), 1);
    let messages = [
        vec![0xb0, 0, 2],
        vec![0xb0, 32, 3],
        vec![0xb0, 1, 64],
        vec![0xc0, 9],
        vec![0xa0, 60, 44],
        vec![0xd0, 55],
        vec![0xe0, 0, 65],
        vec![0xf0, 0x7d, 1, 2, 0xf7],
    ];
    for bytes in &messages {
        a.push(bytes);
    }
    until(|| ca.snapshot().dispatched == 10);
    until(|| fixture.trace.lock().len() >= 11);
    let trace = fixture.trace.lock().clone();
    assert!(trace.contains(&vec![0x94, 60, 100]));
    assert!(trace.contains(&vec![0x95, 60, 90]));
    assert!(trace.contains(&vec![0x84, 60, 37]));
    for bytes in &messages {
        let expected = super::super::packet::Packet::new(bytes)
            .unwrap()
            .with_channel(Some(4));
        assert!(
            trace.contains(&expected.bytes().to_vec()),
            "missing wire bytes{bytes:?}"
        );
    }
    a.push(&[0x94, 60, 100]);
    until(|| ca.snapshot().dispatched == 11);
    assert_eq!(held(&rt, 2), 0);
    let mut updated = config();
    updated.routes[0].track = 5;
    let generation = manager.configure(updated).unwrap();
    until(|| manager.status().applied_generation == generation && !manager.status().pending);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt, 3), 0);
    assert!(fixture.trace.lock().iter().any(|b| b == &[0xb5, 123, 0]));
    let counts = test_alloc::measure(|| a.push(&[0x90, 62, 70]));
    assert_eq!(counts, test_alloc::Counts::default());
    until(|| ca.snapshot().dispatched == 12);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt, 5), 1);
    drop(a);
    drop(ga);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt, 5), 0);
    drop(b);
    drop(gb);
    drop(manager);
    until(|| fixture.closes.load(Relaxed) == 1);
    if let Ok(path) = std::env::var("OMAT_MIDI_ROUTING_TRACE") {
        std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"kind":"software backend and real input worker/renderer; no physical hardware","messages":trace,"activity":engine.cmd.midi_routing().activity()})).unwrap()).unwrap();
    }
}
#[test]
fn blocked_open_cancel_preserves_previous_routing_and_renderer_progress() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let fixture = Arc::new(Fixture::default());
    let manager = Manager::start_backend(
        engine.cmd.clone(),
        Routing::default(),
        Fake(fixture.clone()),
    )
    .unwrap();
    until(|| !manager.status().pending);
    fixture.blocked.store(true, Release);
    manager.configure(config()).unwrap();
    until(|| fixture.entered.load(Acquire));
    assert!(manager.status().pending);
    assert!(manager.status().applied.routes.is_empty());
    assert!(manager.cancel());
    engine.cmd.send(Command::Master(0.71)).unwrap();
    let counts = test_alloc::measure(|| {
        for _ in 0..128 {
            rt.process(&mut [0.0; 128]);
        }
    });
    assert_eq!(counts, test_alloc::Counts::default());
    assert_eq!(rt.master, 0.71);
    fixture.blocked.store(false, Release);
    until(|| !manager.status().pending);
    assert!(manager
        .status()
        .error
        .as_ref()
        .unwrap()
        .contains("cancelled"));
    assert!(manager.status().applied.routes.is_empty());
    until(|| fixture.closes.load(Relaxed) == 1);
    let generation = manager.configure(config()).unwrap();
    until(|| manager.status().applied_generation == generation);
    assert!(manager.status().error.is_none());
    drop(manager);
    until(|| fixture.closes.load(Relaxed) == 2);
}
#[test]
fn ambiguous_missing_and_send_failure_never_select_another_output() {
    let (engine, _rt) = Engine::headless_for_test(48000, 256);
    let fixture = Arc::new(Fixture::default());
    fixture.ambiguous.store(true, Release);
    let manager = Manager::start_backend(
        engine.cmd.clone(),
        Routing::default(),
        Fake(fixture.clone()),
    )
    .unwrap();
    until(|| !manager.status().pending);
    let mut routing = config();
    for r in &mut routing.routes {
        r.output.as_mut().unwrap().id = None;
    }
    manager.configure(routing).unwrap();
    until(|| !manager.status().pending);
    assert!(manager
        .status()
        .error
        .as_ref()
        .unwrap()
        .contains("ambiguous"));
    assert_eq!(fixture.opens.load(Relaxed), 0);
    let mut missing = config();
    missing.routes[0].output.as_mut().unwrap().name = "Absent".into();
    manager.configure(missing).unwrap();
    until(|| !manager.status().pending);
    assert!(manager.status().error.as_ref().unwrap().contains("missing"));
    assert_eq!(fixture.opens.load(Relaxed), 0);
    let generation = manager.configure(config()).unwrap();
    until(|| manager.status().applied_generation == generation);
    fixture.fail.store(true, Release);
    assert!(engine.cmd.midi_routing().emit(
        2,
        super::super::packet::Packet::new(&[0x90, 60, 100]).unwrap()
    ));
    until(|| {
        manager
            .status()
            .error
            .as_ref()
            .is_some_and(|e| e.contains("send failed"))
    });
    assert_eq!(engine.cmd.midi_routing().activity().0[2].failed, 1);
    until(|| fixture.closes.load(Relaxed) == 1);
    assert!(fixture.trace.lock().is_empty());
}
#[test]
fn clip_stop_preserves_a_live_gate_on_the_same_external_pitch() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let fixture = Arc::new(Fixture::default());
    let manager =
        Manager::start_backend(engine.cmd.clone(), config(), Fake(fixture.clone())).unwrap();
    until(|| !manager.status().pending);
    let (mut a, guard, counters) = input(&engine, 71, "Keyboard A", "100:0");
    a.push(&[0x90, 60, 100]);
    until(|| counters.snapshot().dispatched == 1);
    until(|| fixture.trace.lock().len() == 1);
    let clip = &mut rt.tracks[2].clips[0];
    clip.kind = crate::engine::ClipKind::Midi;
    clip.bars = 1.0;
    clip.notes = vec![crate::engine::MidiNote {
        id: crate::engine::midi_edit::NoteId::new(),
        channel: 0,
        release_vel: 37,
        source_timing: None,
        muted: false,
        pitch: 60,
        start: 0.0,
        len: 4.0,
        vel: 80,
    }];
    rt.quant = 0.0;
    engine
        .cmd
        .send(Command::FireClip {
            track: 2,
            scene: 0,
            looping: true,
        })
        .unwrap();
    assert_eq!(
        test_alloc::measure(|| rt.process(&mut [0.0; 128])),
        test_alloc::Counts::default()
    );
    until(|| engine.cmd.midi_routing().activity().0[2].filtered > 0);
    engine.cmd.send(Command::StopTrack { track: 2 }).unwrap();
    rt.process(&mut [0.0; 128]);
    std::thread::sleep(Duration::from_millis(20));
    assert_eq!(
        fixture
            .trace
            .lock()
            .iter()
            .filter(|p| p[0] & 0xf0 == 0x80)
            .count(),
        0
    );
    a.push(&[0x80, 60, 23]);
    until(|| counters.snapshot().dispatched == 2);
    until(|| fixture.trace.lock().iter().any(|p| p == &[0x84, 60, 23]));
    assert_eq!(
        fixture
            .trace
            .lock()
            .iter()
            .filter(|p| p[0] & 0xf0 == 0x90)
            .count(),
        1
    );
    drop(a);
    drop(guard);
    drop(manager);
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "Maintainer validation: creates and removes only private ALSA virtual MIDI ports"]
fn linux_virtual_ports_deliver_two_controllers_all_types_and_reset() {
    use midir::os::unix::{VirtualInput, VirtualOutput};
    let token = format!(
        "omat112-{}-{}",
        std::process::id(),
        crate::sampler_bank::BankId::new().unwrap()
    );
    let name_a = format!("{token}-A");
    let name_b = format!("{token}-B");
    let name_sink = format!("{token}-sink");
    let mut a = MidiOutput::new(&name_a)
        .unwrap()
        .create_virtual("source-A")
        .unwrap();
    let mut b = MidiOutput::new(&name_b)
        .unwrap()
        .create_virtual("source-B")
        .unwrap();
    let trace = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let received = trace.clone();
    let mut input = MidiInput::new(&name_sink).unwrap();
    input.ignore(midir::Ignore::None);
    let sink = input
        .create_virtual(
            "receiver",
            move |_, message, _| received.lock().push(message.to_vec()),
            (),
        )
        .unwrap();
    let inventory = Midir.discover().unwrap();
    let port_a = inventory
        .inputs
        .iter()
        .find(|p| p.name.contains(&name_a))
        .unwrap()
        .clone();
    let port_b = inventory
        .inputs
        .iter()
        .find(|p| p.name.contains(&name_b))
        .unwrap()
        .clone();
    let output = inventory
        .outputs
        .iter()
        .find(|p| p.endpoint.name.contains(&name_sink))
        .unwrap()
        .endpoint
        .clone();
    let mut routing = config();
    routing.routes[0].inputs[0].port = port_a.clone();
    routing.routes[1].inputs[0].port = port_b.clone();
    for route in &mut routing.routes {
        route.output = Some(output.clone());
    }
    let (mut engine, mut rt) = Engine::headless_for_test(48000, 256);
    engine.midi = crate::engine::midi::MidiHub::start_with_routing(
        engine.cmd.clone(),
        engine.snap.clone(),
        crate::engine::midi::InputPolicy::Selected(vec![port_a.name.clone(), port_b.name.clone()]),
        routing.clone(),
    )
    .unwrap();
    until(|| {
        engine
            .midi
            .routing_status()
            .is_some_and(|s| !s.pending && s.applied_generation == 1)
    });
    assert!(engine.midi.routing_status().unwrap().error.is_none());
    until(|| engine.midi.policy_status().is_some_and(|s| !s.pending()));
    assert!(engine.midi.policy_status().unwrap().error.is_none());
    rt.selected_track = 7;
    a.send(&[0x90, 60, 98]).unwrap();
    b.send(&[0x90, 60, 88]).unwrap();
    until(|| engine.midi.input_stats().dispatched >= 2);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt, 2), 1);
    assert_eq!(held(&rt, 3), 1);
    assert_eq!(held(&rt, 7), 0);
    let source = [
        vec![0xb0, 0, 2],
        vec![0xb0, 32, 3],
        vec![0xb0, 1, 64],
        vec![0xc0, 9],
        vec![0xa0, 60, 44],
        vec![0xd0, 55],
        vec![0xe0, 0, 65],
        vec![0xf0, 0x7d, 1, 2, 0xf7],
    ];
    for bytes in &source {
        a.send(bytes).unwrap();
    }
    a.send(&[0x80, 60, 37]).unwrap();
    b.send(&[0x80, 60, 41]).unwrap();
    until(|| engine.midi.input_stats().dispatched >= 12);
    rt.process(&mut [0.0; 128]);
    let mut expected = vec![
        vec![0x94, 60, 98],
        vec![0x95, 60, 88],
        vec![0x84, 60, 37],
        vec![0x85, 60, 41],
    ];
    expected.extend(source.iter().map(|bytes| {
        super::super::packet::Packet::new(bytes)
            .unwrap()
            .with_channel(Some(4))
            .bytes()
            .to_vec()
    }));
    until(|| {
        let actual = trace.lock();
        expected.iter().all(|packet| actual.contains(packet))
    });
    a.send(&[0x90, 64, 100]).unwrap();
    until(|| engine.midi.input_stats().dispatched >= 13);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt, 2), 1);
    routing.routes[0].track = 5;
    routing.routes[0].output_channel = Some(7);
    let generation = engine.midi.configure_routing(routing).unwrap();
    until(|| {
        engine
            .midi
            .routing_status()
            .is_some_and(|s| !s.pending && s.applied_generation == generation)
    });
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt, 2), 0);
    until(|| trace.lock().iter().any(|p| p == &[0xb4, 123, 0]));
    let activity = engine.cmd.midi_routing().activity();
    let shared = engine.cmd.midi_routing().clone();
    engine.midi = crate::engine::midi::MidiHub::without_devices();
    until(|| !shared.alive.load(Acquire));
    drop(a);
    drop(b);
    drop(sink);
    until(|| {
        let retired = Midir.discover().unwrap();
        !retired.inputs.iter().any(|p| p.name.contains(&token))
            && !retired
                .outputs
                .iter()
                .any(|p| p.endpoint.name.contains(&token))
    });
    if let Ok(path) = std::env::var("OMAT_MIDI_ROUTING_NATIVE_TRACE") {
        std::fs::write(path,serde_json::to_vec_pretty(&serde_json::json!({"platform":format!("Linux ALSA {}, midir0.10.4",std::env::consts::ARCH),"physical_hardware":false,"private_ports_only":true,"ports_retired":true,"expected_live_packets":expected,"observed_packets":trace.lock().clone(),"activity":activity})).unwrap()).unwrap();
    }
}

#[test]
fn disconnect_releases_only_its_owned_notes_and_sustain() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let fixture = Arc::new(Fixture::default());
    let mut routes = config();
    routes.routes[1].output_channel = Some(4);
    let manager =
        Manager::start_backend(engine.cmd.clone(), routes, Fake(fixture.clone())).unwrap();
    until(|| !manager.status().pending);
    let (mut a, ga, ca) = input(&engine, 501, "Keyboard A", "100:0");
    let (mut b, gb, cb) = input(&engine, 502, "Keyboard B", "101:0");
    a.push(&[0x90, 60, 100]);
    a.push(&[0xb0, 64, 127]);
    b.push(&[0x90, 60, 80]);
    b.push(&[0x90, 62, 90]);
    b.push(&[0xb0, 64, 127]);
    until(|| ca.snapshot().dispatched == 2 && cb.snapshot().dispatched == 3);
    until(|| fixture.trace.lock().len() == 4);
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt, 2), 1);
    assert_eq!(held(&rt, 3), 2);
    drop(a);
    drop(ga);
    until(|| engine.cmd.midi_routing().live.lock().sources.len() == 1);
    b.push(&[0xb0, 1, 23]);
    until(|| fixture.trace.lock().iter().any(|p| p == &[0xb4, 1, 23]));
    rt.process(&mut [0.0; 128]);
    assert_eq!(held(&rt, 2), 0);
    assert_eq!(held(&rt, 3), 2);
    assert!(!fixture
        .trace
        .lock()
        .iter()
        .any(|p| p[0] & 0xf0 == 0x80 || p == &[0xb4, 64, 0] || p[0] & 0xf0 == 0xb0 && p[1] == 123));
    drop(b);
    drop(gb);
    until(|| fixture.trace.lock().iter().any(|p| p == &[0xb4, 64, 0]));
    let trace = fixture.trace.lock();
    assert_eq!(trace.iter().filter(|p| p[0] & 0xf0 == 0x80).count(), 2);
    drop(trace);
    drop(manager);
}
#[test]
fn feedback_guard_checks_active_inputs_and_preserves_fast_identical_messages() {
    let (engine, _rt) = Engine::headless_for_test(48000, 256);
    let fixture = Arc::new(Fixture::default());
    let manager = Manager::start_backend(
        engine.cmd.clone(),
        Routing::default(),
        Fake(fixture.clone()),
    )
    .unwrap();
    until(|| !manager.status().pending);
    let (synth, guard, _) = input(&engine, 601, "Synth controls", "300:5");
    manager.configure(config()).unwrap();
    until(|| !manager.status().pending);
    assert!(manager
        .status()
        .error
        .as_ref()
        .unwrap()
        .contains("Feedback"));
    assert_eq!(fixture.opens.load(Relaxed), 0);
    drop(synth);
    drop(guard);
    until(|| engine.cmd.midi_routing().live.lock().sources.is_empty());
    let mut routes = config();
    routes.routes[0].output_channel = None;
    let generation = manager.configure(routes).unwrap();
    until(|| manager.status().applied_generation == generation);
    let denied = handoff::start_on_port(
        602,
        pick_map(&builtin_maps().unwrap(), "Generic Keyboard"),
        engine.cmd.clone(),
        engine.midi.log.clone(),
        engine.midi.learn.clone(),
        "Synth controls".into(),
        "300:5".into(),
        Arc::new(handoff::InputCounters::default()),
        true,
        || {},
    );
    assert!(
        denied.is_err(),
        "an input was enabled on an active output client"
    );
    let (mut a, guard, counters) = input(&engine, 603, "Keyboard A", "100:0");
    for _ in 0..4 {
        a.push(&[0x90, 60, 100]);
        a.push(&[0x80, 60, 37]);
        a.push(&[0xb0, 1, 64]);
    }
    until(|| counters.snapshot().dispatched == 12);
    until(|| fixture.trace.lock().len() == 12);
    let trace = fixture.trace.lock();
    assert_eq!(
        trace.iter().filter(|p| p == &&vec![0x90, 60, 100]).count(),
        4
    );
    assert_eq!(
        trace.iter().filter(|p| p == &&vec![0x80, 60, 37]).count(),
        4
    );
    assert_eq!(trace.iter().filter(|p| p == &&vec![0xb0, 1, 64]).count(), 4);
    drop(trace);
    drop(a);
    drop(guard);
    drop(manager);
}
#[test]
fn explicit_rescan_retires_vanished_output_and_requires_exact_reconnect() {
    let (engine, _rt) = Engine::headless_for_test(48000, 256);
    let fixture = Arc::new(Fixture::default());
    let manager =
        Manager::start_backend(engine.cmd.clone(), config(), Fake(fixture.clone())).unwrap();
    until(|| !manager.status().pending);
    assert!(engine.cmd.midi_routing().emit(
        2,
        super::super::packet::Packet::new(&[0x90, 60, 100]).unwrap()
    ));
    until(|| fixture.trace.lock().len() == 1);
    fixture.missing.store(true, Release);
    manager.configure(config()).unwrap();
    until(|| !manager.status().pending);
    assert!(manager.status().error.as_ref().unwrap().contains("missing"));
    assert_eq!(engine.cmd.midi_routing().output_state().0, 0);
    assert_eq!(fixture.closes.load(Relaxed), 1);
    assert!(fixture.trace.lock().iter().any(|p| p == &[0xb4, 123, 0]));
    assert!(!engine.cmd.midi_routing().emit(
        2,
        super::super::packet::Packet::new(&[0x90, 61, 100]).unwrap()
    ));
    fixture.missing.store(false, Release);
    let generation = manager.configure(config()).unwrap();
    until(|| manager.status().applied_generation == generation);
    assert!(manager.status().error.is_none());
    assert_eq!(fixture.opens.load(Relaxed), 2);
    assert!(engine.cmd.midi_routing().emit(
        2,
        super::super::packet::Packet::new(&[0x90, 62, 99]).unwrap()
    ));
    until(|| fixture.trace.lock().iter().any(|p| p == &[0x94, 62, 99]));
    assert!(!fixture.trace.lock().iter().any(|p| p == &[0x94, 61, 100]));
    drop(manager);
}
#[test]
fn bounded_output_overflow_discards_queued_onsets_and_keeps_renderer_progress() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let fixture = Arc::new(Fixture::default());
    let manager =
        Manager::start_backend(engine.cmd.clone(), config(), Fake(fixture.clone())).unwrap();
    until(|| !manager.status().pending);
    fixture.send_blocked.store(true, Release);
    let packet = super::super::packet::Packet::new(&[0x90, 60, 100]).unwrap();
    assert!(engine.cmd.midi_routing().emit(2, packet));
    until(|| fixture.send_entered.load(Acquire));
    let mut admitted = 0;
    let measured = test_alloc::measure(|| {
        for _ in 0..2049 {
            if engine.cmd.midi_routing().emit(2, packet) {
                admitted += 1;
            }
        }
        for _ in 0..128 {
            rt.process(&mut [0.0; 128]);
        }
    });
    assert_eq!(measured, test_alloc::Counts::default());
    assert_eq!(admitted, 2048);
    assert_eq!(engine.cmd.midi_routing().activity().0[2].overruns, 1);
    fixture.send_blocked.store(false, Release);
    until(|| fixture.trace.lock().iter().any(|p| p == &[0xb4, 123, 0]));
    std::thread::sleep(Duration::from_millis(40));
    assert_eq!(
        fixture
            .trace
            .lock()
            .iter()
            .filter(|p| p[0] & 0xf0 == 0x90)
            .count(),
        1,
        "obsolete output onsets replayed after reset"
    );
    drop(manager);
}

#[test]
fn cancel_after_publication_claim_reports_applied_and_recovery_resets_output() {
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    let fixture = Arc::new(Fixture::default());
    let manager =
        Manager::start_backend(engine.cmd.clone(), config(), Fake(fixture.clone())).unwrap();
    until(|| !manager.status().pending);
    fixture.send_blocked.store(true, Release);
    fixture.send_entered.store(false, Release);
    let mut routes = config();
    routes.routes[0].output_channel = Some(7);
    let generation = manager.configure(routes.clone()).unwrap();
    until(|| fixture.send_entered.load(Acquire));
    assert!(manager.status().pending);
    assert!(
        !manager.cancel(),
        "cancel rewrote an already claimed publication"
    );
    fixture.send_blocked.store(false, Release);
    until(|| !manager.status().pending);
    assert_eq!(manager.status().applied_generation, generation);
    assert_eq!(*manager.status().applied, routes);
    let (mut a, guard, counters) = input(&engine, 701, "Keyboard A", "100:0");
    a.push(&[0x90, 60, 100]);
    until(|| counters.snapshot().dispatched == 1);
    until(|| fixture.trace.lock().iter().any(|p| p == &[0x97, 60, 100]));
    engine
        .cmd
        .send(Command::SafetyStop(
            crate::engine::performance::Safety::Stop,
        ))
        .unwrap();
    rt.process(&mut [0.0; 128]);
    until(|| {
        fixture
            .trace
            .lock()
            .iter()
            .filter(|p| p == &&vec![0xb7, 123, 0])
            .count()
            >= 2
    });
    fixture.trace.lock().clear();
    a.push(&[0xc0, 9]);
    a.push(&[0xb0, 1, 64]);
    until(|| counters.snapshot().dispatched == 3);
    assert!(fixture.trace.lock().is_empty());
    assert!(engine.cmd.performance().status().recovery);
    drop(a);
    drop(guard);
    drop(manager);
}
