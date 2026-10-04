//! Connection management publishes status outside MIDI/audio callbacks. GUI,
//! status requests and shell followers all read the same shared snapshot field.
use crate::engine::Snapshot;
use parking_lot::Mutex;
use std::sync::{Arc, Weak};

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Discovered,
    Connecting,
    Connected,
    Failed(String),
    Disconnected,
    Disabled,
}

struct Inner {
    snapshot: Weak<Mutex<Snapshot>>,
    index: usize,
    name: String,
    map: String,
    state: Mutex<State>,
}

#[derive(Clone)]
pub(crate) struct Status(Arc<Inner>);

impl Status {
    pub(crate) fn new(snapshot: &Arc<Mutex<Snapshot>>, name: &str, map: &str) -> Self {
        Self::with_state(snapshot, name, map, State::Connecting)
    }

    pub(crate) fn discovered(snapshot: &Arc<Mutex<Snapshot>>, name: &str, map: &str) -> Self {
        Self::with_state(snapshot, name, map, State::Discovered)
    }

    fn with_state(snapshot: &Arc<Mutex<Snapshot>>, name: &str, map: &str, state: State) -> Self {
        let mut shared = snapshot.lock();
        let index = shared.midi.len();
        shared.midi.push(Self::label(name, map, &state));
        Self(Arc::new(Inner {
            snapshot: Arc::downgrade(snapshot),
            index,
            name: name.into(),
            map: map.into(),
            state: Mutex::new(state),
        }))
    }

    fn label(name: &str, map: &str, state: &State) -> String {
        let state = match state {
            State::Discovered => "discovered",
            State::Connecting => "connecting",
            State::Connected => "connected",
            State::Failed(_) => "failed",
            State::Disconnected => "disconnected",
            State::Disabled => "disabled",
        };
        format!("[{state}] {name} · {map}")
    }

    fn update(&self, next: State) {
        // Serialize transitions and publication together. A late successful
        // connect cannot hide a worker that already ended; its failed-connect
        // diagnostic likewise survives that worker's completion notification.
        let mut state = self.0.state.lock();
        if next == State::Connected && *state != State::Connecting {
            return;
        }
        if next == State::Disconnected && matches!(*state, State::Failed(_)) {
            return;
        }
        *state = next;
        if let Some(snapshot) = self.0.snapshot.upgrade() {
            let mut label = Self::label(&self.0.name, &self.0.map, &state);
            if let State::Failed(error) = &*state {
                label.push_str(": ");
                label.push_str(error);
            }
            let mut shared = snapshot.lock();
            if let Some(entry) = shared.midi.get_mut(self.0.index) {
                *entry = label;
            }
        }
    }

    pub(crate) fn connecting(&self) {
        self.update(State::Connecting);
    }
    pub(crate) fn is_connected(&self) -> bool {
        *self.0.state.lock() == State::Connected
    }
    pub(crate) fn connected(&self) {
        self.update(State::Connected);
    }
    pub(crate) fn failed(&self, error: impl ToString) {
        self.update(State::Failed(error.to_string()));
    }
    pub(crate) fn disconnected(&self) {
        self.update(State::Disconnected);
    }
    pub(crate) fn disabled(&self) {
        self.update(State::Disabled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{test_alloc, Engine};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    fn wire(engine: &Engine) -> serde_json::Value {
        let (mut client, server) = UnixStream::pair().unwrap();
        let commands = engine.cmd.clone();
        let snapshot = engine.snap.clone();
        let handler = std::thread::spawn(move || {
            crate::handle_client_with_limits(
                server,
                commands,
                snapshot,
                crate::ipc_transport::Limits::default(),
            )
            .unwrap();
        });
        client
            .write_all(b"{\"op\":\"status\",\"id\":\"midi-consistency\"}\n")
            .unwrap();
        let mut line = String::new();
        let mut reader = BufReader::new(client);
        reader.read_line(&mut line).unwrap();
        drop(reader);
        handler.join().unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn shared_midi_status_matches_gui_protocol_and_survives_audio_publication() {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 48);
        let status = Status::new(&engine.snap, "MIDI keyboard", "Generic");
        let mut evidence = Vec::new();
        for state in [
            State::Discovered,
            State::Connecting,
            State::Connected,
            State::Failed("open denied".into()),
            State::Disconnected,
        ] {
            // An explicit test transition permits checking every external state;
            // production failure/completion ordering is tested separately below.
            *status.0.state.lock() = State::Connecting;
            status.update(state.clone());
            // No GUI snapshot call is required to publish the device list.
            let expected = engine.snap.lock().midi.clone();
            let before = wire(&engine);
            assert_eq!(before["midi"], serde_json::json!(expected));
            rt.publish_for_test();
            let gui = engine.snapshot();
            assert_eq!(gui.midi, expected);
            assert_eq!(engine.snap.lock().midi, expected);
            let after = wire(&engine);
            assert_eq!(after["midi"], serde_json::json!(gui.midi));
            assert_eq!(after["state_truncated"], false);
            evidence.push(after);
        }
        // The optional real-Quickshell script owns this private output path.
        if let Ok(path) = std::env::var("OMATAINER_MIDI_STATUS_EVIDENCE") {
            std::fs::write(path, serde_json::to_vec(&evidence).unwrap()).unwrap();
        }
    }

    #[test]
    fn midi_status_preserves_failure_and_distinguishes_identical_devices() {
        let (engine, _rt) = Engine::headless_for_test(48_000, 48);
        let first = Status::new(&engine.snap, "same keyboard", "Generic");
        let second = Status::new(&engine.snap, "same keyboard", "Generic");
        first.connected();
        second.failed("permission denied");
        second.disconnected();
        second.connected();
        let values = engine.snapshot().midi;
        assert!(values[0].starts_with("[connected]"));
        assert!(values[1].starts_with("[failed]"));
        assert!(values[1].contains("permission denied"));
        first.disconnected();
        first.connected();
        assert!(engine.snapshot().midi[0].starts_with("[disconnected]"));
        let third = Status::new(&engine.snap, "third", "Generic");
        third.disconnected();
        third.failed("connect failed");
        assert!(engine.snapshot().midi[2].starts_with("[failed]"));
        let old = Arc::downgrade(&engine.snap);
        drop(engine);
        // Status ownership itself never extends the public snapshot lifetime.
        drop(_rt);
        let until = Instant::now() + Duration::from_secs(2);
        while old.upgrade().is_some() {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        first.disconnected();
    }

    #[test]
    fn midi_worker_completion_reports_disconnect_without_callback_status_work() {
        let (engine, mut rt) = Engine::headless_for_test(48_000, 48);
        let status = Status::new(&engine.snap, "synthetic keyboard", "Generic");
        let completed = status.clone();
        let counters = Arc::new(super::super::handoff::InputCounters::default());
        let map = super::super::pick_map(&super::super::builtin_maps().unwrap(), "MIDI keyboard");
        let (mut input, guard) = super::super::handoff::start_with_completion(
            42,
            map,
            engine.cmd.clone(),
            engine.midi.log.clone(),
            "synthetic keyboard".into(),
            counters,
            move || completed.disconnected(),
        )
        .unwrap();
        status.connected();
        let held = engine.snap.lock();
        // Neither incoming bytes nor audio publication take the status lock.
        let counts = test_alloc::measure(|| {
            input.push(&[0x90, 60, 90]);
            for _ in 0..64 {
                rt.process(&mut [0.0; 128]);
            }
        });
        assert_eq!(counts, test_alloc::Counts::default());
        drop(input);
        let finished = std::sync::mpsc::channel();
        let join = std::thread::spawn(move || {
            drop(guard);
            finished.0.send(()).unwrap();
        });
        assert!(finished.1.recv_timeout(Duration::from_millis(20)).is_err());
        assert!(held.midi[0].starts_with("[connected]"));
        drop(held);
        finished.1.recv_timeout(Duration::from_secs(2)).unwrap();
        join.join().unwrap();
        assert!(engine.snapshot().midi[0].starts_with("[disconnected]"));
        rt.publish_for_test();
        assert!(engine.snapshot().midi[0].starts_with("[disconnected]"));
    }
}
