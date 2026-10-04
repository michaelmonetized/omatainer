//! External drivers start private real servers; these tests use the production adapter.
use super::*;
use crate::engine::{Command, Engine};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Default)]
pub(super) struct Trace {
    pub callbacks: AtomicU64,
    pub allocations: AtomicU64,
    pub frees: AtomicU64,
    first_signal_ns: AtomicU64,
    pub input: bool,
}
static TRACES: OnceLock<Mutex<Vec<Arc<Trace>>>> = OnceLock::new();
pub(super) fn new_trace(input: bool) -> Arc<Trace> {
    let trace = Arc::new(Trace {
        input,
        ..Default::default()
    });
    TRACES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap()
        .push(trace.clone());
    trace
}
pub(super) fn observe(trace: &Trace, data: &[f32], channels: usize, rate: u32) {
    if trace.first_signal_ns.load(Ordering::Relaxed) != 0 {
        return;
    }
    if let Some(frame) = data
        .chunks_exact(channels)
        .position(|frame| frame.iter().any(|sample| sample.abs() > 0.0002))
    {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } == 0 {
            let ns = time.tv_sec as u64 * 1_000_000_000
                + time.tv_nsec as u64
                + frame as u64 * 1_000_000_000 / u64::from(rate);
            trace.first_signal_ns.store(ns, Ordering::Release);
        }
    }
}
#[track_caller]
fn wait(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !condition() {
        assert!(Instant::now() < deadline, "Private graph fixture timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn directory() -> PathBuf {
    PathBuf::from(
        std::env::var_os("OMATAINER_NATIVE_GRAPH_DIR").expect("Use the private backend driver"),
    )
}
fn settings() -> crate::preferences::Audio {
    crate::preferences::Audio {
        backend: Some(BACKEND.into()),
        channels: Some(2),
        graph: graph::Routes {
            outputs: vec![
                graph::Link {
                    channel: 0,
                    endpoint: std::env::var("OMATAINER_GRAPH_OUTPUT").unwrap(),
                },
                graph::Link {
                    channel: 1,
                    endpoint: "FutureInterface:playback_02".into(),
                },
            ],
            inputs: vec![graph::Link {
                channel: 0,
                endpoint: std::env::var("OMATAINER_GRAPH_INPUT").unwrap(),
            }],
        },
        ..Default::default()
    }
}

#[test]
#[ignore = "Requires scripts/check-linux-audio.py with a private JACK or PipeWire server"]
fn private_graph_loopback_quantum_reconnect_and_rate_change_retain_project() {
    let root = directory();
    TRACES.get_or_init(Mutex::default).lock().unwrap().clear();
    let saved = settings();
    let mut preferences = crate::preferences::Preferences::defaults(&root);
    preferences.profiles.get_mut("Studio").unwrap().audio = saved.clone();
    preferences.validate().unwrap();
    let encoded = serde_json::to_vec(&preferences).unwrap();
    std::fs::write(root.join("preferences.json"), &encoded).unwrap();
    let reopened: crate::preferences::Preferences =
        serde_json::from_slice(&std::fs::read(root.join("preferences.json")).unwrap()).unwrap();
    assert_eq!(reopened.current().unwrap().audio, saved);
    let (engine, mut rt) = Engine::headless_for_test(48000, 256);
    rt.apply(Command::DeckAudio {
        deck: 0,
        audio: Arc::new(crate::engine::Sample {
            name: "Retained native probe".into(),
            path: String::new(),
            sr: 48000,
            ch: 2,
            bpm: 120.0,
            peaks: Arc::new(Vec::new()),
            data: (0..48000)
                .flat_map(|frame| {
                    let sample = 0.005 * (frame as f32 * 0.071).sin();
                    [sample, sample]
                })
                .collect(),
        }),
    });
    let audio = super::super::start_with_settings(rt, &saved).unwrap();
    wait(|| engine.cmd.audio_metrics().callbacks > 10);
    let inspector = Client::open("OmatainerVerify").unwrap();
    let connections = |channel| {
        unsafe {
            names(j::jack_port_get_all_connections(
                inspector.0,
                inspector.port(&port_name(false, channel)).unwrap(),
            ))
        }
        .unwrap()
    };
    assert_eq!(
        connections(0),
        vec![std::env::var("OMATAINER_GRAPH_OUTPUT").unwrap()]
    );
    assert!(
        connections(1).is_empty(),
        "Missing endpoint fell back to another sink"
    );
    let mut wrong = saved.graph.clone();
    wrong.outputs[0].endpoint = std::env::var("OMATAINER_GRAPH_INPUT").unwrap();
    assert!(inspector.validate_routes(&wrong).is_err());
    let (_, duplicate) = Engine::headless_for_test(48000, 256);
    assert!(
        super::super::start_with_settings(duplicate, &saved).is_err(),
        "Duplicate exact client name was silently renamed"
    );
    let future = Client::open("FutureInterface").unwrap();
    let port = unsafe {
        j::jack_port_register(
            future.0,
            c"playback_02".as_ptr(),
            AUDIO.as_ptr().cast(),
            j::JackPortIsInput.into(),
            0,
        )
    };
    assert!(!port.is_null());
    assert_eq!(
        unsafe { j::jack_set_process_callback(future.0, Some(discard), std::ptr::null_mut()) },
        0
    );
    assert_eq!(unsafe { j::jack_activate(future.0) }, 0);
    wait(|| connections(1) == vec!["FutureInterface:playback_02"]);
    for _ in 0..3 {
        inspector.restore(&saved.graph, false).unwrap();
    }
    assert_eq!(connections(1), vec!["FutureInterface:playback_02"]);
    unsafe {
        j::jack_deactivate(future.0);
        j::jack_port_unregister(future.0, port);
    }
    drop(future);
    wait(|| connections(1).is_empty());
    drop(inspector);
    let status = audio.handle.status();
    let output = &status.active.as_ref().unwrap().plan;
    let input_saved = super::super::routing::model::InputConfig {
        backend: BACKEND.into(),
        device: DEVICE.into(),
        channels: 2,
        format: crate::preferences::AudioFormat::F32,
        buffer_frames: None,
    };
    let plan = input::preview(&input_saved, &discover().unwrap(), output).unwrap();
    audio
        .input
        .apply(
            Some(plan),
            status.generation,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
    wait(|| TRACES.get().unwrap().lock().unwrap().len() == 2);
    wait(|| engine.cmd.send(Command::DeckPlay { deck: 0 }).is_ok());
    wait(|| {
        TRACES
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .iter()
            .all(|trace| trace.first_signal_ns.load(Ordering::Acquire) != 0)
    });
    let traces = TRACES.get().unwrap().lock().unwrap().clone();
    let sent = traces
        .iter()
        .find(|trace| !trace.input)
        .unwrap()
        .first_signal_ns
        .load(Ordering::Acquire);
    let returned = traces
        .iter()
        .find(|trace| trace.input)
        .unwrap()
        .first_signal_ns
        .load(Ordering::Acquire);
    assert!(returned >= sent);
    let latency = returned - sent;
    assert!(
        latency < 100_000_000,
        "Backend software loopback exceeded 100ms: {latency}"
    );
    engine.cmd.send(Command::DeckPlay { deck: 0 }).unwrap();
    engine.cmd.send(Command::Stop).unwrap();
    wait(|| !engine.snap.lock().playing && !engine.snap.lock().decks[0].playing);
    std::fs::write(
        root.join("quantum.ready"),
        b"Change the owned server quantum\n",
    )
    .unwrap();
    wait(|| root.join("quantum.changed").exists());
    wait(|| {
        audio
            .handle
            .status()
            .active
            .as_ref()
            .is_some_and(|active| active.plan.buffer == Some(256))
    });
    assert_eq!(audio.handle.status().generation, status.generation);
    let before = engine.project.capture(&AtomicBool::new(false)).unwrap();
    assert!(before.state.decks[0].audio.is_some());
    std::fs::write(
        root.join("restart.ready"),
        b"Restart only the private server at 44100 Hz\n",
    )
    .unwrap();
    wait(|| root.join("restart.done").exists());
    wait(|| audio.handle.status().phase == super::super::owner::Phase::Offline);
    let offline = engine.project.capture(&AtomicBool::new(false)).unwrap();
    assert_eq!(
        offline.media[offline.state.decks[0].audio.unwrap()].name,
        "Retained native probe"
    );
    let state = audio.handle.status();
    let result = audio
        .handle
        .reconnect_permitted(
            Arc::new(AtomicBool::new(false)),
            audio.handle.performance_permit().unwrap(),
            state.generation,
        )
        .unwrap();
    assert_eq!(result.active.as_ref().unwrap().plan.rate, 44100);
    assert_eq!(engine.project.sample_rate(), 44100);
    assert!(!engine.snap.lock().decks[0].playing);
    let retained = engine.project.capture(&AtomicBool::new(false)).unwrap();
    assert_eq!(
        retained.media[retained.state.decks[0].audio.unwrap()].name,
        "Retained native probe"
    );
    wait(|| engine.cmd.audio_metrics().callbacks > 75);
    assert!(Arc::ptr_eq(
        &before.media[before.state.decks[0].audio.unwrap()],
        &retained.media[retained.state.decks[0].audio.unwrap()]
    ));
    let traces = TRACES.get().unwrap().lock().unwrap().clone();
    let report = serde_json::json!({ "backend_identity": identity(), "initial_rate": 48000, "initial_quantum": 128, "changed_quantum": 256, "reconnected_rate": 44100, "software_loopback_return_ns": latency,
        "callbacks": traces.iter().map(|trace| trace.callbacks.load(Ordering::Acquire)).collect::<Vec<_>>(),
        "callback_allocations": traces.iter().map(|trace| trace.allocations.load(Ordering::Acquire)).collect::<Vec<_>>(),
        "callback_frees": traces.iter().map(|trace| trace.frees.load(Ordering::Acquire)).collect::<Vec<_>>(), "saved_routes_reopened": true, "missing_endpoint_returned_once": true, "duplicate_client_refused": true, "wrong_port_direction_refused": true, "project_media_retained": true, "playback_resumed": false });
    for trace in &traces {
        assert_eq!(trace.allocations.load(Ordering::Acquire), 0);
        assert_eq!(trace.frees.load(Ordering::Acquire), 0);
    }
    std::fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(&report).unwrap(),
    )
    .unwrap();
    drop(audio);
    super::super::owner::finish_shutdown();
}

/// Consume a private fixture sink without producing a return route.
/// Takes a server block and unused context; returns continuation with no audio output ports.
unsafe extern "C" fn discard(_: u32, _: *mut libc::c_void) -> libc::c_int {
    0
}

/// Construct a real graph owner for shipped UI tests.
/// Takes saved audio settings; returns a test engine using the production native backend and no MIDI devices.
pub(crate) fn native_engine(settings: &crate::preferences::Audio) -> Engine {
    let (mut engine, rt) = Engine::headless_for_test(48000, 256);
    engine._audio = Some(super::super::start_with_settings(rt, settings).unwrap());
    engine
}
