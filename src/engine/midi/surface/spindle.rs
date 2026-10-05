use crate::engine::{spindle::Motion, Command, CommandPort};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Spindle {
    previous: Option<u8>,
    ticks: i64,
    sent: i64,
    sent_at: Option<Instant>,
    at: Option<Instant>,
    interval: f64,
}

impl Spindle {
    /// Accumulate the NS7's wrapping encoder counter.
    /// Takes a seven-bit counter and callback time; returns no value.
    pub fn position(&mut self, value: u8, at: Instant) {
        if let Some(previous) = self.previous.replace(value) {
            self.ticks += i64::from((i16::from(value) - i16::from(previous) + 64).rem_euclid(128) - 64);
        }
        if let Some(previous) = self.at.replace(at) { self.interval = at.saturating_duration_since(previous).as_secs_f64(); }
    }

    /// Hand accumulated movement to the renderer at a bounded rate.
    /// Takes input owner, deck, current time and command port; returns no value.
    pub fn flush(&mut self, source: u64, deck: u8, now: Instant, cmd: &CommandPort) {
        let Some(at) = self.at else { return; };
        if let Some(sent_at) = self.sent_at {
            if self.ticks == self.sent || now.saturating_duration_since(sent_at) < Duration::from_millis(4) { return; }
        }
        let rate = self.sent_at.map_or(0.0, |previous| {
            ((self.ticks - self.sent) as f64 / at.saturating_duration_since(previous).as_secs_f64().max(0.0001) / crate::engine::spindle::TICKS_PER_SECOND).clamp(-16.0, 16.0) as f32
        });
        let motion = Motion { ticks: self.ticks, rate, at, hold: (self.interval * 2.0).clamp(0.008, 0.05) };
        if cmd.send(Command::DeckSpindle { source, deck, motion }).is_ok() {
            self.sent = self.ticks; self.sent_at = Some(at);
        }
    }
}
