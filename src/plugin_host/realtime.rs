use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
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
    pub upstream_delay: u32,
}
impl Default for Context {
    fn default() -> Self {
        Self {
            bpm: 120.,
            beat: 0.,
            sample_position: 0,
            playing: false,
            signature: [4, 4],
            upstream_delay: 0,
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
    latency: u32,
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
            latency: 0,
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
    editor: AtomicU8,
    editor_open: AtomicBool,
    editor_busy: AtomicBool,
    values: Vec<AtomicU64>,
    editor_error: Mutex<Option<String>>,
}
struct Snapshot {
    after: Option<u64>,
    reply: std::sync::mpsc::SyncSender<Result<Saved, String>>,
    deadline: Instant,
}
struct Barrier {
    after: Option<u64>,
    reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    deadline: Instant,
}
enum Job {
    Snapshot(Snapshot),
    Barrier(Barrier),
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
    /// Wait for submitted audio only on an independent offline render worker.
    /// Takes cancellation; returns after the worker publishes all preceding blocks, or a bounded refusal.
    pub fn barrier(&self, cancel: &AtomicBool) -> Result<(), String> {
        let (reply, receiver) = std::sync::mpsc::sync_channel(1);
        let seq = self.shared.submitted.load(Ordering::Acquire);
        self.jobs.try_send(Job::Barrier(Barrier { after: (seq > 0).then_some(seq - 1), reply, deadline: Instant::now() + Duration::from_secs(10) })).map_err(|_| "Plugin render owner is busy".to_string())?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if cancel.load(Ordering::Acquire) { return Err("Plugin render cancelled".into()); }
            if Instant::now() >= deadline { return Err("Plugin render timed out".into()); }
            match receiver.recv_timeout(Duration::from_millis(10)) {
                Ok(result) => return result,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err("Plugin render owner disconnected".into()),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
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
    /// Request native editor visibility on the isolated owner.
    /// Takes desired visibility; returns atomic admission without locks, allocation or third-party calls.
    pub fn editor(&self, open: bool) -> bool {
        if open && !self.class.info.has_gui || self.shared.fault.load(Ordering::Acquire) || self.shared.cancel.load(Ordering::Acquire) { return false; }
        self.shared.editor_busy.store(true,Ordering::Release);
        self.shared.editor.store(if open {2} else {1},Ordering::Release); true
    }
    /// Read an isolated processor's latest normalized control value.
    /// Takes a stable writable parameter ID; returns an atomic worker observation for the bounded generic editor.
    pub fn value(&self,id:u32) -> Option<f64> { self.class.parameters.iter().filter(|p|!p.is_read_only).take(128).position(|p|p.id==id).and_then(|index|self.shared.values.get(index)).map(|value|f64::from_bits(value.load(Ordering::Acquire))) }
    pub fn editing(&self) -> bool { self.editor_open() || self.shared.editor_busy.load(Ordering::Acquire) }
    pub fn editor_open(&self) -> bool { self.shared.editor_open.load(Ordering::Acquire) }
    pub fn editor_error(&self) -> Option<String> { self.shared.editor_error.lock().unwrap_or_else(|e|e.into_inner()).clone() }
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
            .clone().or_else(|| self.shared.fault.load(Ordering::Acquire).then(|| "Processor stopped after an event, output or latency bound was exceeded".into()))
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
    held_notes: [[bool;128];16],
    pending_parameters: [Parameter; 512],
    pending_parameter_count: usize,
    prepared: bool,
    latency: u32,
    output_delay: Option<u32>,
    available: bool,
    pub control: Control,
}
/// Share read-only endpoint metadata across queued command references.
/// Ring buffers are private and every access requires exclusive mutable ownership; shared operations use only the atomic control handle.
unsafe impl Sync for Endpoint {}
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
            editor: AtomicU8::new(0),
            editor_open: AtomicBool::new(false),
            editor_busy: AtomicBool::new(false),
            values: class.parameters.iter().filter(|p|!p.is_read_only).take(128).map(|p|AtomicU64::new(p.value.to_bits())).collect(),
            editor_error: Mutex::new(None),
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
            latency: control.latency(),
            output_delay: None,
            available: false,
            prepared: false,
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
            held_notes: [[false;128];16],
            pending_parameters: [Parameter::default(); 512],
            pending_parameter_count: 0,
            control,
        })
    }
    fn boundary(&mut self, context: Context) {
        self.output.fill([0.; BLOCK]);
        self.available = false;
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
                self.latency = packet.latency.saturating_add(BLOCK as u32 * BRIDGE_BLOCKS as u32);
                self.output_delay = Some(self.latency.saturating_add(packet.context.upstream_delay));
                self.available = true;
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
    /// Publish a block's reported delay before graph alignment begins.
    /// Takes exact input-time context; returns the delay belonging to this output block without waiting.
    pub(crate) fn begin_frame(&mut self, context: Context) -> u32 {
        if self.offset == 0 && !self.prepared { self.boundary(context); self.prepared = true; }
        self.latency
    }
    pub(crate) fn block_offset(&self) -> usize { self.offset }
    pub(crate) fn output_delay(&self) -> Option<u32> { self.output_delay }
    pub(crate) fn available(&self) -> bool { self.available && !self.faulted() }
    pub(crate) fn recontextualize(&mut self, context: Context) { if self.offset == 0 { if let Some(packet) = &mut self.current { packet.context = context; } } }
    pub(crate) fn faulted(&self) -> bool { self.control.shared.fault.load(Ordering::Acquire) }
    /// Stop a failed event stream without leaving a sounding worker voice.
    /// Takes this endpoint; retires its isolated process while graph output becomes silent.
    pub(crate) fn refuse(&self) { self.control.shared.fault.store(true, Ordering::Release); self.control.shared.cancel.store(true, Ordering::Release); }
    pub(crate) fn bytes(&self) -> usize {
        let class = &self.control.class;
        POOL * std::mem::size_of::<Packet>() + std::mem::size_of::<Self>() + std::mem::size_of::<Class>()
            + class.info.path.as_os_str().len() + class.info.name.capacity() + class.info.vendor.capacity() + class.info.version.capacity() + class.info.category.capacity() + class.info.uid.capacity()
            + (class.layout.inputs.capacity() + class.layout.outputs.capacity()) * std::mem::size_of::<vst3_host::AudioBusConfig>()
            + self.control.shared.values.capacity() * std::mem::size_of::<AtomicU64>()
            + class.parameters.capacity() * std::mem::size_of::<vst3_host::Parameter>()
            + class.parameters.iter().map(|p|p.name.capacity()+p.unit.capacity()).sum::<usize>()
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
        if matches!(bytes[0] & 0xf0,0x80 | 0x90) && bytes[1] < 128 { self.held_notes[(bytes[0] & 15) as usize][bytes[1] as usize] = bytes[0] & 0xf0 == 0x90 && bytes[2] > 0; }
        true
    }
    /// Release the notes this processor actually received.
    /// Takes exclusive endpoint ownership; queues individual note offs or stops an overflowing processor without relying on optional controller mappings.
    pub(crate) fn release_notes(&mut self) {
        for channel in 0..16 { for note in 0..128 {
            if self.held_notes[channel][note] && !self.midi([0x80 | channel as u8,note as u8,0]) { self.refuse(); return; }
        } }
    }
    /// Queue normalized automation at the current sample offset.
    /// Takes parameter identity and value; returns false on invalid values or fixed event/pool exhaustion.
    pub fn parameter(&mut self, id: u32, value: f64) -> bool {
        if !value.is_finite()
            || !(0.0..=1.0).contains(&value)
            || !self.parameter_room()
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
    /// Check admission for one sample-offset parameter event.
    /// Takes the endpoint; returns whether its prepared queue can accept an edit without allocation.
    pub fn parameter_room(&self) -> bool {
        self.parameter_slots() > 0
    }
    pub fn parameter_slots(&self) -> usize { if self.faulted() {0} else {512usize.saturating_sub(self.pending_parameter_count + self.current.as_ref().map_or(0, |p| p.parameter_count))} }
    /// Advance one sample without locks, IPC, allocation or waiting.
    /// Takes flattened bus inputs and exact transport context at block starts; returns flattened outputs at the fixed bridge delay, with silence for late or failed workers.
    pub fn tick(&mut self, input: [f32; MAX_CHANNELS], context: Context) -> [f32; MAX_CHANNELS] {
        self.begin_frame(context);
        if self.faulted() { return [0.; MAX_CHANNELS]; }
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
            self.prepared = false;
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
/// Publish bounded writable parameter observations from the isolated processor.
/// Takes qualified class IDs, shared scalar storage and worker values; rejects reordered, missing or invalid values before touching controls.
fn publish_parameters(class:&Class,shared:&Shared,values:&[(u32,f64)]) -> Result<(),String> {
    if values.len() != shared.values.len() || values.iter().zip(class.parameters.iter().filter(|p|!p.is_read_only).take(128)).any(|((id,value),p)|*id!=p.id || !value.is_finite() || !(0.0..=1.0).contains(value)) { return Err("Plugin writable parameters changed or returned invalid values; review a compatible replacement".into()); }
    for (target,(_,value)) in shared.values.iter().zip(values) { target.store(value.to_bits(),Ordering::Release); }
    Ok(())
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
        let editor = shared.editor.swap(0,Ordering::AcqRel);
        if editor != 0 {
            match process.exchange(&Request::Editor {open:editor == 2},&shared.cancel,Duration::from_secs(3)) {
                Ok(Response::Ok) => { shared.editor_open.store(editor == 2,Ordering::Release); *shared.editor_error.lock().unwrap_or_else(|e|e.into_inner()) = None; }
                Ok(_) => return Err("Invalid plugin editor response".into()),
                Err(error) => { *shared.editor_error.lock().unwrap_or_else(|e|e.into_inner()) = Some(error); }
            }
            shared.editor_busy.store(false,Ordering::Release);
        }
        if pending.is_none() {
            pending = jobs.try_recv().ok();
        }
        let expired = match &pending {
            Some(Job::Snapshot(job)) => job.deadline <= Instant::now(),
            Some(Job::Barrier(job)) => job.deadline <= Instant::now(),
            _ => false,
        };
        if expired {
            match pending.take().unwrap() {
                Job::Snapshot(job) => { let _ = job.reply.try_send(Err("Plugin state capture expired".into())); }
                Job::Barrier(job) => { let _ = job.reply.try_send(Err("Plugin render expired".into())); }
            }
        }
        let ready = match &pending {
            Some(Job::Snapshot(job)) => {
                job.after.is_none()
                    || last.is_some_and(|seq| job.after.is_some_and(|after| seq >= after))
            }
            Some(Job::Barrier(job)) => job.after.is_none() || last.is_some_and(|seq| job.after.is_some_and(|after| seq >= after)),
            None => false,
        };
        if ready {
            match pending.take().unwrap() {
                Job::Barrier(job) => { let _ = job.reply.try_send(Ok(())); }
                Job::Snapshot(job) => {
                    let result = match process.exchange(
                        &Request::State,
                        &shared.cancel,
                        Duration::from_secs(3),
                    ) {
                        Ok(Response::State { saved,parameters }) => saved.validate().and_then(|_|publish_parameters(class,shared,&parameters)).map(|_| saved),
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
            outputs, latency, editor_open, parameters, ..
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
        publish_parameters(class,shared,&parameters)?;
        shared.latency.store(latency, Ordering::Release);
        shared.editor_open.store(editor_open,Ordering::Release);
        packet.latency = latency;
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
