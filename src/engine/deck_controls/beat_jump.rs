use super::{super::DeckRt, BEAT_JUMP_SIZES};

impl DeckRt {
    /// Move a source position by musical beats.
    /// Takes source frames, signed beats, optional loop bounds and fallback clock; returns a bounded destination.
    fn beat_jump_position(&self, position: f64, beats: f64, bounds: Option<(f64, f64)>, sr: f32, bpm: f32) -> Option<f64> {
        let frames = self.audio.as_ref()?.frames() as f64;
        if frames <= 0.0 || !position.is_finite() { return None; }
        let mut target = self.grid_beat_at(position, sr, bpm) + beats;
        if let Some((start, length)) = bounds {
            if start < 0.0 || length <= 1.0 || start + length > frames { return None; }
            let origin = self.grid_beat_at(start, sr, bpm);
            let span = self.grid_beats_between(start, start + length, sr, bpm);
            if !span.is_finite() || span <= 0.0 { return None; }
            target = origin + (target - origin).rem_euclid(span);
        }
        let target = self.grid_position_at(target, sr, bpm);
        target.is_finite().then_some(target.clamp(0.0, frames))
    }

    /// Jump without changing cue, loop, pitch or transport ownership.
    /// Takes direction and fallback clock; moves audible and held slip clocks through their respective musical loops.
    pub(super) fn beat_jump(&mut self, forward: bool, sr: f32, bpm: f32) {
        let beats = f64::from(BEAT_JUMP_SIZES[usize::from(self.controls.beat_jump_size)])
            * if forward { 1.0 } else { -1.0 };
        let bounds = self.loop_on.then_some((self.loop_start, self.loop_len));
        let Some(position) = self.beat_jump_position(self.pos, beats, bounds, sr, bpm) else { return; };
        let underlying = self.controls.saved_loop.map_or(bounds, |(on, start, length)| on.then_some((start, length)));
        let jump_clock = |clock: Option<f64>| match clock {
            Some(p) => self.beat_jump_position(p, beats, underlying, sr, bpm).map(Some), None => Some(None),
        };
        let (Some(forward_clock), Some(performance_clock), Some(slip_clock)) = (
            jump_clock(self.controls.forward), jump_clock(self.controls.performance_forward), jump_clock(self.controls.slip_forward)
        ) else { return; };
        self.controls.forward = forward_clock;
        self.controls.performance_forward = performance_clock;
        self.controls.slip_forward = slip_clock;
        if position != self.pos { self.transition_to(position, sr, super::super::DeckTransition::Jump); }
    }
}
