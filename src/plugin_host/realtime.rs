use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const POOL: usize = 8;
pub(crate) const BRIDGE_BLOCKS: u64 = 3;
#[derive(Clone, Copy, Debug)]
pub(crate) struct Context {
    pub bpm: f64,
    pub beat: f64,
    pub sample_position: i64,
    pub playing: bool,
    pub signature: [i32; 2],
}
impl Default for Context {
    fn default() -> Self {
        Self {
            bpm: 120.,
            beat: 0.,
            sample_position: 0,
            playing: false,
            signature: [4, 4],
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Parameter {
    id: u32,
    value: f64,
    offset: i32,
}
#[derive(Clone, Copy, Debug, Default)]
struct Midi {
    offset: i32,
    bytes: [u8; 3],
}
struct Packet {
    sequence: u64,
    context: Context,
    input: [[f32; BLOCK]; MAX_CHANNELS],
    output: [[f32; BLOCK]; MAX_CHANNELS],
    parameters: [Parameter; 512],
    parameter_count: usize,
    midi: [Midi; 512],
    midi_count: usize,
}
impl Packet {
    fn new() -> Self {
        Self {
            sequence: 0,
            context: Context::default(),
            input: [[0.; BLOCK]; MAX_CHANNELS],
            output: [[0.; BLOCK]; MAX_CHANNELS],
            parameters: [Parameter::default(); 512],
            parameter_count: 0,
            midi: [Midi::default(); 512],
            midi_count: 0,
        }
    }
}
struct Shared {
    cancel: AtomicBool,
    fault: AtomicBool,
    error: Mutex<Option<String>>,
    latency: AtomicU32,
    missed: AtomicU64,
    submitted: AtomicU64,
}
struct Snapshot {
    after: Option<u64>,
    reply: std::sync::mpsc::SyncSender<Result<Saved, String>>,
    deadline: Instant,
}
enum Job {
    Snapshot(Snapshot),
    Editor(bool),
}
#[derive(Clone)]
pub(crate) struct Control {
    shared: Arc<Shared>,
    jobs: crossbeam_channel::Sender<Job>,
    pub class: Arc<Class>,
}
impl std::fmt::Debug for Control {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginControl")
            .field("class", &self.class.info.uid)
            .field("fault", &self.shared.fault.load(Ordering::Acquire))
            .finish()
    }
}
impl Control {
    /// Capture opaque state on the plugin owner after already-submitted blocks.
    /// Takes the caller's cancellation flag; returns bounded identity-bound state without calling plugin code on audio or UI threads.
    pub fn snapshot(&self, cancel: &AtomicBool) -> Result<Saved, String> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let seq = self.shared.submitted.load(Ordering::Acquire);
        let deadline = Instant::now() + Duration::from_secs(10);
        self.jobs
            .try_send(Job::Snapshot(Snapshot {
                after: (seq > 0).then_some(seq - 1),
                reply: tx,
                deadline,
            }))
            .map_err(|_| "Plugin state work is busy or unavailable".to_string())?;
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("Plugin state capture cancelled".into());
            }
            if Instant::now() >= deadline {
                return Err("Plugin state capture timed out".into());
            }
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(result) => return result,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Plugin state owner disconnected".into())
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
    /// Queue a native editor operation. Takes the requested visibility; returns queue admission without waiting for third-party code.
    pub fn editor(&self, open: bool) -> Result<(), String> {
        self.jobs
            .try_send(Job::Editor(open))
            .map_err(|_| "Plugin editor work is busy or unavailable".into())
    }
    pub fn latency(&self) -> u32 {
        self.shared
            .latency
            .load(Ordering::Acquire)
            .saturating_add(BLOCK as u32 * BRIDGE_BLOCKS as u32)
    }
    pub fn missed_blocks(&self) -> u64 {
        self.shared.missed.load(Ordering::Acquire)
    }
    pub fn error(&self) -> Option<String> {
        self.shared
            .error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

pub(crate) struct Endpoint {
    requests: rtrb::Producer<Box<Packet>>,
    responses: rtrb::Consumer<Box<Packet>>,
    free: Vec<Box<Packet>>,
    ready: [Option<Box<Packet>>; POOL],
    current: Option<Box<Packet>>,
    output: [[f32; BLOCK]; MAX_CHANNELS],
    sequence: u64,
    offset: usize,
    pending_midi: [Midi; 512],
    pending_midi_count: usize,
    pending_parameters: [Parameter; 512],
    pending_parameter_count: usize,
    pub control: Control,
}
impl std::fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginEndpoint")
            .field("control", &self.control)
            .field("sequence", &self.sequence)
            .finish()
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        self.control.shared.cancel.store(true, Ordering::Release);
    }
}
impl Endpoint {
    /// Prepare an isolated processor and its bounded audio exchange.
    /// Takes saved state and the actual output rate on a setup worker; returns a single-owner audio endpoint with three blocks of explicit bridge latency.
    pub fn start(saved: Saved, rate: u32, cancel: &AtomicBool) -> Result<Self, String> {
        saved.validate()?;
        let mut process = process::Process::start(&executable()?)?;
        let class = match process.exchange(
            &Request::Load { saved, rate },
            cancel,
            Duration::from_secs(10),
        )? {
            Response::Loaded { class } => class,
            _ => return Err("Invalid plugin load acknowledgement".into()),
        };
        class.validate()?;
        let class = Arc::new(class);
        let shared = Arc::new(Shared {
            cancel: AtomicBool::new(false),
            fault: AtomicBool::new(false),
            error: Mutex::new(None),
            latency: AtomicU32::new(class.latency),
            missed: AtomicU64::new(0),
            submitted: AtomicU64::new(0),
        });
        let (requests, rx) = rtrb::RingBuffer::new(POOL);
        let (tx, responses) = rtrb::RingBuffer::new(POOL);
        let (jobs, job_rx) = crossbeam_channel::bounded(8);
        let control = Control {
            shared: shared.clone(),
            jobs,
            class: class.clone(),
        };
        std::thread::Builder::new()
            .name("plugin-audio-owner".into())
            .spawn(move || {
                let result = owner(&mut process, rx, tx, job_rx, &class, &shared);
                if let Err(error) = result {
                    *shared.error.lock().unwrap_or_else(|e| e.into_inner()) = Some(error);
                    shared.fault.store(true, Ordering::Release);
                }
            })
            .map_err(|e| e.to_string())?;
        let mut free = Vec::with_capacity(POOL);
        for _ in 0..POOL {
            free.push(Box::new(Packet::new()));
        }
        Ok(Self {
            requests,
            responses,
            free,
            ready: std::array::from_fn(|_| None),
            current: None,
            output: [[0.; BLOCK]; MAX_CHANNELS],
            sequence: 0,
            offset: 0,
            pending_midi: [Midi::default(); 512],
            pending_midi_count: 0,
            pending_parameters: [Parameter::default(); 512],
            pending_parameter_count: 0,
            control,
        })
    }
    fn boundary(&mut self, context: Context) {
        self.output.fill([0.; BLOCK]);
        while let Ok(packet) = self.responses.pop() {
            let slot = packet.sequence as usize % POOL;
            if self.ready[slot].is_some() {
                self.free.push(packet);
                self.control.shared.missed.fetch_add(1, Ordering::Relaxed);
            } else {
                self.ready[slot] = Some(packet);
            }
        }
        if self.sequence >= BRIDGE_BLOCKS {
            let expected = self.sequence - BRIDGE_BLOCKS;
            let slot = expected as usize % POOL;
            if self.ready[slot]
                .as_ref()
                .is_some_and(|p| p.sequence == expected)
            {
                let packet = self.ready[slot].take().unwrap();
                self.output = packet.output;
                self.free.push(packet);
            } else {
                self.control.shared.missed.fetch_add(1, Ordering::Relaxed);
            }
            for slot in &mut self.ready {
                if slot.as_ref().is_some_and(|p| p.sequence < expected) {
                    self.free.push(slot.take().unwrap());
                }
            }
        }
        self.current = self.free.pop();
        if let Some(packet) = &mut self.current {
            packet.sequence = self.sequence;
            packet.context = context;
            packet.parameter_count = 0;
            packet.midi_count = 0;
        }
    }
    /// Queue a channel message at the current sample offset.
    /// Takes three MIDI bytes; returns false on the fixed event/pool bound without allocation or waiting.
    pub fn midi(&mut self, bytes: [u8; 3]) -> bool {
        let occupied = self.current.as_ref().map_or(0, |p| p.midi_count);
        if self.pending_midi_count + occupied >= 512 {
            return false;
        }
        self.pending_midi[self.pending_midi_count] = Midi {
            offset: self.offset as i32,
            bytes,
        };
        self.pending_midi_count += 1;
        true
    }
    /// Queue normalized automation at the current sample offset.
    /// Takes parameter identity and value; returns false on invalid values or fixed event/pool exhaustion.
    pub fn parameter(&mut self, id: u32, value: f64) -> bool {
        let occupied = self.current.as_ref().map_or(0, |p| p.parameter_count);
        if !value.is_finite()
            || !(0.0..=1.0).contains(&value)
            || self.pending_parameter_count + occupied >= 512
        {
            return false;
        }
        self.pending_parameters[self.pending_parameter_count] = Parameter {
            id,
            value,
            offset: self.offset as i32,
        };
        self.pending_parameter_count += 1;
        true
    }
    /// Advance one sample without locks, IPC, allocation or waiting.
    /// Takes flattened bus inputs and exact transport context at block starts; returns flattened outputs at the fixed bridge delay, with silence for late or failed workers.
    pub fn tick(&mut self, input: [f32; MAX_CHANNELS], context: Context) -> [f32; MAX_CHANNELS] {
        if self.offset == 0 {
            self.boundary(context);
        }
        if let Some(packet) = &mut self.current {
            for event in &self.pending_midi[..self.pending_midi_count] {
                packet.midi[packet.midi_count] = *event;
                packet.midi_count += 1;
            }
            for event in &self.pending_parameters[..self.pending_parameter_count] {
                packet.parameters[packet.parameter_count] = *event;
                packet.parameter_count += 1;
            }
        }
        self.pending_midi_count = 0;
        self.pending_parameter_count = 0;
        if let Some(packet) = &mut self.current {
            for (channel, value) in packet.input.iter_mut().zip(input) {
                channel[self.offset] = if value.is_finite() { value } else { 0. };
            }
        }
        let output = std::array::from_fn(|c| self.output[c][self.offset]);
        self.offset += 1;
        if self.offset == BLOCK {
            if let Some(packet) = self.current.take() {
                let sequence = packet.sequence;
                match self.requests.push(packet) {
                    Ok(()) => {
                        self.control
                            .shared
                            .submitted
                            .store(sequence + 1, Ordering::Release);
                    }
                    Err(rtrb::PushError::Full(packet)) => {
                        self.free.push(packet);
                        self.control.shared.missed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            self.offset = 0;
            self.sequence += 1;
        }
        output
    }
}
fn executable() -> Result<PathBuf, String> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os("OMATAINER_TEST_BIN") {
        return Ok(path.into());
    }
    std::env::current_exe().map_err(|e| e.to_string())
}
fn owner(
    process: &mut process::Process,
    mut requests: rtrb::Consumer<Box<Packet>>,
    mut responses: rtrb::Producer<Box<Packet>>,
    jobs: crossbeam_channel::Receiver<Job>,
    class: &Class,
    shared: &Shared,
) -> Result<(), String> {
    let mut last = None;
    let mut pending = None;
    while !shared.cancel.load(Ordering::Acquire) {
        if pending.is_none() {
            pending = jobs.try_recv().ok();
        }
        if let Some(Job::Snapshot(job)) = &pending {
            if job.deadline <= Instant::now() {
                let _ = job
                    .reply
                    .try_send(Err("Plugin state capture expired".into()));
                pending = None;
            }
        }
        let ready = match &pending {
            Some(Job::Snapshot(job)) => {
                job.after.is_none()
                    || last.is_some_and(|seq| job.after.is_some_and(|after| seq >= after))
            }
            Some(Job::Editor(_)) => true,
            None => false,
        };
        if ready {
            match pending.take().unwrap() {
                Job::Editor(open) => {
                    if !matches!(
                        process.exchange(
                            &Request::Editor { open },
                            &shared.cancel,
                            Duration::from_secs(3)
                        )?,
                        Response::Ok
                    ) {
                        return Err("Invalid plugin editor response".into());
                    }
                }
                Job::Snapshot(job) => {
                    let result = match process.exchange(
                        &Request::State,
                        &shared.cancel,
                        Duration::from_secs(3),
                    ) {
                        Ok(Response::State { saved }) => saved.validate().map(|_| saved),
                        Ok(_) => Err("Invalid plugin state response".into()),
                        Err(e) => Err(e),
                    };
                    let failed = result.is_err();
                    let _ = job.reply.try_send(result);
                    if failed {
                        return Err("Plugin state owner failed; current processor stopped".into());
                    }
                }
            }
        }
        let mut packet = match requests.pop() {
            Ok(packet) => packet,
            Err(_) => {
                std::thread::sleep(Duration::from_millis(1));
                continue;
            }
        };
        let mut offset = 0;
        let inputs = class
            .layout
            .inputs
            .iter()
            .map(|b| {
                let channels = packet.input[offset..offset + b.channel_count]
                    .iter()
                    .map(|c| c.to_vec())
                    .collect();
                offset += b.channel_count;
                channels
            })
            .collect();
        let c = packet.context;
        let frame = Frame {
            inputs,
            frames: BLOCK,
            bpm: c.bpm,
            beat: c.beat,
            sample_position: c.sample_position,
            playing: c.playing,
            signature: c.signature,
            parameters: packet.parameters[..packet.parameter_count]
                .iter()
                .map(|p| (p.id, p.value, p.offset))
                .collect(),
            midi: packet.midi[..packet.midi_count]
                .iter()
                .map(|m| (m.offset, m.bytes))
                .collect(),
        };
        let response = process.exchange(
            &Request::Process { frame },
            &shared.cancel,
            Duration::from_secs(3),
        )?;
        let Response::Audio {
            outputs, latency, ..
        } = response
        else {
            return Err("Invalid plugin audio response".into());
        };
        if outputs.len() != class.layout.outputs.len()
            || outputs.iter().zip(&class.layout.outputs).any(|(b, l)| {
                b.len() != l.channel_count
                    || b.iter()
                        .any(|c| c.len() != BLOCK || c.iter().any(|v| !v.is_finite()))
            })
        {
            return Err(
                "Plugin output layout changed or returned invalid audio; rebuild the graph".into(),
            );
        }
        packet.output.fill([0.; BLOCK]);
        for (target, source) in packet.output.iter_mut().zip(outputs.iter().flatten()) {
            target.copy_from_slice(source);
        }
        shared.latency.store(latency, Ordering::Release);
        last = Some(packet.sequence);
        loop {
            match responses.push(packet) {
                Ok(()) => break,
                Err(rtrb::PushError::Full(returned)) => packet = returned,
            };
            if shared.cancel.load(Ordering::Acquire) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
