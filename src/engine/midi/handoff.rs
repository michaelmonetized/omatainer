//! The native MIDI callback only copies bounded bytes and updates atomics.
//! Parsing, admission, learning and logging belong to each input's worker.
use super::{handle_msg, Action, MidiMap, MsgKind};
use crate::engine::{Command, CommandPort};
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::*};
use std::sync::Arc;
use std::time::Duration;

mod latest;

const EVENT_BYTES: usize = 256;
const EVENTS: usize = 256;

#[derive(Clone, Copy)]
struct Event {
    epoch: u64,
    sequence: u64,
    safety: u64,
    routing: u64,
    learning: u64,
    len: u16,
    bytes: [u8; EVENT_BYTES],
}
impl Event {
    fn new(epoch: u64, sequence: u64, bytes: &[u8]) -> Self {
        let mut event = Self {
            epoch,
            sequence,
            safety: 0,
            routing: 0,
            learning: 0,
            len: bytes.len() as u16,
            bytes: [0; EVENT_BYTES],
        };
        event.bytes[..bytes.len()].copy_from_slice(bytes);
        event
    }
    fn bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }
}

#[derive(Default)]
pub struct InputCounters {
    received: AtomicU64,
    queued: AtomicU64,
    coalesced: AtomicU64,
    dropped: AtomicU64,
    oversized: AtomicU64,
    disconnected: AtomicU64,
    resets: AtomicU64,
    dispatched: AtomicU64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct InputStats {
    pub received: u64,
    pub queued: u64,
    pub coalesced: u64,
    pub dropped: u64,
    pub oversized: u64,
    pub disconnected: u64,
    pub resets: u64,
    pub dispatched: u64,
}
impl InputCounters {
    pub fn snapshot(&self) -> InputStats {
        InputStats {
            received: self.received.load(Relaxed),
            queued: self.queued.load(Relaxed),
            coalesced: self.coalesced.load(Relaxed),
            dropped: self.dropped.load(Relaxed),
            oversized: self.oversized.load(Relaxed),
            disconnected: self.disconnected.load(Relaxed),
            resets: self.resets.load(Relaxed),
            dispatched: self.dispatched.load(Relaxed),
        }
    }
}

struct Shared {
    learning: Arc<super::learn::Shared>,
    epoch: AtomicU64,
    performance: crate::engine::performance::Handle,
    stop_pending: AtomicBool,
    shutdown: AtomicBool,
    enabled: AtomicBool,
    counters: Arc<InputCounters>,
    routing: Arc<super::routing::Shared>,
}

struct Rules {
    cc: [[bool; 128]; 16],
    stop: [[bool; 128]; 16],
}
impl Rules {
    fn new(map: &MidiMap) -> Self {
        let mut rules = Self {
            cc: [[false; 128]; 16],
            stop: [[false; 128]; 16],
        };
        for binding in &map.bindings {
            if binding.data >= 128 {
                continue;
            }
            for channel in 0..16 {
                if binding.ch != 0xff && binding.ch as usize != channel {
                    continue;
                }
                if binding.kind == MsgKind::Note && binding.action == Action::Stop {
                    rules.stop[channel][binding.data as usize] = true;
                }
                // Only direct absolute assignments. Relative jog, browse,
                // selection, transport and every note remain ordered barriers.
                if binding.kind == MsgKind::Cc
                    && matches!(
                        binding.action,
                        Action::DeckPitch
                            | Action::DeckGain
                            | Action::DeckEqHi
                            | Action::DeckEqMid
                            | Action::DeckEqLow
                            | Action::DeckFilter
                            | Action::Xfader
                            | Action::Master
                            | Action::CueMix
                            | Action::TrackFader
                            | Action::FxWet
                    )
                {
                    rules.cc[channel][binding.data as usize] = true;
                }
            }
        }
        rules
    }
    fn coalescible(&self, bytes: &[u8]) -> bool {
        bytes.len() == 3
            && bytes[0] & 0xf0 == 0xb0
            && bytes[1] < 128
            && bytes[2] < 128
            && self.cc[(bytes[0] & 15) as usize][bytes[1] as usize]
    }
    fn has_stop(&self, bytes: &[u8]) -> bool {
        // Bounded overflow-only scan preserves stop even with interleaved
        // realtime bytes. Normal MIDI parsing remains in handle_msg.
        let mut note = [0; 3];
        let mut len = 0;
        for &byte in bytes {
            if byte == 0xfc {
                return true;
            }
            if byte >= 0xf8 {
                continue;
            }
            if byte >= 0x80 {
                note[0] = byte;
                len = 1;
                continue;
            }
            if len == 0 {
                continue;
            }
            note[len] = byte;
            len += 1;
            if len == 3 {
                if note[0] & 0xf0 == 0x90
                    && note[2] > 0
                    && self.stop[(note[0] & 15) as usize][note[1] as usize]
                {
                    return true;
                }
                len = 0;
            }
        }
        false
    }
}

pub(super) struct InputSink {
    producer: rtrb::Producer<Event>,
    shared: Arc<Shared>,
    rules: Rules,
    pending_cc: latest::Writer,
    sequence: u64,
}
impl InputSink {
    fn overflow(&mut self, bytes: &[u8]) {
        self.shared.counters.dropped.fetch_add(1, Relaxed);
        if self.pending_cc.take().is_some() {
            self.shared.counters.dropped.fetch_add(1, Relaxed);
        }
        if bytes.len() > EVENT_BYTES || self.shared.learning.ordered.load(Acquire) || self.rules.has_stop(bytes) {
            self.shared.stop_pending.store(true, Release);
        }
        self.shared.epoch.fetch_add(1, Release);
    }
    pub(super) fn push(&mut self, bytes: &[u8]) {
        self.shared.counters.received.fetch_add(1, Relaxed);
        if self.shared.shutdown.load(Acquire) || self.producer.is_abandoned() {
            self.shared.counters.disconnected.fetch_add(1, Relaxed);
            return;
        }
        // Newly connecting inputs remain inert until the manager rechecks
        // current preferences after the potentially blocking OS open call.
        if !self.shared.enabled.load(Acquire) {
            self.shared.counters.dropped.fetch_add(1, Relaxed);
            return;
        }
        if bytes.len() > EVENT_BYTES {
            self.shared.counters.oversized.fetch_add(1, Relaxed);
            self.overflow(bytes); // Never scans/copies an unbounded callback payload.
            return;
        }
        let epoch = self.shared.epoch.load(Acquire);
        self.sequence = self.sequence.wrapping_add(1);
        let mut event = Event::new(epoch, self.sequence, bytes);
        event.safety = self.shared.performance.input_epoch();
        event.routing = self.shared.routing.generation.load(Acquire);
        event.learning = self.shared.learning.revision.load(Acquire);
        let cc = !self.shared.learning.ordered.load(Acquire) && !self.shared.routing.explicit.load(Acquire) && self.rules.coalescible(bytes);
        if cc && self.pending_cc.same_pending_key(&event) {
            if self.pending_cc.publish(event) {
                self.shared.counters.coalesced.fetch_add(1, Relaxed);
            } else {
                self.shared.counters.queued.fetch_add(1, Relaxed);
            }
            return;
        }
        // A producer claims any older pending assignment before crossing a
        // different control/note. The reader may already own it; its sequence
        // merge then preserves that older event ahead of this new barrier.
        if let Some(pending) = self.pending_cc.take() {
            if self.producer.push(pending).is_err() {
                self.shared.counters.dropped.fetch_add(1, Relaxed);
                self.overflow(bytes);
                return;
            }
        }
        match self.producer.push(event) {
            Ok(()) => {
                self.shared.counters.queued.fetch_add(1, Relaxed);
            }
            Err(_) if cc => {
                self.pending_cc.publish(event);
                self.shared.counters.queued.fetch_add(1, Relaxed);
            }
            Err(_) => self.overflow(bytes),
        }
    }
}

struct InputWorker {
    decoder: super::surface::Decoder,
    learning_revision: u64,
    consumer: rtrb::Consumer<Event>,
    shared: Arc<Shared>,
    epoch: u64,
    safety: u64,
    pending_cc: latest::Reader,
    cached_cc: Option<Event>,
    cached_fifo: Option<Event>,
    source: u64,
    sources: super::routing::Sources,
    map: MidiMap,
    cmd: CommandPort,
    log: Arc<Mutex<Vec<String>>>,
    shift: Arc<Mutex<[bool; 4]>>,
    name: String,
    port_id: String,
}
impl InputWorker {
    fn reset(&mut self) {
        self.decoder.reset();
        self.shared.routing.release(self.sources,&self.cmd);
        *self.shift.lock() = [false; 4];
        self.shared.counters.resets.fetch_add(1, Relaxed);
    }
    fn step(&mut self) -> bool {
        let learning = self.shared.learning.revision.load(Acquire);
        if learning != self.learning_revision { self.reset(); self.learning_revision = learning; }
        let safety = self.shared.performance.input_epoch();
        if self.safety != safety { *self.shift.lock() = [false; 4]; self.decoder.reset(); self.safety = safety; }
        let epoch = self.shared.epoch.load(Acquire);
        let reset = epoch != self.epoch;
        if reset {
            self.reset();
            self.epoch = epoch;
        }
        if self.shared.stop_pending.swap(false, AcqRel) {
            let _ = self.cmd.send(Command::Stop);
        }
        let Some(event) = self.next_event() else {
            return reset;
        };
        if event.epoch == self.shared.epoch.load(Acquire) && event.epoch == self.epoch && event.safety == safety && event.learning == learning {
            let cmd=self.cmd.for_input_epoch(event.safety);
            for frame in super::routing::packet::frames(event.bytes()) {
                match frame {
                    super::routing::packet::Frame::Musical(packet) => {
                        if packet.bytes().len()==3 && packet.channel().is_some() {
                            let message:[u8;3]=packet.bytes().try_into().unwrap();
                            match self.shared.learning.input_at(self.source,&self.name,&self.port_id,&message,&self.map,event.learning) {
                                super::learn::Dispatch::Consume => continue,
                                super::learn::Dispatch::Binding(binding) => { let _=super::dispatch(&binding,self.source,message[0]&0xf0,message[2],&message,&cmd,&self.shift);continue; },
                                super::learn::Dispatch::Normal => {},
                            }
                        }
                        self.shared.routing.input(self.sources,&self.name,&self.port_id,event.routing,packet,&cmd,|allow_live| {
                            if packet.bytes().len()==3 && packet.channel().is_some() {
                                let frame:[u8;3]=packet.bytes().try_into().unwrap();
                                if !self.decoder.input(&self.map,&frame,&cmd) {
                                    super::handle_channel(&frame,self.source,&self.map,&cmd,&self.log,&self.shift,&self.name,allow_live);
                                }
                            }
                        });
                    },
                    super::routing::packet::Frame::Realtime(status) => handle_msg(&[status],self.source,&self.map,&cmd,&self.log,&self.shift,&self.name),
                    super::routing::packet::Frame::Malformed => self.shared.routing.malformed(),
                }
            }
            self.shared.counters.dispatched.fetch_add(1, Relaxed);
        } else {
            if event.learning != learning && Rules::new(&self.map).has_stop(event.bytes()) { let _=self.cmd.send(Command::Stop); }
            self.shared.counters.dropped.fetch_add(1, Relaxed);
        }
        true
    }
    fn next_event(&mut self) -> Option<Event> {
        self.next_event_after_probe(|| {})
    }
    fn next_event_after_probe(&mut self, after_fifo_probe: impl FnOnce()) -> Option<Event> {
        // Take the mailbox BEFORE probing FIFO: all earlier FIFO publications
        // happened before this CC. Keep both heads until their sequence wins.
        // A producer racing an empty FIFO probe is handled on the next step,
        // never by dispatching a new mailbox value past that empty observation.
        if self.cached_cc.is_none() {
            self.cached_cc = self.pending_cc.take();
        }
        if self.cached_fifo.is_none() {
            self.cached_fifo = self.consumer.pop().ok();
        }
        after_fifo_probe();
        match (self.cached_fifo, self.cached_cc) {
            (Some(fifo), Some(cc)) if fifo.sequence <= cc.sequence => self.cached_fifo.take(),
            (Some(_), Some(_)) | (None, Some(_)) => self.cached_cc.take(),
            (Some(_), None) => self.cached_fifo.take(),
            (None, None) => None,
        }
    }
    fn run(mut self) {
        while !self.shared.shutdown.load(Acquire) && !self.consumer.is_abandoned() {
            let mut worked = false;
            for _ in 0..64 {
                if !self.step() {
                    break;
                }
                worked = true;
            }
            if !worked {
                std::thread::park_timeout(Duration::from_millis(1));
            }
        }
        self.finish();
    }
    fn finish(&mut self) {
        // Disconnect never replays queued onsets, but already handed-off stop
        // messages still retain their safety meaning. Callback closure or the
        // shutdown flag prevents new queue admission during this bounded drain.
        let rules = Rules::new(&self.map);
        let mut stop = self.shared.stop_pending.swap(false, AcqRel);
        while let Some(event) = self.next_event() {
            stop |= rules.has_stop(event.bytes());
            self.shared.counters.dropped.fetch_add(1, Relaxed);
        }
        let _ = self.pending_cc.take();
        self.reset();
        if stop {
            let _ = self.cmd.send(Command::Stop);
        }
    }
}

pub(super) struct InputGuard {
    shared: Arc<Shared>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl InputGuard {
    pub(super) fn enable(&self) {
        self.shared.enabled.store(true, Release);
    }
}
impl Drop for InputGuard {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

fn channel(
    capacity: usize,
    source: u64,
    map: MidiMap,
    cmd: CommandPort,
    log: Arc<Mutex<Vec<String>>>,
    name: String,
    port_id:String,
    counters: Arc<InputCounters>,
) -> std::io::Result<(InputSink, InputWorker)> {
    let routing=cmd.midi_routing().clone();
    let sources=routing.register(source,&name,&port_id)?;
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    let shared = Arc::new(Shared {
        learning: cmd.midi_learn(),
        epoch: AtomicU64::new(0),
        performance: cmd.performance().clone(),
        stop_pending: AtomicBool::new(false),
        shutdown: AtomicBool::new(false),
        enabled: AtomicBool::new(true),
        counters,
        routing,
    });
    shared.learning.connected(source,&name,&port_id);
    let rules = Rules::new(&map);
    let (pending_writer, pending_reader) = latest::channel();
    Ok((
        InputSink {
            producer,
            shared: shared.clone(),
            rules,
            pending_cc: pending_writer,
            sequence: 0,
        },
        InputWorker {
            decoder: super::surface::Decoder::default(),
            learning_revision: shared.learning.revision.load(Acquire),
            consumer,
            shared,
            epoch: 0,
            safety: cmd.performance().input_epoch(),
            pending_cc: pending_reader,
            cached_cc: None,
            cached_fifo: None,
            source,
            sources,
            map,
            cmd,
            log,
            shift: Arc::new(Mutex::new([false; 4])),
            port_id,
            name,
        },
    ))
}

pub(super) fn start(
    source: u64,
    map: MidiMap,
    cmd: CommandPort,
    log: Arc<Mutex<Vec<String>>>,
    name: String,
    counters: Arc<InputCounters>,
) -> std::io::Result<(InputSink, InputGuard)> {
    start_with_completion(source, map, cmd, log,  name, counters, || {})
}

pub(super) fn start_with_completion(
    source: u64,
    map: MidiMap,
    cmd: CommandPort,
    log: Arc<Mutex<Vec<String>>>,
    name: String,
    counters: Arc<InputCounters>,
    completed: impl FnOnce() + Send + 'static,
) -> std::io::Result<(InputSink, InputGuard)> {
    start_gated(
        source, map, cmd, log,  name, counters, true, completed,
    )
}

pub(super) fn start_gated(
    source: u64,
    map: MidiMap,
    cmd: CommandPort,
    log: Arc<Mutex<Vec<String>>>,
    name: String,
    counters: Arc<InputCounters>,
    enabled: bool,
    completed: impl FnOnce() + Send + 'static,
) -> std::io::Result<(InputSink, InputGuard)> {
    let id=name.clone();
    start_on_port(source,map,cmd,log,name,id,counters,enabled,completed)
}
pub(super) fn start_on_port(
    source:u64,map:MidiMap,cmd:CommandPort,log:Arc<Mutex<Vec<String>>>,
    name:String,id:String,counters:Arc<InputCounters>,enabled:bool,completed:impl FnOnce()+Send+'static,
) -> std::io::Result<(InputSink,InputGuard)> {
    let (sink, worker) = channel(EVENTS, source, map, cmd, log,  name, id, counters)?;
    sink.shared.enabled.store(enabled, Release);
    let shared = sink.shared.clone();
    let worker = std::thread::Builder::new()
        .name(format!("omatainer-midi-{source}"))
        .spawn(move || {
            worker.run();
            completed();
        })?;
    Ok((
        sink,
        InputGuard {
            shared,
            worker: Some(worker),
        },
    ))
}
impl Drop for InputWorker {
    fn drop(&mut self) {
        self.shared.learning.disconnected(self.source);
        self.shared.routing.release(self.sources,&self.cmd);
        self.shared.routing.unregister(self.source);
    }
}

#[cfg(test)]
mod tests;
