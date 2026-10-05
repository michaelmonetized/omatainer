use std::time::Instant;

pub(crate) const TICKS_PER_TURN: f64 = 3600.0;
pub(crate) const TICKS_PER_SECOND: f64 = 2000.0;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Motion {
    pub ticks: i64,
    pub rate: f32,
    pub at: Instant,
    pub hold: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct Playback {
    pub source: u64,
    motion: Motion,
    anchor: f64,
    elapsed: f64,
    ratio: f64,
}

impl Playback {
    /// Attach a physical spindle to a source position.
    /// Takes its input owner, measured motion and source seconds; returns a playback clock.
    pub fn new(source: u64, motion: Motion, seconds: f64) -> Self {
        Self { source, motion, anchor: seconds, elapsed: 0.0, ratio: 1.0 }
    }

    /// Retain every encoder tick while replacing the latest velocity.
    /// Takes measured motion; returns no value.
    pub fn update(&mut self, motion: Motion) {
        self.anchor += (motion.ticks - self.motion.ticks) as f64 / TICKS_PER_SECOND * self.ratio;
        self.motion = motion;
    }

    /// Align a seek with the current physical position.
    /// Takes source seconds; returns no value.
    pub fn rebase(&mut self, seconds: f64) { self.anchor = seconds; self.elapsed = 0.0; }

    /// Change audio tempo while retaining physical platter speed.
    /// Takes a tempo ratio and current source seconds; returns no value.
    pub fn tempo(&mut self, ratio: f32, seconds: f64) {
        if self.ratio != f64::from(ratio) { self.rebase(seconds); self.ratio = f64::from(ratio); }
    }

    /// Capture input age once at an audio block boundary.
    /// Takes the monotonic block time; returns no value.
    pub fn begin(&mut self, now: Instant) {
        self.elapsed = now.saturating_duration_since(self.motion.at).as_secs_f64();
    }

    /// Advance the physical clock without reading a wall clock per sample.
    /// Takes output sample rate; returns source seconds and current signed rate.
    pub fn next(&mut self, sr: f32) -> (f64, f32) {
        self.elapsed += 1.0 / f64::from(sr);
        (self.anchor + f64::from(self.motion.rate) * self.ratio * self.elapsed.min(self.motion.hold), self.rate())
    }

    /// Read the current signed audio rate.
    /// Takes this clock; returns zero when input movement has expired.
    pub fn rate(&self) -> f32 { if self.elapsed <= self.motion.hold { self.motion.rate * self.ratio as f32 } else { 0.0 } }

    /// Read physical turns per second independently of audio tempo.
    /// Takes this clock; returns signed rotation or zero for an expired report.
    pub fn turn_rate(&self) -> f32 { if self.elapsed <= self.motion.hold { self.motion.rate * (TICKS_PER_SECOND / TICKS_PER_TURN) as f32 } else { 0.0 } }

    /// Read the accumulated physical rotation.
    /// Takes this clock; returns signed turns with bounded prediction between reports.
    pub fn turns(&self) -> f64 {
        (self.motion.ticks as f64 + f64::from(self.motion.rate) * TICKS_PER_SECOND * self.elapsed.min(self.motion.hold)) / TICKS_PER_TURN
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Command, RtEngine, Snapshot};
    use std::sync::Arc;
    use parking_lot::Mutex;
    use std::time::Duration;

    #[test]
    fn encoder_clock_follows_forward_reverse_hold_and_one_full_turn() {
        let at = Instant::now();
        let motion = Motion { ticks: 0, rate: 1.0, at, hold: 0.008 };
        let mut s = Playback::new(42, motion, 10.0);
        s.update(Motion { ticks: 3600, ..motion });
        assert_eq!(s.turns(), 1.0);
        s.begin(at + Duration::from_millis(20));
        let (stopped, rate) = s.next(48000.0);
        assert_eq!(rate, 0.0); assert!((stopped - 11.808).abs() < 1e-8);
        s.update(Motion { ticks: 1800, rate: -1.0, at: at + Duration::from_millis(20), ..motion });
        s.begin(at + Duration::from_millis(20));
        let (reverse, rate) = s.next(48000.0);
        assert_eq!(rate, -1.0); assert!(reverse < 10.9);
        assert!((s.turns() - 0.5).abs() < 0.001);
    }

    #[test]
    fn renderer_follows_vinyl_and_a_held_record_is_silent_without_releasing_play() {
        let (_tx, rx) = crossbeam_channel::bounded(64);
        let mut rt = RtEngine::new(48000.0, rx, Arc::new(Mutex::new(Snapshot::default())));
        rt.decks[0].playing = true;
        rt.decks[0].pos = 48000.0;
        let at = Instant::now();
        let motion = Motion { ticks: 0, rate: 1.0, at, hold: 0.008 };
        rt.apply(Command::DeckSpindle { source: 42, deck: 0, motion });
        rt.apply(Command::DeckSpindle { source: 42, deck: 0, motion: Motion { ticks: 20, ..motion } });
        for _ in 0..500 { rt.render_deck(0); }
        assert!(rt.decks[0].playing);
        assert_eq!(rt.decks[0].rate, 0.0);
        assert!((rt.decks[0].pos - 48864.0).abs() < 0.1);
        for _ in 0..1000 { let (l,r) = rt.render_deck(0); assert!(l.is_finite() && r.is_finite()); }
        assert!(rt.render_deck(0).0.abs() < 0.0001);
        let held = rt.decks[0].pos;
        rt.apply(Command::DeckTouch { deck: 0, on: true });
        rt.apply(Command::DeckJog { deck: 0, delta: -0.02 });
        rt.render_deck(0);
        assert!(rt.decks[0].pos < held);
        rt.apply(Command::DeckTouch { deck: 0, on: false });
        rt.apply(Command::DeckSpindleRelease { source: 41, deck: 0 });
        assert!(rt.decks[0].spindle.is_some());
        rt.apply(Command::DeckSpindleRelease { source: 42, deck: 0 });
        assert!(rt.decks[0].spindle.is_none()); assert!(!rt.decks[0].playing);
    }
}
