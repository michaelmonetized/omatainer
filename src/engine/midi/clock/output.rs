use super::*;
use crate::engine::{performance::WorkPermit, CommandPort};
use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender};
use midir::{MidiOutput, MidiOutputConnection, MidiOutputPort};
use std::{cmp::Ordering as Compare, collections::BinaryHeap};

#[derive(Clone, Debug)]
pub(crate) struct Status {
    pub requested: Arc<Config>,
    pub applied: Arc<Config>,
    pub pending: bool,
    pub generation: u64,
    pub outputs: Vec<Endpoint>,
    pub actual: Vec<Endpoint>,
    pub truncated: bool,
    pub error: Option<String>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            requested: Arc::new(Config::default()),
            applied: Arc::new(Config::default()),
            pending: false,
            generation: 0,
            outputs: Vec::new(),
            actual: Vec::new(),
            truncated: false,
            error: None,
        }
    }
}
struct Cancellation {
    flag: Arc<AtomicBool>,
    phase: AtomicU8,
}
struct Request {
    config: Arc<Config>,
    cancel: Arc<Cancellation>,
    permit: Option<WorkPermit>,
}
struct Management {
    status: ArcSwap<Status>,
    stop: AtomicBool,
    pending: parking_lot::Mutex<Option<Arc<Cancellation>>>,
}
pub(crate) struct Manager {
    shared: Arc<Shared>,
    management: Arc<Management>,
    requests: Sender<Request>,
    worker: Option<std::thread::JoinHandle<()>>,
    cmd: CommandPort,
}
impl Manager {
    /// Start the sole owner of explicitly selected clock ports.
    /// Takes the command port and saved policy; returns a background owner or a validation/thread refusal.
    pub(crate) fn start(cmd: CommandPort, config: Config) -> Result<Self, String> {
        Self::start_backend(cmd, config, Midir)
    }
    pub(super) fn start_backend<B: Backend>(
        cmd: CommandPort,
        config: Config,
        backend: B,
    ) -> Result<Self, String> {
        config.validate()?;
        let shared = cmd.clock_output().clone();
        let events = shared
            .receiver
            .lock()
            .take()
            .ok_or("MIDI clock output owner already started")?;
        let management = Arc::new(Management {
            status: ArcSwap::from_pointee(Status::default()),
            stop: AtomicBool::new(false),
            pending: parking_lot::Mutex::new(None),
        });
        let (requests, receiver) = bounded(1);
        let state = management.clone();
        let clocks = shared.clone();
        let commands = cmd.clone();
        let worker = std::thread::Builder::new()
            .name("omatainer-midi-clock".into())
            .spawn(move || {
                Worker {
                    backend,
                    shared: clocks,
                    management: state,
                    cmd: commands,
                    events,
                    active: Vec::new(),
                    pending: BinaryHeap::with_capacity(8192),
                    serial: 0,
                    safety: 0,
                    fault: 0,
                }
                .run(receiver)
            })
            .map_err(|e| e.to_string())?;
        let owner = Self {
            shared,
            management,
            requests,
            worker: Some(worker),
            cmd,
        };
        owner.queue(config, None)?;
        Ok(owner)
    }
    fn queue(&self, config: Config, permit: Option<WorkPermit>) -> Result<(), String> {
        let mut pending = self.management.pending.lock();
        if pending.is_some() {
            return Err("A clock output change is pending; wait or cancel it".into());
        }
        let cancel = Arc::new(Cancellation {
            flag: permit
                .as_ref()
                .map_or_else(|| Arc::new(AtomicBool::new(false)), WorkPermit::cancel),
            phase: AtomicU8::new(0),
        });
        let config = Arc::new(config);
        let mut status = (*self.management.status.load_full()).clone();
        status.requested = config.clone();
        status.pending = true;
        status.error = None;
        self.management.status.store(Arc::new(status));
        *pending = Some(cancel.clone());
        if self
            .requests
            .try_send(Request {
                config,
                cancel,
                permit,
            })
            .is_err()
        {
            *pending = None;
            let mut status = (*self.management.status.load_full()).clone();
            status.pending = false;
            status.error = Some("MIDI clock worker is unavailable".into());
            self.management.status.store(Arc::new(status));
            return Err("MIDI clock worker is unavailable".into());
        }
        Ok(())
    }
    /// Queue a reviewed clock policy on its output owner.
    /// Takes selected ports and compensation; returns immediately with admission or a Studio/protection refusal.
    pub(crate) fn configure(&self, config: Config) -> Result<(), String> {
        config.validate()?;
        if self.management.stop.load(Acquire)
            || self
                .worker
                .as_ref()
                .is_none_or(|worker| worker.is_finished())
        {
            return Err("MIDI clock owner stopped; restart to retry".into());
        }
        let permit = self
            .cmd
            .performance()
            .optional_work()
            .map_err(|e| e.to_string())?;
        self.queue(config, Some(permit))
    }
    /// Read the worker's requested and applied destinations.
    /// Takes this manager; returns an immutable status without waiting for backend discovery or sends.
    pub(crate) fn status(&self) -> Arc<Status> {
        self.management.status.load_full()
    }
    /// Cancel an uncommitted clock configuration.
    /// Takes this manager; returns whether a request was available to cancel.
    pub(crate) fn cancel(&self) -> bool {
        self.management
            .pending
            .lock()
            .as_ref()
            .is_some_and(|cancel| {
                if cancel.phase.compare_exchange(0, 2, AcqRel, Acquire).is_ok() {
                    cancel.flag.store(true, Release);
                    true
                } else {
                    false
                }
            })
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        self.shared.enabled.store(false, Release);
        self.management.stop.store(true, Release);
        self.cancel();
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}
pub(super) struct Port<P> {
    pub endpoint: Endpoint,
    pub port: P,
}
pub(super) trait Backend: Send + 'static {
    type Port;
    type Connection;
    fn discover(&mut self) -> Result<Vec<Port<Self::Port>>, String>;
    fn connect(&mut self, port: &Self::Port) -> Result<Self::Connection, String>;
    fn send(&mut self, connection: &mut Self::Connection, bytes: &[u8]) -> Result<(), String>;
}
struct Active<C> {
    endpoint: Endpoint,
    connection: Option<C>,
}
#[derive(Clone, Copy)]
struct Due {
    event: Event,
    serial: u64,
}
impl PartialEq for Due {
    fn eq(&self, other: &Self) -> bool {
        (self.event.deadline_ns, self.serial) == (other.event.deadline_ns, other.serial)
    }
}
impl Eq for Due {}
impl PartialOrd for Due {
    fn partial_cmp(&self, other: &Self) -> Option<Compare> {
        Some(self.cmp(other))
    }
}
impl Ord for Due {
    fn cmp(&self, other: &Self) -> Compare {
        (other.event.deadline_ns, other.serial).cmp(&(self.event.deadline_ns, self.serial))
    }
}
struct Worker<B: Backend> {
    backend: B,
    shared: Arc<Shared>,
    management: Arc<Management>,
    cmd: CommandPort,
    events: Receiver<Event>,
    active: Vec<Active<B::Connection>>,
    pending: BinaryHeap<Due>,
    serial: u64,
    safety: u64,
    fault: u8,
}
impl<B: Backend> Worker<B> {
    fn stop_ports(&mut self) -> Result<(), String> {
        self.shared.retired_outputs.store(Arc::new(
            self.active
                .iter()
                .map(|active| active.endpoint.clone())
                .collect(),
        ));
        self.shared
            .retired_until
            .store(self.shared.now_ns().saturating_add(1_000_000_000), Release);
        let mut error = None;
        for active in &mut self.active {
            if let Some(connection) = &mut active.connection {
                if let Err(failure) = self.backend.send(connection, &[0xfc]) {
                    error.get_or_insert(failure);
                }
            }
        }
        self.pending.clear();
        while self.events.try_recv().is_ok() {}
        self.shared.running.store(false, Release);
        error.map_or(Ok(()), Err)
    }
    fn cancelled(&self, request: &Request) -> bool {
        self.management.stop.load(Acquire)
            || request.cancel.flag.load(Acquire)
            || request.cancel.phase.load(Acquire) == 2
            || request.permit.as_ref().is_some_and(WorkPermit::cancelled)
    }
    fn prepare(
        &mut self,
        request: &Request,
    ) -> Result<(Vec<Active<B::Connection>>, Vec<Option<usize>>), String> {
        if self.cancelled(request) {
            return Err("Clock output change cancelled".into());
        }
        let ports = self.backend.discover()?;
        if self.cancelled(request) {
            return Err("Clock output change cancelled".into());
        }
        let mut status = (*self.management.status.load_full()).clone();
        status.outputs = ports
            .iter()
            .take(256)
            .map(|port| port.endpoint.clone())
            .collect();
        status.truncated = ports.len() > 256;
        self.management.status.store(Arc::new(status));
        let mut active = Vec::new();
        let mut reuse = Vec::new();
        if request.config.enabled {
            for wanted in &request.config.ports {
                let matches: Vec<_> = ports
                    .iter()
                    .filter(|port| {
                        wanted.matches(
                            &port.endpoint.name,
                            port.endpoint.id.as_deref().unwrap_or(""),
                        )
                    })
                    .collect();
                if matches.len() != 1 {
                    return Err(format!(
                        "Clock output {} is {}; choose an exact unique port ID",
                        wanted.name,
                        if matches.is_empty() {
                            "missing"
                        } else {
                            "ambiguous"
                        }
                    ));
                }
                let port = matches[0];
                if active
                    .iter()
                    .any(|active: &Active<B::Connection>| active.endpoint == port.endpoint)
                {
                    return Err(
                        "Clock selections resolve to the same output; choose distinct ports".into(),
                    );
                }
                if let Some(index) = self.active.iter().position(|active| {
                    active.endpoint == port.endpoint && active.connection.is_some()
                }) {
                    active.push(Active {
                        endpoint: port.endpoint.clone(),
                        connection: None,
                    });
                    reuse.push(Some(index));
                } else {
                    active.push(Active {
                        endpoint: port.endpoint.clone(),
                        connection: Some(self.backend.connect(&port.port)?),
                    });
                    reuse.push(None);
                }
                if self.cancelled(request) {
                    return Err("Clock output change cancelled".into());
                }
            }
        }
        Ok((active, reuse))
    }
    fn apply(&mut self, request: Request) {
        let running = self.shared.running.load(Acquire);
        let applied = self.management.status.load_full().applied.clone();
        let result = (|| {
            if running && request.config.enabled {
                return Err(
                    "Stop the song transport before changing clock outputs or compensation"
                        .to_owned(),
                );
            }
            self.shared.enabled.store(false, Release);
            if let Err(error) = self.stop_ports() {
                self.shared.fail(Fault::Send);
                return Err(format!("Clock stop was not confirmed: {error}"));
            }
            let (mut next, reuse) = self.prepare(&request)?;
            if self.cancelled(&request) {
                return Err("Clock output change cancelled".into());
            }
            let _claim = request
                .permit
                .as_ref()
                .map(|permit| permit.commit().map_err(|e| e.to_string()))
                .transpose()?;
            if request
                .cancel
                .phase
                .compare_exchange(0, 1, AcqRel, Acquire)
                .is_err()
            {
                return Err("Clock output change cancelled".into());
            }
            for (port, reuse) in next.iter_mut().zip(reuse) {
                if let Some(index) = reuse {
                    port.connection = self.active[index].connection.take();
                }
            }
            self.active = next;
            self.shared.actual_outputs.store(Arc::new(
                self.active
                    .iter()
                    .map(|active| active.endpoint.clone())
                    .collect(),
            ));
            self.shared
                .compensation_ms
                .store(request.config.compensation_ms, Release);
            self.shared.fault.store(0, Release);
            self.fault = 0;
            self.shared.generation.fetch_add(1, AcqRel);
            self.shared.enabled.store(request.config.enabled, Release);
            Ok(())
        })();
        let mut status = (*self.management.status.load_full()).clone();
        status.pending = false;
        match result {
            Ok(()) => {
                status.applied = request.config;
                status.generation = self.shared.generation.load(Acquire);
                status.actual = self
                    .active
                    .iter()
                    .map(|active| active.endpoint.clone())
                    .collect();
                status.error = None;
            }
            Err(error) => {
                status.error = Some(error);
                if !running
                    && !self.management.stop.load(Acquire)
                    && self.shared.fault.load(Acquire) == 0
                {
                    self.shared.generation.fetch_add(1, AcqRel);
                    self.shared.enabled.store(applied.enabled, Release);
                }
            }
        }
        self.management.status.store(Arc::new(status));
        self.management.pending.lock().take();
    }
    fn send_due(&mut self) {
        for _ in 0..64 {
            let now = self.shared.now_ns();
            let Some(due) = self
                .pending
                .peek()
                .copied()
                .filter(|due| due.event.deadline_ns <= now)
            else {
                break;
            };
            self.pending.pop();
            if due.event.generation != self.shared.generation.load(Acquire) {
                continue;
            }
            let (bytes, length) = due.event.message.bytes();
            for active in &mut self.active {
                if let Some(connection) = &mut active.connection {
                    if self.backend.send(connection, &bytes[..length]).is_err() {
                        self.shared.fail(Fault::Send);
                        return;
                    }
                }
            }
            if due.event.message == Message::Clock {
                self.shared.sent.fetch_add(1, Relaxed);
                let late = self.shared.now_ns().saturating_sub(due.event.deadline_ns);
                if late > 1_000_000 {
                    self.shared.late.fetch_add(1, Relaxed);
                }
                self.shared.max_late_ns.fetch_max(late, Relaxed);
            }
        }
    }
    fn run(&mut self, requests: Receiver<Request>) {
        self.safety = self.cmd.performance().input_epoch();
        while !self.management.stop.load(Acquire) {
            if let Ok(request) = requests.try_recv() {
                self.apply(request);
            }
            let safety = self.cmd.performance().input_epoch();
            if self.safety != safety {
                self.safety = safety;
                if self.shared.enabled.load(Acquire) || !self.active.is_empty() {
                    self.shared.fail(Fault::Safety);
                }
            }
            let end = self.shared.callback_until.load(Acquire);
            if self.shared.enabled.load(Acquire)
                && self.shared.running.load(Acquire)
                && end != 0
                && self.shared.now_ns() > end.saturating_add(500_000_000)
            {
                self.shared.fail(Fault::Callback);
            }
            let fault = self.shared.fault.load(Acquire);
            if fault != self.fault {
                self.fault = fault;
                let _ = self.stop_ports();
            }
            if self.shared.enabled.load(Acquire) {
                for _ in 0..64 {
                    let Ok(event) = self.events.try_recv() else {
                        break;
                    };
                    if event.generation != self.shared.generation.load(Acquire) {
                        continue;
                    }
                    if self.pending.len() == 8192 {
                        self.shared.fail(Fault::QueueFull);
                        break;
                    }
                    self.serial = self.serial.wrapping_add(1);
                    self.pending.push(Due {
                        event,
                        serial: self.serial,
                    });
                }
                self.send_due();
            }
            std::thread::park_timeout(if self.pending.is_empty() {
                Duration::from_millis(1)
            } else {
                Duration::from_micros(100)
            });
        }
        self.shared.enabled.store(false, Release);
        let _ = self.stop_ports();
        self.active.clear();
        self.shared.actual_outputs.store(Arc::new(Vec::new()));
        self.management.pending.lock().take();
    }
}
impl<B: Backend> Drop for Worker<B> {
    fn drop(&mut self) {
        self.shared.enabled.store(false, Release);
        self.shared.running.store(false, Release);
        let mut status = (*self.management.status.load_full()).clone();
        status.pending = false;
        if !self.management.stop.load(Acquire) {
            status.error = Some("MIDI clock owner stopped unexpectedly; restart to retry".into());
        }
        self.management.status.store(Arc::new(status));
        self.management.pending.lock().take();
    }
}
struct Midir;
impl Backend for Midir {
    type Port = MidiOutputPort;
    type Connection = MidiOutputConnection;
    fn discover(&mut self) -> Result<Vec<Port<Self::Port>>, String> {
        let output = MidiOutput::new("omatainer-clock-discovery").map_err(|e| e.to_string())?;
        output
            .ports()
            .iter()
            .map(|port| {
                Ok(Port {
                    endpoint: Endpoint {
                        name: output.port_name(port).map_err(|e| e.to_string())?,
                        id: Some(port.id()),
                    },
                    port: port.clone(),
                })
            })
            .collect()
    }
    fn connect(&mut self, port: &Self::Port) -> Result<Self::Connection, String> {
        MidiOutput::new("omatainer-clock-output")
            .map_err(|e| e.to_string())?
            .connect(port, "omatainer-clock-out")
            .map_err(|e| e.to_string())
    }
    fn send(&mut self, connection: &mut Self::Connection, bytes: &[u8]) -> Result<(), String> {
        connection.send(bytes).map_err(|e| e.to_string())
    }
}
