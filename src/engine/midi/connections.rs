//! One management worker owns OS discovery, connection attempts and teardown.
//! Only the existing fixed-work InputSink runs in a raw MIDI callback.
use super::policy::{self, Control, InputPolicy, PolicyError, PolicyStatus, Request};
use super::{device_status::Status, handoff, next_source_id, pick_map, MidiMap};
use crate::engine::{CommandPort, Snapshot};
use crossbeam_channel::{bounded, Sender};
use midir::{Ignore, MidiInput, MidiInputConnection};
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
    Performance(crate::engine::performance::Error),
}

struct Activity {
    busy: AtomicBool,
    alive: AtomicBool,
    retry_pending: AtomicBool,
    policy: Control,
}
struct WorkerLife(Arc<Activity>);
impl Drop for WorkerLife {
    fn drop(&mut self) {
        self.0.alive.store(false, Release);
        self.0.busy.store(false, Release);
        self.0.policy.unavailable();
    }
}

pub(super) struct Manager {
    requests: Option<Sender<()>>,
    activity: Arc<Activity>,
    worker: Option<JoinHandle<()>>,
}

impl Manager {
    pub(super) fn start_with_policy<B: Backend>(
        backend: B,
        snapshot: &Arc<Mutex<Snapshot>>,
        cmd: CommandPort,
        maps: Vec<MidiMap>,
        log: Arc<Mutex<Vec<String>>>,
        counters: Arc<handoff::InputCounters>,
        policy: InputPolicy,
    ) -> std::io::Result<Self> {
        policy
            .validate()
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        // Keyboard and mouse remain usable with or without connected hardware.
        Status::new(snapshot, "keyboard + mouse", "built-in").connected();
        let activity = Arc::new(Activity {
            busy: AtomicBool::new(true),
            alive: AtomicBool::new(true),
            retry_pending: AtomicBool::new(false),
            policy: Control::with_performance(policy, cmd.performance().clone()),
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
                    counters,
                    entries: Vec::new(),
                    backend_status: None,
                    activity: shared.clone(),
                };
                loop {
                    // There is at most one coalesced wake, never a policy-job
                    // backlog. A request changed during OS work is read afresh.
                    let _ = receiver.try_recv();
                    if shared.policy.stopped() {
                        break;
                    }
                    shared.busy.store(true, Release);
                    let request = shared.policy.requested();
                    if !shared.retry_pending.swap(false, AcqRel)
                        && !shared.policy.status().pending()
                    {
                        shared.busy.store(false, Release);
                        match receiver.recv_timeout(std::time::Duration::from_millis(1500)) {
                            Ok(()) => {},
                            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                                if let Ok(_permit) = worker.cmd.performance().project_change() {
                                    shared.busy.store(true, Release);
                                    let request = shared.policy.requested();
                                    worker.refresh(&request);
                                    shared.busy.store(false, Release);
                                }
                            }
                        }
                        continue;
                    }
                    if !worker.refresh(&request) {
                        continue;
                    }
                    drop(request);
                    shared.busy.store(false, Release);
                }
                // Connections close, then input worker guards join, on this worker.
            })?;
        Ok(Self {
            requests: Some(requests),
            activity,
            worker: Some(worker),
        })
    }
    pub(super) fn configure(&self, policy: InputPolicy) -> Result<u64, PolicyError> {
        if !self.available() {
            return Err(PolicyError::Unavailable);
        }
        let generation = self.activity.policy.request(policy)?;
        match self
            .requests
            .as_ref()
            .ok_or(PolicyError::Unavailable)?
            .try_send(())
        {
            Ok(()) | Err(crossbeam_channel::TrySendError::Full(())) => Ok(generation),
            Err(crossbeam_channel::TrySendError::Disconnected(())) => {
                self.activity.policy.unavailable();
                Err(PolicyError::Unavailable)
            }
        }
    }
    pub(super) fn policy_status(&self) -> Arc<PolicyStatus> {
        self.activity.policy.status()
    }
    pub(super) fn policy_reader(&self) -> impl Fn() -> Arc<PolicyStatus> + Send + 'static {
        let activity = self.activity.clone();
        move || activity.policy.status()
    }
    pub(super) fn retry(&self) -> Retry {
        if !self.available() {
            return Retry::Unavailable;
        }
        if self.policy_status().pending() {
            return Retry::AlreadyRunning;
        }
        if self
            .activity
            .busy
            .compare_exchange(false, true, AcqRel, Acquire)
            .is_err()
        {
            return Retry::AlreadyRunning;
        }
        // Retry is a deliberate device-topology operation too. Reissue the
        // current policy so it carries the same generation-qualified exclusive
        // permit through queued discovery, blocked OS calls and completion.
        let policy = (*self.activity.policy.status().requested_policy).clone();
        match self.configure(policy) {
            Ok(_) => Retry::Queued,
            Err(error) => {
                self.activity.busy.store(false, Release);
                if let PolicyError::Performance(error) = error { Retry::Performance(error) } else { Retry::Unavailable }
            }
        }
    }

    pub(super) fn busy(&self) -> bool {
        self.activity.busy.load(Acquire) || self.policy_status().pending()
    }
    pub(super) fn available(&self) -> bool {
        self.activity.alive.load(Acquire)
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        self.activity.policy.stop();
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
    counters: Arc<handoff::InputCounters>,
    entries: Vec<Entry<B::Port, B::Connection>>,
    backend_status: Option<Status>,
    activity: Arc<Activity>,
}
impl<B: Backend> Worker<B> {
    fn refresh(&mut self, request: &Request) -> bool {
        // Apply exclusions before any new discovery/connect OS call. Allowed
        // sources keep their connection and source ID across preference edits.
        for entry in &mut self.entries {
            if !request.policy.allows(&entry.name) {
                entry.active.take();
                entry.status.disabled();
            }
        }
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
                    status.failed(&error);
                }
                return self
                    .activity
                    .policy
                    .complete(request, None, Vec::new(), Some(error));
            }
        };
        if self.activity.policy.stopped() {
            return true;
        }
        if !self.activity.policy.is_current(request) {
            return false;
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
        let mut error = None;
        for entry in &mut self.entries {
            if self.activity.policy.stopped() {
                return true;
            }
            if !self.activity.policy.is_current(request) {
                return false;
            }
            if !entry.present {
                entry.active.take();
                entry.status.disconnected();
                continue;
            }
            if !request.policy.allows(&entry.name) {
                entry.active.take();
                entry.status.disabled();
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
            let pair = handoff::start_on_port(
                next_source_id(),
                entry.map.clone(),
                self.cmd.clone(),
                self.log.clone(),
                entry.name.clone(),
                entry.id.clone(),
                self.counters.clone(),
                false,
                move || completed.disconnected(),
            );
            let (input, worker) = match pair {
                Ok(pair) => pair,
                Err(failure) => {
                    entry.status.failed(&failure);
                    error = Some(failure.to_string());
                    continue;
                }
            };
            match self.backend.connect(&entry.port, &entry.name, input) {
                Ok(connection) => {
                    if !self
                        .activity
                        .policy
                        .activate(&entry.name, || worker.enable())
                    {
                        drop(connection);
                        drop(worker);
                        if self.activity.policy.stopped() {
                            entry.status.disconnected();
                        } else {
                            entry.status.disabled();
                        }
                        continue;
                    }
                    entry.status.connected();
                    entry.active = Some(Active {
                        _connection: connection,
                        _worker: worker,
                    });
                }
                Err(failure) => {
                    entry.status.failed(&failure);
                    error = Some(failure);
                    drop(worker); // failure survives completion; retry waits for it
                }
            }
        }
        let mut available = Vec::new();
        let mut truncated = false;
        for entry in self.entries.iter().filter(|entry| entry.present) {
            if available.contains(&entry.name) {
                continue;
            }
            if available.len() >= policy::MAX_AVAILABLE_INPUTS
                || entry.name.len() > policy::MAX_INPUT_NAME_BYTES
                || entry.name.is_empty()
                || entry.name.contains('\0')
            {
                truncated = true;
            } else {
                available.push(entry.name.clone());
            }
        }
        available.sort();
        let missing = match request.policy.as_ref() {
            InputPolicy::Selected(names) => names
                .iter()
                .filter(|name| {
                    !self
                        .entries
                        .iter()
                        .any(|entry| entry.present && &entry.name == *name)
                })
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        self.activity
            .policy
            .complete(request, Some((available, truncated)), missing, error)
    }
}

pub(super) struct MidirBackend;
pub(super) fn application_port(name: &str) -> bool { name.to_ascii_lowercase().starts_with("omatainer") }
impl Backend for MidirBackend {
    type Port = midir::MidiInputPort;
    type Connection = MidiInputConnection<()>;
    fn discover(&mut self) -> Result<Vec<Port<Self::Port>>, String> {
        let probe = MidiInput::new("omatainer-discover").map_err(|error| error.to_string())?;
        let ports = probe
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
            .collect::<Result<Vec<_>, String>>()?;
        Ok(ports.into_iter().filter(|port| !application_port(&port.name)).collect())
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

}

#[cfg(test)]
mod policy_tests;
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
