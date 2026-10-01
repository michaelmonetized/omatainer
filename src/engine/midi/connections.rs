//! One management worker owns OS discovery, connection attempts and teardown.
//! Only the existing fixed-work InputSink runs in a raw MIDI callback.
use super::{device_status::Status, handoff, next_source_id, pick_map, MidiMap};
use crate::engine::{CommandPort, Snapshot};
use crossbeam_channel::{bounded, Sender};
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use parking_lot::Mutex;
use std::sync::atomic::{
    AtomicBool,
    Ordering::{AcqRel, Acquire, Release},
};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retry {
    Queued,
    AlreadyRunning,
    Unavailable,
}

struct Activity {
    busy: AtomicBool,
    alive: AtomicBool,
    stop: AtomicBool,
}
struct WorkerLife(Arc<Activity>);
impl Drop for WorkerLife {
    fn drop(&mut self) {
        self.0.alive.store(false, Release);
        self.0.busy.store(false, Release);
    }
}

pub(super) struct Manager {
    requests: Option<Sender<()>>,
    activity: Arc<Activity>,
    worker: Option<JoinHandle<()>>,
}

impl Manager {
    pub(super) fn start<B: Backend>(
        backend: B,
        snapshot: &Arc<Mutex<Snapshot>>,
        cmd: CommandPort,
        maps: Vec<MidiMap>,
        log: Arc<Mutex<Vec<String>>>,
        learn: Arc<Mutex<Option<String>>>,
        counters: Arc<handoff::InputCounters>,
    ) -> std::io::Result<Self> {
        // Keyboard and mouse remain usable with or without connected hardware.
        Status::new(snapshot, "keyboard + mouse", "built-in").connected();
        let activity = Arc::new(Activity {
            busy: AtomicBool::new(true),
            alive: AtomicBool::new(true),
            stop: AtomicBool::new(false),
        });
        let (requests, receiver) = bounded(1);
        let shared = activity.clone();
        let snapshot = Arc::downgrade(snapshot);
        let worker = std::thread::Builder::new()
            .name("omatainer-midi-connect".into())
            .spawn(move || {
                let _life = WorkerLife(shared.clone());
                let mut worker = Worker {
                    backend,
                    snapshot,
                    cmd,
                    maps,
                    log,
                    learn,
                    counters,
                    entries: Vec::new(),
                    backend_status: None,
                    activity: shared.clone(),
                };
                loop {
                    if shared.stop.load(Acquire) {
                        break;
                    }
                    worker.refresh();
                    shared.busy.store(false, Release);
                    if receiver.recv().is_err() {
                        break;
                    }
                }
                // Connections close, then input worker guards join, on this worker.
            })?;
        Ok(Self {
            requests: Some(requests),
            activity,
            worker: Some(worker),
        })
    }
    pub(super) fn retry(&self) -> Retry {
        if !self.available() {
            return Retry::Unavailable;
        }
        if self
            .activity
            .busy
            .compare_exchange(false, true, AcqRel, Acquire)
            .is_err()
        {
            return Retry::AlreadyRunning;
        }
        if self
            .requests
            .as_ref()
            .is_some_and(|tx| tx.try_send(()).is_ok())
        {
            Retry::Queued
        } else {
            self.activity.busy.store(false, Release);
            Retry::Unavailable
        }
    }
    pub(super) fn busy(&self) -> bool {
        self.activity.busy.load(Acquire)
    }
    pub(super) fn available(&self) -> bool {
        self.activity.alive.load(Acquire)
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        self.activity.stop.store(true, Release);
        self.requests.take(); // Wakes an idle worker without waiting for its queue.
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            }
            // An OS call cannot be forcibly cancelled. The worker retains all
            // connection/guard ownership and closes them after the call returns.
            // Never join a blocked backend or input-worker teardown on the GUI.
        }
    }
}

pub(super) struct Port<P> {
    id: String,
    name: String,
    port: P,
}
pub(super) trait Backend: Send + 'static {
    type Port;
    type Connection;
    fn discover(&mut self) -> Result<Vec<Port<Self::Port>>, String>;
    fn connect(
        &mut self,
        port: &Self::Port,
        name: &str,
        input: handoff::InputSink,
    ) -> Result<Self::Connection, String>;
    fn refresh_output(&mut self) {}
}
struct Active<C> {
    _connection: C, // Field order is intentional: stop callbacks before joining.
    _worker: handoff::InputGuard,
}
struct Entry<P, C> {
    id: String,
    name: String,
    port: P,
    map: MidiMap,
    status: Status,
    active: Option<Active<C>>,
    present: bool,
}
struct Worker<B: Backend> {
    backend: B,
    snapshot: Weak<Mutex<Snapshot>>,
    cmd: CommandPort,
    maps: Vec<MidiMap>,
    log: Arc<Mutex<Vec<String>>>,
    learn: Arc<Mutex<Option<String>>>,
    counters: Arc<handoff::InputCounters>,
    entries: Vec<Entry<B::Port, B::Connection>>,
    backend_status: Option<Status>,
    activity: Arc<Activity>,
}
impl<B: Backend> Worker<B> {
    fn refresh(&mut self) {
        if let Some(status) = &self.backend_status {
            status.connecting();
        }
        let ports = match self.backend.discover() {
            Ok(ports) => {
                if let Some(status) = &self.backend_status {
                    status.connected();
                }
                ports
            }
            Err(error) => {
                if self.backend_status.is_none() {
                    if let Some(snapshot) = self.snapshot.upgrade() {
                        self.backend_status =
                            Some(Status::new(&snapshot, "MIDI input backend", "discovery"));
                    }
                }
                if let Some(status) = &self.backend_status {
                    status.failed(error);
                }
                return;
            }
        };
        if self.activity.stop.load(Acquire) {
            return;
        }
        for entry in &mut self.entries {
            entry.present = false;
        }
        // Publish all discoveries before connecting the first port. A slow or
        // failing connection does not falsely imply other ports are connected.
        for port in ports {
            if port.name.to_lowercase().contains("through") {
                continue;
            }
            if let Some(entry) = self
                .entries
                .iter_mut()
                .find(|entry| entry.id == port.id && entry.name == port.name)
            {
                entry.port = port.port;
                entry.present = true;
            } else if let Some(snapshot) = self.snapshot.upgrade() {
                let map = pick_map(&self.maps, &port.name);
                let status = Status::discovered(&snapshot, &port.name, &map.name);
                self.entries.push(Entry {
                    id: port.id,
                    name: port.name,
                    port: port.port,
                    map,
                    status,
                    active: None,
                    present: true,
                });
            }
        }
        for entry in &mut self.entries {
            if self.activity.stop.load(Acquire) {
                return;
            }
            if !entry.present {
                entry.active.take();
                entry.status.disconnected();
                continue;
            }
            if entry.active.is_some() && entry.status.is_connected() {
                continue;
            }
            // Complete any prior callback/worker before resetting this status;
            // a late completion can never overwrite a subsequent successful try.
            entry.active.take();
            entry.status.connecting();
            let completed = entry.status.clone();
            let pair = handoff::start_with_completion(
                next_source_id(),
                entry.map.clone(),
                self.cmd.clone(),
                self.log.clone(),
                self.learn.clone(),
                entry.name.clone(),
                self.counters.clone(),
                move || completed.disconnected(),
            );
            let (input, worker) = match pair {
                Ok(pair) => pair,
                Err(error) => {
                    entry.status.failed(error);
                    continue;
                }
            };
            match self.backend.connect(&entry.port, &entry.name, input) {
                Ok(connection) => {
                    entry.status.connected();
                    entry.active = Some(Active {
                        _connection: connection,
                        _worker: worker,
                    });
                }
                Err(error) => {
                    entry.status.failed(error);
                    drop(worker); // failure survives completion; retry waits for it
                }
            }
        }
        if !self.activity.stop.load(Acquire) {
            self.backend.refresh_output();
        }
    }
}

pub(super) struct MidirBackend {
    outs: Arc<Mutex<Vec<MidiOutputConnection>>>,
}
impl MidirBackend {
    pub(super) fn new(outs: Arc<Mutex<Vec<MidiOutputConnection>>>) -> Self {
        Self { outs }
    }
}
impl Drop for MidirBackend {
    fn drop(&mut self) {
        // Output ownership is also retired by the management worker, even if
        // MidiHub's public output handle outlives that worker briefly.
        self.outs.lock().clear();
    }
}
impl Backend for MidirBackend {
    type Port = midir::MidiInputPort;
    type Connection = MidiInputConnection<()>;
    fn discover(&mut self) -> Result<Vec<Port<Self::Port>>, String> {
        let probe = MidiInput::new("omatainer-discover").map_err(|error| error.to_string())?;
        probe
            .ports()
            .into_iter()
            .map(|port| {
                let name = probe.port_name(&port).map_err(|error| error.to_string())?;
                Ok(Port {
                    id: port.id(),
                    name,
                    port,
                })
            })
            .collect()
    }
    fn connect(
        &mut self,
        port: &Self::Port,
        name: &str,
        mut input: handoff::InputSink,
    ) -> Result<Self::Connection, String> {
        let mut midi = MidiInput::new("omatainer-input").map_err(|error| error.to_string())?;
        midi.ignore(Ignore::None);
        midi.connect(
            port,
            &format!("omatainer-in-{name}"),
            move |_time, message, _| input.push(message),
            (),
        )
        .map_err(|error| error.to_string())
    }
    fn refresh_output(&mut self) {
        if !self.outs.lock().is_empty() {
            return;
        }
        if let Ok(probe) = MidiOutput::new("omatainer-output") {
            let ports: Vec<_> = probe
                .ports()
                .into_iter()
                .filter_map(|port| probe.port_name(&port).ok().map(|name| (name, port)))
                .collect();
            drop(probe);
            for (name, port) in ports {
                if name.to_lowercase().contains("through") {
                    continue;
                }
                if let Ok(midi) = MidiOutput::new("omatainer-output") {
                    if let Ok(connection) = midi.connect(&port, &format!("omatainer-out-{name}")) {
                        self.outs.lock().push(connection);
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
