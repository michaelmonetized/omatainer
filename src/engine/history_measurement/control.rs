//! One outstanding session operation, acknowledged at a renderer boundary.
//! No creative command, project undo, or performance-protection gate owns it.
use crossbeam_channel::{Receiver, Sender};
use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action { Start(u64), End(u64) }
#[derive(Clone, Copy, Debug)]
pub(super) struct Request { pub id: u64, pub action: Action }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome { Started, Ended, AlreadyActive, WrongSession, NoOutput, Unavailable }
impl Outcome {
    fn code(self) -> u64 { match self { Self::Started => 1, Self::Ended => 2,
        Self::AlreadyActive => 3, Self::WrongSession => 4, Self::NoOutput => 5, Self::Unavailable => 6 } }
    fn from_code(value: u64) -> Self { match value { 1 => Self::Started, 2 => Self::Ended,
        3 => Self::AlreadyActive, 4 => Self::WrongSession, 5 => Self::NoOutput, _ => Self::Unavailable } }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ack {
    pub request: u64,
    pub session: u64,
    pub outcome: Outcome,
    pub wall_ns: u64,
    pub frame: u64,
    pub rate: u32,
    pub incomplete: bool,
    pub dropped: u64,
    pub current: [u64; 2],
}

struct Shared {
    sender: Sender<Request>,
    observations: std::sync::Mutex<Option<Receiver<super::Observation>>>,
    next: AtomicU64,
    sessions: AtomicU64,
    pending: AtomicU64,
    connected: AtomicBool,
    measurement_available: AtomicBool,
    ack_sequence: AtomicU64,
    ack: [AtomicU64; 10],
    progress_sequence: AtomicU64,
    progress: [AtomicU64; 6],
    prepare_state: AtomicU64,
    now_playing_state: AtomicU64,
    play_sequence: [AtomicU64; 2],
    play: [[AtomicU64; 5]; 2],
    active: AtomicU64,
    incomplete: AtomicBool,
    dropped: AtomicU64,
    wall_origin: u64,
    clock_origin: std::time::Instant,
}

#[derive(Clone)]
pub(crate) struct Handle(Arc<Shared>);
pub(super) struct Endpoint { pub handle: Handle, receiver: Receiver<Request> }
impl Endpoint {
    pub fn new(observations: Receiver<super::Observation>) -> Self {
        let (sender, receiver) = crossbeam_channel::bounded(1);
        let clock_origin = std::time::Instant::now();
        let wall_origin = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .ok().and_then(|d| u64::try_from(d.as_nanos()).ok()).unwrap_or(0);
        let handle = Handle(Arc::new(Shared { sender, observations: std::sync::Mutex::new(Some(observations)), next: AtomicU64::new(1), sessions: AtomicU64::new(1), pending: AtomicU64::new(0),
            connected: AtomicBool::new(true), measurement_available: AtomicBool::new(true), ack_sequence: AtomicU64::new(0),
            ack: std::array::from_fn(|_| AtomicU64::new(0)), progress_sequence: AtomicU64::new(0),
            progress: std::array::from_fn(|_| AtomicU64::new(0)),
            prepare_state: AtomicU64::new(0), now_playing_state: AtomicU64::new(0), play_sequence: std::array::from_fn(|_| AtomicU64::new(0)),
            play: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))), active: AtomicU64::new(0),
            incomplete: AtomicBool::new(false), dropped: AtomicU64::new(0), wall_origin, clock_origin }));
        Self { handle, receiver }
    }
    pub fn request(&self) -> Option<Request> { self.receiver.try_recv().ok() }
    pub fn clock(&self) -> Option<u64> { self.handle.clock() }
    pub fn monitor_state(&self) -> [u64; 2] { [self.handle.0.prepare_state.load(Ordering::Acquire), self.handle.0.now_playing_state.load(Ordering::Acquire)] }
    pub fn record_play(&self, value: DigitalPlay) {
        let deck = usize::from(value.deck);
        let shared = &self.handle.0;
        shared.play_sequence[deck].fetch_add(1, Ordering::AcqRel);
        for (slot, word) in shared.play[deck].iter().zip([value.load, value.wall_ns, value.first_frame, u64::from(value.frames), u64::from(value.rate)]) {
            slot.store(word, Ordering::Relaxed);
        }
        shared.play_sequence[deck].fetch_add(1, Ordering::Release);
    }
    pub fn measurement_available(&self, available: bool) { self.handle.0.measurement_available.store(available, Ordering::Release); }
    pub fn publish_status(&self, session: u64, incomplete: bool, dropped: u64) {
        self.handle.0.active.store(session, Ordering::Release);
        self.handle.0.incomplete.store(incomplete, Ordering::Release);
        self.handle.0.dropped.store(dropped, Ordering::Release);
    }
    pub fn progress(&self, value: Progress) {
        let shared = &self.handle.0;
        shared.progress_sequence.fetch_add(1, Ordering::AcqRel);
        for (slot, value) in shared.progress.iter().zip([value.session, value.wall_ns, value.frame,
            u64::from(value.rate), u64::from(value.incomplete), value.dropped]) { slot.store(value, Ordering::Relaxed); }
        shared.progress_sequence.fetch_add(1, Ordering::Release);
    }
    pub fn acknowledge(&self, ack: Ack) {
        let shared = &self.handle.0;
        shared.ack_sequence.fetch_add(1, Ordering::AcqRel);
        for (slot, value) in shared.ack.iter().zip([ack.request, ack.session, ack.outcome.code(), ack.wall_ns,
            ack.frame, u64::from(ack.rate), u64::from(ack.incomplete), ack.dropped, ack.current[0], ack.current[1]]) { slot.store(value, Ordering::Relaxed); }
        shared.ack_sequence.fetch_add(1, Ordering::Release);
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) { self.handle.0.connected.store(false, Ordering::Release); }
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Progress {
    pub session: u64, pub wall_ns: u64, pub frame: u64, pub rate: u32,
    pub incomplete: bool, pub dropped: u64,
}
/// One complete window of confirmed digital main output from a playing deck.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DigitalPlay {
    pub deck: u8, pub load: u64, pub wall_ns: u64, pub first_frame: u64, pub frames: u32, pub rate: u32,
}
impl Handle {
    /// Check whether the renderer can attribute its current output to playing decks.
    /// Takes no arguments; returns false for unmeasured routes or a disconnected renderer.
    pub fn can_measure(&self) -> bool {
        self.0.connected.load(Ordering::Acquire) && self.0.measurement_available.load(Ordering::Acquire)
    }
    /// Enable transient prepare-queue measurement.
    /// Takes the requested state; returns an error if its bounded revision is exhausted.
    pub fn set_prepare_monitor(&self, enabled: bool) -> Result<(), &'static str> {
        self.0.prepare_state.fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
            if state & 1 == u64::from(enabled) { Some(state) }
            else { (state >> 1).checked_add(1).filter(|next| *next <= u64::MAX >> 1).map(|next| next << 1 | u64::from(enabled)) }
        }).map(|_| ()).map_err(|_| "Prepare measurement revision exhausted")
    }
    /// Enable the independent optional now-playing measurement owner.
    /// Takes enabled intent; returns a refusal if its revision is exhausted, without changing prepare-queue intent.
    pub fn set_now_playing_monitor(&self, enabled: bool) -> Result<(), &'static str> {
        self.0.now_playing_state.fetch_update(Ordering::AcqRel, Ordering::Acquire, |state| {
            if state & 1 == u64::from(enabled) { Some(state) }
            else { (state >> 1).checked_add(1).filter(|next|*next <= u64::MAX >> 1).map(|next|next << 1 | u64::from(enabled)) }
        }).map(|_|()).map_err(|_|"Now-playing measurement revision exhausted")
    }
    /// Capture the monotonic civil-time boundary used by output observations.
    /// Takes no arguments; returns None when the renderer clock is unavailable.
    pub fn clock(&self) -> Option<u64> {
        (self.0.wall_origin != 0).then_some(())?;
        self.0.wall_origin.checked_add(u64::try_from(self.0.clock_origin.elapsed().as_nanos()).ok()?)
    }
    /// Read the last fully classified playing window for a deck.
    /// Takes a deck index; returns a coherent observation or None during publication/disconnection.
    pub fn digital_play(&self, deck: usize) -> Option<DigitalPlay> {
        if deck >= 2 || !self.can_measure() { return None; }
        let sequence = self.0.play_sequence[deck].load(Ordering::Acquire);
        if sequence & 1 != 0 { return None; }
        let words: [u64; 5] = std::array::from_fn(|i| self.0.play[deck][i].load(Ordering::Relaxed));
        std::sync::atomic::fence(Ordering::Acquire);
        if !self.can_measure() || sequence != self.0.play_sequence[deck].load(Ordering::Relaxed) || words[0] == 0 || words[1] == 0 { return None; }
        Some(DigitalPlay { deck: deck as u8, load: words[0], wall_ns: words[1], first_frame: words[2], frames: words[3] as u32, rate: words[4] as u32 })
    }
    /// Read before draining observation events. This fences the durable prefix
    /// at complete classification windows, excluding the pending partial one.
    pub fn progress(&self) -> Option<Progress> {
        let sequence = self.0.progress_sequence.load(Ordering::Acquire);
        if sequence & 1 != 0 { return None; }
        let words: [u64; 6] = std::array::from_fn(|i| self.0.progress[i].load(Ordering::Relaxed));
        std::sync::atomic::fence(Ordering::Acquire);
        if sequence != self.0.progress_sequence.load(Ordering::Relaxed) || words[0] == 0 { return None; }
        Some(Progress { session: words[0], wall_ns: words[1], frame: words[2], rate: words[3] as u32,
            incomplete: words[4] != 0, dropped: words[5] })
    }

    pub fn take_observations(&self) -> Option<Receiver<super::Observation>> {
        self.0.observations.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
    pub fn new_session(&self) -> Result<u64, &'static str> {
        self.0.sessions.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1)).map_err(|_| "history session identities exhausted")
    }
    pub fn submit(&self, action: Action) -> Result<u64, &'static str> {
        if !self.0.connected.load(Ordering::Acquire) { return Err("history renderer unavailable"); }
        if matches!(action, Action::Start(0) | Action::End(0)) { return Err("invalid history session identity"); }
        let id = self.0.next.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| "history request identities exhausted")?;
        self.0.pending.compare_exchange(0, id, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "a history operation is still awaiting its renderer receipt")?;
        if self.0.sender.try_send(Request { id, action }).is_err() {
            self.0.pending.store(0, Ordering::Release);
            return Err("history renderer request unavailable");
        }
        Ok(id)
    }
    /// One bounded coherent read. The caller consumes the exact terminal
    /// receipt before another request may overwrite it, even after rejection.
    pub fn poll(&self, request: u64) -> Result<Option<Ack>, &'static str> {
        if self.0.pending.load(Ordering::Acquire) != request { return Err("history request is no longer pending"); }
        let sequence = self.0.ack_sequence.load(Ordering::Acquire);
        if sequence & 1 != 0 { return Ok(None); }
        let words: [u64; 10] = std::array::from_fn(|i| self.0.ack[i].load(Ordering::Relaxed));
        std::sync::atomic::fence(Ordering::Acquire);
        if sequence != self.0.ack_sequence.load(Ordering::Relaxed) { return Ok(None); }
        if words[0] != request {
            return if self.0.connected.load(Ordering::Acquire) { Ok(None) }
                else { Err("history renderer disconnected before confirmation; outcome unknown") };
        }
        let ack = Ack { request, session: words[1], outcome: Outcome::from_code(words[2]),
            wall_ns: words[3], frame: words[4], rate: words[5] as u32,
            incomplete: words[6] != 0, dropped: words[7], current: [words[8], words[9]] };
        self.0.pending.compare_exchange(request, 0, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "history receipt was already consumed")?;
        Ok(Some(ack))
    }
    /// Live diagnostics may be one callback apart; the terminal receipt is
    /// coherent and authoritative for final session coverage.
    pub fn status(&self) -> (u64, bool, u64, bool) {
        (self.0.active.load(Ordering::Acquire), self.0.incomplete.load(Ordering::Acquire),
            self.0.dropped.load(Ordering::Acquire), self.0.connected.load(Ordering::Acquire))
    }
}
