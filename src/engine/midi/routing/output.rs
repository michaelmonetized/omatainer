//! One output owner performs discovery, connection, send, reset and teardown.
//! GUI requests and renderer delivery never call a MIDI backend.
use super::{control::*, Endpoint, Routing};
use crate::engine::{performance::WorkPermit, CommandPort, session::MAX_TRACKS as TRACKS};
use arc_swap::ArcSwap;
use crossbeam_channel::{bounded, Receiver, Sender};
use midir::{MidiInput, MidiOutput, MidiOutputConnection, MidiOutputPort};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering::*};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Status {
    pub requested: Arc<Routing>,
    pub applied: Arc<Routing>,
    pub requested_generation: u64,
    pub applied_generation: u64,
    pub pending: bool,
    pub error: Option<String>,
    pub inputs: Vec<Endpoint>,
    pub outputs: Vec<Endpoint>,
    pub truncated: bool,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            requested: Arc::new(Routing::default()),
            applied: Arc::new(Routing::default()),
            requested_generation: 0,
            applied_generation: 0,
            pending: false,
            error: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            truncated: false,
        }
    }
}
struct Cancellation {
    flag: Arc<AtomicBool>,
    phase: AtomicU8,
}
struct Request {
    generation: u64,
    config: Arc<Routing>,
    permit: Option<WorkPermit>,
    cancel: Arc<Cancellation>,
}
struct Management {
    status: ArcSwap<Status>,
    stop: AtomicBool,
    next: AtomicU64,
    pending: parking_lot::Mutex<Option<Arc<Cancellation>>>,
}
pub(crate) struct Manager {
    management: Arc<Management>,
    requests: Sender<Request>,
    shared: Arc<Shared>,
    worker: Option<std::thread::JoinHandle<()>>,
    cmd: CommandPort,
}
impl Manager {
    pub(crate) fn start(cmd: CommandPort, config: Routing) -> Result<Self, String> {
        Self::start_backend(cmd, config, Midir)
    }
    pub(super) fn start_backend<B: Backend>(
        cmd: CommandPort,
        config: Routing,
        backend: B,
    ) -> Result<Self, String> {
        config.validate()?;
        let shared = cmd.midi_routing().clone();
        let events = shared
            .receiver
            .lock()
            .take()
            .ok_or("MIDI output owner already started")?;
        let management = Arc::new(Management {
            status: ArcSwap::from_pointee(Status::default()),
            stop: AtomicBool::new(false),
            next: AtomicU64::new(2),
            pending: parking_lot::Mutex::new(None),
        });
        let (requests, receiver) = bounded(1);
        let managed = management.clone();
        let owner = shared.clone();
        let commands = cmd.clone();
        // Initial startup policy is installed before input connections can
        // become enabled, including a profile that starts protected.
        shared
            .generation
            .store(if config.enabled { 0 } else { 1 }, Release);
        shared.explicit.store(config.enabled, Release);
        let worker = std::thread::Builder::new()
            .name("omatainer-midi-output".into())
            .spawn(move || {
                let mut worker = Worker {
                    backend,
                    shared: owner,
                    management: managed,
                    cmd: commands,
                    events,
                    active: Vec::new(),
                    route_indices: [None; TRACKS],
                    generation: 0,
                    epoch: 1,
                    safety: 0,
                    gates: HashMap::with_capacity(crate::engine::project::MAX_TOTAL_NOTES + 256),
                    pedals: HashMap::with_capacity(MAX_PEDALS),
                    retire: Vec::with_capacity(crate::engine::project::MAX_TOTAL_NOTES + 256),
                };
                worker.run(receiver);
            })
            .map_err(|e| e.to_string())?;
        let manager = Self {
            management,
            requests,
            shared,
            worker: Some(worker),
            cmd,
        };
        manager.queue(1, config, None)?;
        Ok(manager)
    }
    fn queue(
        &self,
        generation: u64,
        config: Routing,
        permit: Option<WorkPermit>,
    ) -> Result<u64, String> {
        let mut pending = self.management.pending.lock();
        if pending.is_some() {
            return Err(
                "MIDI routing change is still pending; cancel or wait for its receipt".into(),
            );
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
        status.requested_generation = generation;
        status.pending = true;
        status.error = None;
        self.management.status.store(Arc::new(status));
        *pending = Some(cancel.clone());
        if self
            .requests
            .try_send(Request {
                generation,
                config,
                permit,
                cancel,
            })
            .is_err()
        {
            *pending = None;
            let mut status = (*self.management.status.load_full()).clone();
            status.pending = false;
            status.error = Some("MIDI routing worker is unavailable".into());
            self.management.status.store(Arc::new(status));
            return Err("MIDI routing worker is unavailable".into());
        }
        Ok(generation)
    }
    pub(crate) fn configure(&self, config: Routing) -> Result<u64, String> {
        config.validate()?;
        if self.management.stop.load(Acquire)
            || self.worker.as_ref().is_none_or(|w| w.is_finished())
        {
            return Err("MIDI output owner is unavailable; restart to retry".into());
        }
        let permit = self
            .cmd
            .performance()
            .optional_work()
            .map_err(|e| e.to_string())?;
        let generation = self
            .management
            .next
            .fetch_update(Relaxed, Relaxed, |generation| generation.checked_add(1))
            .map_err(|_| "MIDI routing generation space exhausted")?;
        self.queue(generation, config, Some(permit))
    }
    pub(crate) fn status(&self) -> Arc<Status> {
        self.management.status.load_full()
    }
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
        self.management.stop.store(true, Release);
        self.shared.mask.store(0, Release);
        self.shared.generation.store(0, Release);
        self.cancel();
        self.shared.reset_outputs();
        if let Some(worker) = self.worker.take() {
            if worker.is_finished() {
                let _ = worker.join();
            }
            // Blocked backend work retains ownership on its worker. GUI close
            // cannot force an OS send/open call to return or join it here.
        }
    }
}
pub(super) struct Port<P> {
    pub endpoint: Endpoint,
    pub port: P,
}
pub(super) struct Inventory<P> {
    pub inputs: Vec<Endpoint>,
    pub outputs: Vec<Port<P>>,
}
pub(super) trait Backend: Send + 'static {
    type Port;
    type Connection;
    fn discover(&mut self) -> Result<Inventory<Self::Port>, String>;
    fn connect(&mut self, port: &Self::Port) -> Result<Self::Connection, String>;
    fn send(&mut self, connection: &mut Self::Connection, bytes: &[u8]) -> Result<(), String>;
}
struct Active<C> {
    endpoint: Endpoint,
    connection: Option<C>,
    counts: Box<[[u32; 128]; 16]>,
    clip_channels: [u16; TRACKS],
}
impl<C> Active<C> {
    fn new(endpoint: Endpoint, connection: Option<C>) -> Self {
        Self {
            endpoint,
            connection,
            counts: Box::new([[0; 128]; 16]),
            clip_channels: [0; TRACKS],
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GateKey {
    track: u8,
    owner: Owner,
    port: usize,
    channel: u8,
    note: u8,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PedalKey {
    track: u8,
    owner: Owner,
    port: usize,
    channel: u8,
}
enum Candidate<C> {
    New(Active<C>),
    Reuse(usize),
}
struct Prepared<C> {
    candidate: Vec<Candidate<C>>,
    indices: [Option<usize>; TRACKS],
    inputs: Vec<Endpoint>,
    outputs: Vec<Endpoint>,
    truncated: bool,
}
struct Worker<B: Backend> {
    backend: B,
    shared: Arc<Shared>,
    management: Arc<Management>,
    cmd: CommandPort,
    events: Receiver<OutputEvent>,
    active: Vec<Active<B::Connection>>,
    route_indices: [Option<usize>; TRACKS],
    generation: u64,
    epoch: u64,
    safety: u64,
    gates: HashMap<GateKey, u32>,
    pedals: HashMap<PedalKey, ()>,
    retire: Vec<GateKey>,
}
impl<B: Backend> Worker<B> {
    fn cancelled(&self, r: &Request) -> bool {
        self.management.stop.load(Acquire)
            || r.cancel.flag.load(Acquire)
            || r.cancel.phase.load(Acquire) == 2
            || r.permit.as_ref().is_some_and(WorkPermit::cancelled)
    }
    fn prepare(&mut self, r: &Request) -> Result<Prepared<B::Connection>, String> {
        if self.cancelled(r) {
            return Err("MIDI routing cancelled before apply".into());
        }
        let inventory = self.backend.discover()?;
        if self.cancelled(r) {
            return Err("MIDI routing cancelled before apply".into());
        }
        let mut prepared = Prepared {
            candidate: Vec::new(),
            indices: [None; TRACKS],
            inputs: inventory.inputs.iter().take(256).cloned().collect(),
            outputs: inventory
                .outputs
                .iter()
                .take(256)
                .map(|p| p.endpoint.clone())
                .collect(),
            truncated: inventory.inputs.len() > 256 || inventory.outputs.len() > 256,
        };
        let mut status = (*self.management.status.load_full()).clone();
        status.inputs = prepared.inputs.clone();
        status.outputs = prepared.outputs.clone();
        status.truncated = prepared.truncated;
        self.management.status.store(Arc::new(status));
        // An explicit rescan retires vanished connections even when a proposed
        // replacement cannot be prepared. It never substitutes another port.
        for index in 0..self.active.len() {
            if self.active[index].connection.is_some()
                && !inventory
                    .outputs
                    .iter()
                    .any(|port| port.endpoint == self.active[index].endpoint)
            {
                let reset_error = self.reset_port(index).err();
                self.active[index].connection.take();
                self.shared
                    .live
                    .lock()
                    .actual_outputs
                    .retain(|endpoint| endpoint != &self.active[index].endpoint);
                for track in 0..TRACKS {
                    if self.route_indices[track] == Some(index) {
                        self.route_indices[track] = None;
                        self.shared.mask.fetch_and(!(1 << track), AcqRel);
                    }
                }
                if let Some(error) = reset_error {
                    let mut status = (*self.management.status.load_full()).clone();
                    status.error = Some(format!(
                        "Disconnected output reset was not confirmed: {error}"
                    ));
                    self.management.status.store(Arc::new(status));
                }
            }
        }
        if !r.config.enabled {
            return Ok(prepared);
        }
        for route in &r.config.routes {
            let Some(wanted) = &route.output else {
                continue;
            };
            let matches: Vec<_> = inventory
                .outputs
                .iter()
                .filter(|p| {
                    wanted.matches(&p.endpoint.name, p.endpoint.id.as_deref().unwrap_or(""))
                })
                .collect();
            if matches.len() != 1 {
                return Err(format!(
                    "Output {} is {}; choose an exact unique port id",
                    wanted.name,
                    if matches.is_empty() {
                        "missing"
                    } else {
                        "ambiguous"
                    }
                ));
            }
            let actual = &matches[0].endpoint;
            if self
                .shared
                .live
                .lock()
                .sources
                .iter()
                .any(|source| source.endpoint.conflicts(actual))
            {
                return Err("Feedback guard: deselect the output device from MIDI inputs before applying its output route".into());
            }
            // Resolve ids as well as preference labels before any connection.
            if r.config.routes.iter().flat_map(|r| &r.inputs).any(|i| {
                inventory.inputs.iter().any(|p| {
                    i.port.matches(&p.name, p.id.as_deref().unwrap_or("")) && p.conflicts(actual)
                })
            }) {
                return Err(format!(
                    "Feedback guard refuses output {} on an active input device",
                    actual.name
                ));
            }
            let index = prepared.candidate.iter().position(|c| match c {
                Candidate::New(a) => &a.endpoint == actual,
                Candidate::Reuse(i) => &self.active[*i].endpoint == actual,
            });
            let index = if let Some(index) = index {
                index
            } else {
                let candidate = if let Some(i) = self
                    .active
                    .iter()
                    .position(|a| a.endpoint == *actual && a.connection.is_some())
                {
                    Candidate::Reuse(i)
                } else {
                    Candidate::New(Active::new(
                        actual.clone(),
                        Some(self.backend.connect(&matches[0].port)?),
                    ))
                };
                prepared.candidate.push(candidate);
                prepared.candidate.len() - 1
            };
            prepared.indices[usize::from(route.track)] = Some(index);
            if self.cancelled(r) {
                return Err("MIDI routing cancelled before apply".into());
            }
        }
        Ok(prepared)
    }
    fn reset_port(&mut self, index: usize) -> Result<(), String> {
        let active = &mut self.active[index];
        let mut first = None;
        if let Some(connection) = &mut active.connection {
            for ch in 0..16 {
                for bytes in [
                    &[0xb0 | ch, 120, 0][..],
                    &[0xb0 | ch, 123, 0][..],
                    &[0xb0 | ch, 121, 0][..],
                    &[0xe0 | ch, 0, 64][..],
                    &[0xd0 | ch, 0][..],
                ] {
                    if let Err(error) = self.backend.send(connection, bytes) {
                        first.get_or_insert(error);
                    }
                }
            }
        }
        active.counts.fill([0; 128]);
        active.clip_channels.fill(0);
        self.gates.retain(|key, _| key.port != index);
        self.pedals.retain(|key, _| key.port != index);
        first.map_or(Ok(()), Err)
    }
    fn reset(&mut self) -> Result<(), String> {
        let mut first = None;
        for index in 0..self.active.len() {
            if let Err(error) = self.reset_port(index) {
                first.get_or_insert(error);
            }
        }
        first.map_or(Ok(()), Err)
    }
    fn apply(&mut self, r: Request) {
        let prepared = self.prepare(&r);
        let result: Result<(), String> = (|| {
            let prepared = prepared?;
            if self.cancelled(&r) {
                return Err("MIDI routing cancelled before apply".into());
            }
            let _claim = r
                .permit
                .as_ref()
                .map(|p| p.commit().map_err(|e| e.to_string()))
                .transpose()?;
            let shared = self.shared.clone();
            let mut live = shared.live.lock();
            if prepared.candidate.iter().any(|candidate| {
                let endpoint = match candidate {
                    Candidate::New(a) => &a.endpoint,
                    Candidate::Reuse(i) => &self.active[*i].endpoint,
                };
                live.sources
                    .iter()
                    .any(|source| source.endpoint.conflicts(endpoint))
            }) {
                return Err("Feedback guard: an input activated on the prepared output device; previous routing was retained".into());
            }
            if r.cancel
                .phase
                .compare_exchange(0, 1, AcqRel, Acquire)
                .is_err()
            {
                return Err("MIDI routing cancelled before apply".into());
            }
            shared.generation.store(0, Release);
            shared.mask.store(0, Release);
            for source in &live.sources {
                shared.release(source.sources, &self.cmd);
            }
            let reset_error = self.reset().err();
            let mut next = Vec::with_capacity(prepared.candidate.len());
            for candidate in prepared.candidate {
                match candidate {
                    Candidate::New(a) => next.push(a),
                    Candidate::Reuse(i) => next.push(Active::new(
                        self.active[i].endpoint.clone(),
                        self.active[i].connection.take(),
                    )),
                }
            }
            self.active = next;
            live.actual_outputs = self.active.iter().map(|a| a.endpoint.clone()).collect();
            self.route_indices = prepared.indices;
            self.generation = r.generation;
            self.epoch = shared.output_epoch.fetch_add(1, AcqRel) + 1;
            live.config = r.config.clone();
            shared.bind_identity(&r.config);
            shared.mask.store(r.config.output_mask(), Release);
            shared.explicit.store(r.config.enabled, Release);
            shared.generation.store(r.generation, Release);
            let mut status = (*self.management.status.load_full()).clone();
            status.applied = r.config.clone();
            status.applied_generation = r.generation;
            status.inputs = prepared.inputs;
            status.outputs = prepared.outputs;
            status.truncated = prepared.truncated;
            status.error = reset_error
                .map(|e| format!("Routing applied; previous output reset was not confirmed: {e}"));
            self.management.status.store(Arc::new(status));
            Ok(())
        })();
        let mut pending = self.management.pending.lock();
        let mut status = (*self.management.status.load_full()).clone();
        status.pending = false;
        if let Err(error) = result {
            status.error = Some(error.chars().take(2048).collect());
        }
        self.management.status.store(Arc::new(status));
        *pending = None;
    }
    fn event(&mut self, event: OutputEvent) {
        if self.management.stop.load(Acquire)
            || event.generation != self.generation
            || event.epoch != self.epoch
            || event.generation != self.shared.generation.load(Acquire)
            || event.epoch != self.shared.output_epoch.load(Acquire)
        {
            return;
        }
        if self.cmd.performance().status().recovery {
            return;
        }
        let config = self.shared.config();
        let Some(route) = config.routes.iter().find(|r| r.track == event.track) else {
            return;
        };
        if let Some(clear) = event.clear {
            if let Some(index) = self.route_indices[usize::from(event.track)] {
                self.clear_owned(event.track, index, clear);
            }
            return;
        }
        if !self.shared.target_current(event.track) {
            self.shared.counts[usize::from(event.track)].filtered.fetch_add(1, Relaxed); return;
        }
        if !route.filter.accepts(event.packet.bytes()) {
            self.shared.counts[usize::from(event.track)]
                .filtered
                .fetch_add(1, Relaxed);
            return;
        }
        let Some(index) = self.route_indices[usize::from(event.track)] else {
            return;
        };
        if self.active[index].connection.is_none() {
            self.shared.counts[usize::from(event.track)]
                .failed
                .fetch_add(1, Relaxed);
            return;
        }
        let packet = event.packet.with_channel(route.output_channel);
        if let Some(channel) = packet.channel() {
            if matches!(event.owner, Owner::Clip { .. } | Owner::ClipLane(_)) {
                self.active[index].clip_channels[usize::from(event.track)] |= 1 << channel;
            }
        }
        if packet.bytes()[0] & 0xf0 == 0xb0 && packet.bytes()[1] == 64 {
            let channel = packet.channel().unwrap();
            let key = PedalKey {
                track: event.track,
                owner: event.owner,
                port: index,
                channel,
            };
            if packet.bytes()[2] >= 64 {
                if self.pedals.len() >= MAX_PEDALS && !self.pedals.contains_key(&key) {
                    self.shared.overrun(usize::from(event.track));
                    return;
                }
                self.pedals.insert(key, ());
            } else {
                self.pedals.remove(&key);
                if self
                    .pedals
                    .keys()
                    .any(|key| key.port == index && key.channel == channel)
                {
                    self.shared.counts[usize::from(event.track)]
                        .filtered
                        .fetch_add(1, Relaxed);
                    return;
                }
            }
        }
        if let Some((note, _, on)) = packet.note() {
            let channel = packet.channel().unwrap();
            let key = GateKey {
                track: event.track,
                owner: event.owner,
                port: index,
                channel,
                note,
            };
            let counts = &mut self.active[index].counts[usize::from(channel)][usize::from(note)];
            if on {
                if self.gates.len() >= crate::engine::project::MAX_TOTAL_NOTES + 256
                    && !self.gates.contains_key(&key)
                {
                    self.shared.reset_outputs();
                    self.shared.counts[usize::from(event.track)]
                        .failed
                        .fetch_add(1, Relaxed);
                    return;
                }
                let owned = self.gates.entry(key).or_default();
                // One physical live key replaces its prior onset. A clip note
                // may span several cycles and retains its own bounded count.
                if matches!(event.owner, Owner::Live { .. }) && *owned > 0 {
                    if *counts == 1 {
                        self.wire(
                            event.track,
                            index,
                            super::packet::Packet::new(&[0x80 | channel, note, 64]).unwrap(),
                        );
                        self.wire(event.track, index, packet);
                        return;
                    }
                    self.shared.counts[usize::from(event.track)]
                        .filtered
                        .fetch_add(1, Relaxed);
                    return;
                }
                let Some(next_owned) = owned.checked_add(event.weight) else {
                    self.shared.overrun(usize::from(event.track));
                    return;
                };
                let Some(next_total) = counts.checked_add(event.weight) else {
                    self.shared.overrun(usize::from(event.track));
                    return;
                };
                let old_total = *counts;
                *owned = next_owned;
                *counts = next_total;
                if old_total > 0 {
                    self.shared.counts[usize::from(event.track)]
                        .filtered
                        .fetch_add(1, Relaxed);
                    return;
                }
            } else {
                if let Some(owned) = self.gates.get_mut(&key) {
                    *owned -= 1;
                    *counts = counts.saturating_sub(1);
                    if *owned == 0 {
                        self.gates.remove(&key);
                    }
                } else if *counts > 0 {
                    return;
                }
                if *counts > 0 {
                    self.shared.counts[usize::from(event.track)]
                        .filtered
                        .fetch_add(1, Relaxed);
                    return;
                }
            }
        }
        self.wire(event.track, index, packet);
    }
    fn wire(&mut self, track: u8, index: usize, packet: super::packet::Packet) {
        let Some(connection) = &mut self.active[index].connection else {
            return;
        };

        match self.backend.send(connection, packet.bytes()) {
            Ok(()) => {
                self.shared.counts[usize::from(track)]
                    .sent
                    .fetch_add(1, Relaxed);
            }
            Err(error) => {
                self.shared.counts[usize::from(track)]
                    .failed
                    .fetch_add(1, Relaxed);
                let mut status = (*self.management.status.load_full()).clone();
                status.error = Some(format!(
                    "MIDI output {} send failed: {}",
                    self.active[index].endpoint.name,
                    error.chars().take(1024).collect::<String>()
                ));
                self.management.status.store(Arc::new(status));
                self.shared.reset_outputs();
                self.active[index].connection.take();
            }
        }
    }
    fn clear_owned(&mut self, track: u8, index: usize, clear: Clear) {
        let matches = |owner: Owner, owner_track: u8| match clear {
            Clear::Track => owner_track == track,
            Clear::Clip => {
                matches!(owner,Owner::Clip {track:owner,..}|Owner::ClipLane(owner) if owner==track)
            }
            Clear::Source(source) => matches!(owner,Owner::Live {source:owner,..} if owner==source),
        };
        self.retire.clear();
        self.retire.extend(
            self.gates
                .keys()
                .filter(|key| key.port == index && matches(key.owner, key.track))
                .copied(),
        );
        let mut released = [[false; 128]; 16];
        for key in self.retire.drain(..) {
            if let Some(count) = self.gates.remove(&key) {
                let total =
                    &mut self.active[index].counts[usize::from(key.channel)][usize::from(key.note)];
                *total = total.saturating_sub(count);
                released[usize::from(key.channel)][usize::from(key.note)] |= *total == 0;
            }
        }
        let mut pedals = [false; 16];
        self.pedals.retain(|key, _| {
            let remove = key.port == index && matches(key.owner, key.track);
            if remove {
                pedals[usize::from(key.channel)] = true;
            }
            !remove
        });
        for (ch, notes) in released.iter().enumerate() {
            for (note, &off) in notes.iter().enumerate() {
                if off {
                    self.wire(
                        track,
                        index,
                        super::packet::Packet::new(&[0x80 | ch as u8, note as u8, 64]).unwrap(),
                    );
                }
            }
            if pedals[ch]
                && !self
                    .pedals
                    .keys()
                    .any(|key| key.port == index && usize::from(key.channel) == ch)
            {
                self.wire(
                    track,
                    index,
                    super::packet::Packet::new(&[0xb0 | ch as u8, 64, 0]).unwrap(),
                );
            }
        }
        if matches!(clear, Clear::Clip | Clear::Track) {
            self.active[index].clip_channels[usize::from(track)] = 0;
        }
    }
    fn run(&mut self, requests: Receiver<Request>) {
        self.shared.alive.store(true, Release);
        while !self.management.stop.load(Acquire) {
            if let Ok(request) = requests.try_recv() {
                self.apply(request);
            }
            let safety = self.cmd.performance().input_epoch();
            if self.safety != safety {
                self.safety = safety;
                self.shared.reset_outputs();
            }
            let epoch = self.shared.output_epoch.load(Acquire);
            if epoch != self.epoch {
                if let Err(error) = self.reset() {
                    let mut status = (*self.management.status.load_full()).clone();
                    status.error = Some(format!("MIDI reset not confirmed: {error}"));
                    self.management.status.store(Arc::new(status));
                }
                self.epoch = epoch;
            }
            for _ in 0..64 {
                let Ok(event) = self.events.try_recv() else {
                    break;
                };
                self.event(event);
            }
            std::thread::park_timeout(Duration::from_millis(1));
        }
        self.shared.generation.store(0, Release);
        self.shared.mask.store(0, Release);
        let _ = self.reset();
        self.active.clear();
        self.shared.alive.store(false, Release);
        self.management.pending.lock().take();
    }
}
impl<B: Backend> Drop for Worker<B> {
    fn drop(&mut self) {
        self.shared.generation.store(0, Release);
        self.shared.mask.store(0, Release);
        self.shared.alive.store(false, Release);
        self.shared.live.lock().actual_outputs.clear();
        self.management.pending.lock().take();
        let mut status = (*self.management.status.load_full()).clone();
        status.pending = false;
        if !self.management.stop.load(Acquire) {
            status.error = Some("MIDI output owner stopped unexpectedly; restart to retry".into());
        }
        self.management.status.store(Arc::new(status));
    }
}
struct Midir;
impl Backend for Midir {
    type Port = MidiOutputPort;
    type Connection = MidiOutputConnection;
    fn discover(&mut self) -> Result<Inventory<Self::Port>, String> {
        let input = MidiInput::new("omatainer-route-inputs").map_err(|e| e.to_string())?;
        let inputs = input
            .ports()
            .iter()
            .map(|port| {
                Ok(Endpoint {
                    name: input.port_name(port).map_err(|e| e.to_string())?,
                    id: Some(port.id()),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let output = MidiOutput::new("omatainer-route-outputs").map_err(|e| e.to_string())?;
        let outputs = output
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
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Inventory { inputs, outputs })
    }
    fn connect(&mut self, port: &Self::Port) -> Result<Self::Connection, String> {
        MidiOutput::new("omatainer-route-output")
            .map_err(|e| e.to_string())?
            .connect(port, "omatainer-route-out")
            .map_err(|e| e.to_string())
    }
    fn send(&mut self, connection: &mut Self::Connection, bytes: &[u8]) -> Result<(), String> {
        connection.send(bytes).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
pub(crate) mod tests;
