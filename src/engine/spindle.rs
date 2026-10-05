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
    position: f64,
    elapsed: f64,
    ratio: f64,
    velocity: f64,
    smoothing: (f32, f64),
}

impl Playback {
    /// Attach a physical spindle to a source position.
    /// Takes its input owner, measured motion and source seconds; returns a playback clock.
    pub fn new(source: u64, motion: Motion, seconds: f64) -> Self {
        Self { source, motion, position: seconds, elapsed: 0.0, ratio: 1.0, velocity: f64::from(motion.rate), smoothing: (0.0, 0.0) }
    }

    /// Replace measured motion without jumping the audio position.
    /// Takes measured motion; returns no value.
    pub fn update(&mut self, motion: Motion) {
        self.motion = motion;
    }

    /// Align a seek with the current physical position.
    /// Takes source seconds; returns no value.
    pub fn rebase(&mut self, seconds: f64) { self.position = seconds; }

    /// Change audio tempo while retaining physical platter speed.
    /// Takes a tempo ratio; returns no value.
    pub fn tempo(&mut self, ratio: f32) { self.ratio = f64::from(ratio); }

    /// Capture input age once at an audio block boundary.
    /// Takes the monotonic block time; returns no value.
    pub fn begin(&mut self, now: Instant) {
        self.elapsed = now.saturating_duration_since(self.motion.at).as_secs_f64();
    }

    /// Advance the physical clock without reading a wall clock per sample.
    /// Takes output sample rate; returns source seconds and current signed rate.
    pub fn next(&mut self, sr: f32) -> (f64, f32) {
        self.elapsed += 1.0 / f64::from(sr);
        if self.smoothing.0 != sr { self.smoothing = (sr, 1.0 - (-1.0 / (f64::from(sr) * 0.001)).exp()); }
        self.velocity += (f64::from(self.rate()) - self.velocity) * self.smoothing.1;
        if self.velocity.abs() < 0.00001 { self.velocity = 0.0; }
        self.position += self.velocity / f64::from(sr);
        (self.position, self.velocity as f32)
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
        for _ in 0..600 { s.next(48000.0); }
        let (stopped, rate) = s.next(48000.0);
        assert_eq!(rate, 0.0); assert!((stopped - 10.001).abs() < 0.00002);
        s.update(Motion { ticks: 1800, rate: -1.0, at: at + Duration::from_millis(20), ..motion });
        s.begin(at + Duration::from_millis(20));
        for _ in 0..100 { s.next(48000.0); }
        let (reverse, rate) = s.next(48000.0);
        assert!(rate < -0.85); assert!(reverse < stopped);
        assert!((s.turns() - 0.5).abs() < 0.002);
    }

    #[test]
    fn usb_arrival_jitter_cannot_splice_the_audio_waveform() {
        let at = Instant::now();
        let motion = Motion { ticks: 0, rate: 1.0, at, hold: 0.008 };
        let mut spindle = Playback::new(42, motion, 10.0);
        let mut error = 0.0;
        for frame in 0..48000 {
            if frame % 240 == 0 {
                let ticks = frame / 24;
                let jitter = [0, 800, 100, 600][frame / 240 % 4];
                spindle.update(Motion { ticks: ticks as i64, at: at + Duration::from_micros(frame as u64 * 1_000_000 / 48000 + jitter), ..motion });
                spindle.begin(at + Duration::from_micros(frame as u64 * 1_000_000 / 48000 + 1000));
            }
            let (position, rate) = spindle.next(48000.0);
            let reference = 10.0 + (frame + 1) as f64 / 48000.0;
            let phase = std::f64::consts::TAU * 1000.0;
            error += ((position * phase).sin() - (reference * phase).sin()).powi(2);
            assert_eq!(rate, 1.0);
            assert!((position - reference).abs() < 1e-9);
        }
        assert!(error / 48000.0 < 1e-12, "steady 1 kHz tone must survive input jitter: {error}");
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
        for _ in 0..1200 { rt.render_deck(0); }
        assert!(rt.decks[0].playing);
        assert_eq!(rt.decks[0].rate, 0.0);
        assert!(rt.decks[0].pos > 48400.0 && rt.decks[0].pos < 48440.0);
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
