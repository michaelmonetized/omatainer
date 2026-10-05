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
    samples: [Option<(Instant, i64)>; 32],
    cursor: usize,
    direction: i16,
    rate: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_jitter_does_not_modulate_a_freely_rotating_record() {
        let at = Instant::now();
        let mut spindle = Spindle::default();
        let (cmd, received) = CommandPort::channel(256);
        let mut maximum = 0.0_f32;
        for i in 0..200_u64 {
            let jitter = [0, 400, 100, 300][i as usize % 4];
            let time = at + Duration::from_micros(i * 1000 + jitter);
            spindle.position((i * 2 % 128) as u8, time);
            spindle.flush(42, 0, time, &cmd);
            for command in received.try_iter() {
                if let Command::DeckSpindle { motion, .. } = command {
                    if i >= 32 { maximum = maximum.max((motion.rate - 1.0).abs()); }
                }
            }
        }
        assert!(maximum < 0.01, "free rotation varied by {maximum}");
        spindle.position(((398_u64 - 2) % 128) as u8, at + Duration::from_millis(201));
        assert!(spindle.rate < 0.0, "reverse movement must discard the forward estimate");
    }
}

impl Spindle {
    /// Accumulate the NS7's wrapping encoder counter.
    /// Takes a seven-bit counter and callback time; returns no value.
    pub fn position(&mut self, value: u8, at: Instant) {
        if let Some(previous) = self.previous.replace(value) {
            let delta = (i16::from(value) - i16::from(previous) + 64).rem_euclid(128) - 64;
            if delta == 0 { return; }
            if self.direction != delta.signum() {
                self.samples.fill(None);
                self.samples[0] = self.at.map(|at| (at, self.ticks));
                self.cursor = 1;
                self.direction = delta.signum();
            }
            self.ticks += i64::from(delta);
            self.rate = self.at.map_or(0.0, |previous| {
                (f64::from(delta) / at.saturating_duration_since(previous).as_secs_f64().max(0.0001) / crate::engine::spindle::TICKS_PER_SECOND).clamp(-16.0, 16.0) as f32
            });
        }
        if let Some(previous) = self.at.replace(at) { self.interval = at.saturating_duration_since(previous).as_secs_f64(); }
        self.samples[self.cursor] = Some((at, self.ticks));
        self.cursor = (self.cursor + 1) % self.samples.len();
        let (mut n, mut x, mut y, mut xx, mut xy) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for &(time, ticks) in self.samples.iter().flatten() {
            let age = at.saturating_duration_since(time).as_secs_f64();
            if age > 0.032 { continue; }
            let offset = (ticks - self.ticks) as f64;
            n += 1.0; x += age; y += offset; xx += age * age; xy += age * offset;
        }
        let variance = n * xx - x * x;
        if variance > 1e-12 {
            self.rate = ((x * y - n * xy) / variance / crate::engine::spindle::TICKS_PER_SECOND).clamp(-16.0, 16.0) as f32;
        }
    }

    /// Hand accumulated movement to the renderer at a bounded rate.
    /// Takes input owner, deck, current time and command port; returns no value.
    pub fn flush(&mut self, source: u64, deck: u8, now: Instant, cmd: &CommandPort) {
        let Some(at) = self.at else { return; };
        if let Some(sent_at) = self.sent_at {
            if self.ticks == self.sent || now.saturating_duration_since(sent_at) < Duration::from_millis(4) { return; }
        }
        let motion = Motion { ticks: self.ticks, rate: self.rate, at, hold: (self.interval * 2.0).clamp(0.012, 0.05) };
        if cmd.send(Command::DeckSpindle { source, deck, motion }).is_ok() {
            self.sent = self.ticks; self.sent_at = Some(at);
        }
    }
}
