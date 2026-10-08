use super::{Event, Message, Shared};
use std::{
    sync::{atomic::Ordering::*, Arc},
    time::Instant,
};

pub(crate) struct Runtime {
    shared: Arc<Shared>,
    generation: u64,
    transport: u64,
    active: bool,
    expected: Option<f64>,
    next_tick: u64,
    base_ns: u64,
    rate: u32,
}
impl Runtime {
    /// Prepare a clock producer without constructing an output device.
    /// Takes its shared bounded queue; returns an inactive audio-owned scheduler.
    pub(crate) fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            generation: 0,
            transport: 0,
            active: false,
            expected: None,
            next_tick: 0,
            base_ns: 0,
            rate: 0,
        }
    }
    /// Anchor clocks to the first output sample of a callback.
    /// Takes sample rate, frame count and reported first-frame playback time; returns after publishing timing quality and a callback watchdog deadline.
    pub(crate) fn begin(&mut self, rate: u32, frames: usize, first: Option<Instant>) {
        let now = self.shared.now_ns();
        let scheduled = first
            .and_then(|instant| instant.checked_duration_since(self.shared.origin))
            .map(super::ns);
        let estimate = now
            .saturating_add((frames as u64).saturating_mul(1_000_000_000) / u64::from(rate.max(1)));
        self.begin_at(
            rate,
            frames,
            scheduled.unwrap_or(estimate),
            scheduled.is_some(),
        );
    }
    /// Read clock state without waiting for its output owner.
    /// Takes this renderer scheduler; returns its bounded diagnostic counters.
    pub(crate) fn counters(&self) -> super::Counters {
        self.shared.counters()
    }
    /// Anchor one block to a reviewed monotonic output timestamp.
    /// Takes rate, frame count, first-frame timestamp and backend timing availability; returns without allocation or locks.
    pub(crate) fn begin_at(&mut self, rate: u32, frames: usize, base_ns: u64, known: bool) {
        self.rate = rate;
        self.base_ns = base_ns;
        self.shared.backend_timing.store(known, Release);
        let end = base_ns
            .saturating_add((frames as u64).saturating_mul(1_000_000_000) / u64::from(rate.max(1)));
        self.shared.callback_until.store(end, Release);
    }
    fn deadline(&self, offset: usize) -> u64 {
        let time = self.base_ns.saturating_add(
            (offset as u64).saturating_mul(1_000_000_000) / u64::from(self.rate.max(1)),
        );
        let correction = i64::from(self.shared.compensation_ms.load(Acquire)) * 1_000_000;
        time.saturating_add_signed(correction)
    }
    fn emit(&self, message: Message, offset: usize) -> bool {
        let event = Event {
            message,
            deadline_ns: self.deadline(offset),
            generation: self.generation,
        };
        if self.shared.events.try_send(event).is_err() {
            self.shared.overflow.fetch_add(1, Relaxed);
            self.shared.fail(super::Fault::QueueFull);
            return false;
        }
        if message == Message::Clock {
            self.shared.scheduled.fetch_add(1, Relaxed);
        }
        true
    }
    fn start(&mut self, beat: f64, offset: usize) -> bool {
        if !beat.is_finite() || !(0.0..=4095.75).contains(&beat) {
            self.shared.fail(super::Fault::Position);
            return false;
        }
        let position = (beat * 4.0).floor() as u16;
        if beat <= 1e-10 {
            if !self.emit(Message::Start, offset) {
                return false;
            }
        } else {
            if !self.emit(Message::Position(position), offset)
                || !self.emit(Message::Continue, offset)
            {
                return false;
            }
        }
        self.next_tick = (beat * 24.0 - 1e-9).ceil().max(0.0) as u64;
        self.shared
            .position_sixteenths
            .store(u64::from(position), Release);
        true
    }
    /// Schedule MIDI clock and transport at an audio sample boundary.
    /// Takes pre/post sample song beats, running state, transport epoch and sample offset; returns after bounded queue submission without heap work.
    pub(crate) fn frame(
        &mut self,
        beat: f64,
        next_beat: f64,
        running: bool,
        transport: u64,
        offset: usize,
    ) {
        let generation = self.shared.generation.load(Acquire);
        if !self.shared.enabled.load(Acquire) || self.rate == 0 {
            self.active = false;
            self.expected = None;
            self.generation = generation;
            return;
        }
        if self.generation != generation {
            self.active = false;
            self.expected = None;
            self.generation = generation;
        }
        let moved = self
            .expected
            .is_some_and(|expected| (beat - expected).abs() > 1e-8);
        if self.active && (!running || transport != self.transport || moved) {
            if !self.emit(Message::Stop, offset) {
                return;
            }
            self.active = false;
        }
        self.transport = transport;
        if running && !self.active {
            if !self.start(beat, offset) {
                return;
            }
            self.active = true;
        }
        if self.active {
            if !beat.is_finite()
                || !next_beat.is_finite()
                || next_beat < beat
                || next_beat - beat > 1.0 / 24.0
            {
                self.shared.fail(super::Fault::Timing);
                self.active = false;
                return;
            }
            let current = beat * 24.0;
            if current + 1e-9 >= self.next_tick as f64 {
                if !self.emit(Message::Clock, offset) {
                    self.active = false;
                    return;
                }
                self.next_tick = self.next_tick.saturating_add(1);
            }
        }
        self.shared.running.store(self.active, Release);
        self.expected = Some(next_beat);
    }
}
