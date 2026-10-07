use super::{
    super::{DeckRt, DeckTransition},
    Button, State, QUANTIZE_DIVISIONS,
};

impl State {
    /// Recognize gestures that share the background timeline.
    /// Takes a held button; returns whether a playing deck must retain its slip origin.
    pub(super) fn slip_button(button: Button) -> bool {
        matches!(
            button,
            Button::Reverse
                | Button::Bleep
                | Button::Cue
                | Button::HotCue(_)
                | Button::Roll(_)
                | Button::Slice(_)
        )
    }

    /// Read nested ownership without allocating or inspecting input devices.
    /// Takes this state; returns whether any temporary slip button is held.
    fn slip_held(&self) -> bool {
        self.reverse_latched || self.counts[..2]
            .iter()
            .chain(&self.counts[5..])
            .any(|count| *count != 0)
    }

    /// Cancel the background timeline at a deliberate transport boundary.
    /// Takes this state; clears shadow, pending return and its original loop.
    pub(in crate::engine) fn cancel_slip(&mut self) {
        self.slip_forward = None;
        self.slip_loop = None;
        self.slip_beat = 0.0;
        self.slip_due = None;
        self.slip_return = None;
    }

    /// Retire a deliberate interruption until the old gesture ends.
    /// Takes this state; clears the timeline and prevents a still-held input from starting it again.
    pub(in crate::engine) fn interrupt_slip(&mut self) {
        self.cancel_slip();
        self.slip_interrupted = true;
    }
}

impl DeckRt {
    /// Capture one source timeline before a temporary jump.
    /// Takes source/output clock information; starts only on an enabled playing source and preserves an existing nested root.
    pub(in crate::engine) fn begin_slip(&mut self, sr: f32, bpm: f32) {
        if !self.controls.slip || !self.playing || self.audio.is_none() {
            return;
        }
        self.controls.slip_interrupted = false;
        if self.controls.slip_forward.is_none() {
            self.controls.slip_forward = Some(self.pos);
            self.controls.slip_loop = Some(self.controls.saved_loop.unwrap_or((
                self.loop_on,
                self.loop_start,
                self.loop_len,
            )));
            self.controls.slip_beat = self.grid_beat_at(self.pos, sr, bpm);
        }
        self.controls.slip_due = None;
        self.controls.slip_return = None;
    }

    /// Translate a pending musical return into the original source loop.
    /// Takes remaining beats and clocks; returns the exact source-frame marker through variable tempo anchors and repeating bounds.
    fn slip_target(&self, remaining: f64, sr: f32, bpm: f32) -> f64 {
        let position = self.controls.slip_forward.unwrap_or(self.pos);
        let mut beat = self.grid_beat_at(position, sr, bpm) + remaining;
        if let Some((true, start, len)) = self.controls.slip_loop.filter(|(_, _, len)| *len > 1.0) {
            let first = self.grid_beat_at(start, sr, bpm);
            let last = self.grid_beat_at(start + len, sr, bpm);
            if beat >= last - 1e-9 && last > first {
                let span = last - first;
                let phase = (beat - first).rem_euclid(span);
                beat = first
                    + if phase < 1e-9 || span - phase < 1e-9 {
                        0.0
                    } else {
                        phase
                    };
            }
        }
        self.grid_position_at(beat, sr, bpm).clamp(
            0.0,
            self.audio
                .as_ref()
                .map_or(0.0, |audio| audio.frames() as f64),
        )
    }

    /// Advance an independent source-owned slip clock.
    /// Takes one output frame's nominal forward source step and clock; returns to its predicted timeline after the final nested gesture and chosen release boundary.
    pub(in crate::engine) fn tick_slip(&mut self, step: f64, sr: f32, bpm: f32) {
        let scratching = self.touching
            || self.follows_spindle()
                && self
                    .spindle
                    .as_ref()
                    .is_some_and(super::super::spindle::Playback::scratching);
        let held = scratching || self.controls.slip_held();
        if self.controls.slip_interrupted {
            if held {
                return;
            }
            self.controls.slip_interrupted = false;
        }
        if !self.controls.slip || !self.playing || self.audio.is_none() {
            self.controls.cancel_slip();
            return;
        }
        if held {
            self.begin_slip(sr, bpm);
        }
        let Some(position) = self.controls.slip_forward else {
            return;
        };
        if !held {
            let due = if let Some(index) = self.controls.slip_release {
                let division = QUANTIZE_DIVISIONS[usize::from(index)];
                {
                    if self.controls.slip_due.is_none() {
                        self.controls.slip_due =
                            Some(((self.controls.slip_beat - 1e-9) / division).ceil() * division);
                    }
                    self.controls.slip_due.unwrap()
                }
            } else {
                self.controls.slip_beat
            };
            let remaining = (due - self.controls.slip_beat).max(0.0);
            self.controls.slip_return = Some(self.slip_target(remaining, sr, bpm));
            if remaining <= 1e-9 {
                let position = self.slip_target(0.0, sr, bpm);
                if let Some((on, start, len)) = self.controls.slip_loop {
                    self.loop_on = on;
                    self.loop_start = start;
                    self.loop_len = len;
                }
                self.controls.forward = None;
                self.controls.performance_forward = None;
                self.controls.saved_loop = None;
                self.controls.cancel_slip();
                let source_sr = self
                    .audio
                    .as_ref()
                    .map_or(f64::from(sr), |audio| f64::from(audio.sr));
                let tempo = self
                    .grid
                    .as_ref()
                    .and_then(|grid| grid.bpm_at(position / source_sr))
                    .map_or_else(
                        || self.audio.as_ref().map_or(self.bpm, |audio| audio.bpm),
                        |tempo| tempo as f32,
                    );
                self.rate = if self.sync {
                    self.sync_bpm / tempo.max(1.0)
                } else {
                    self.pitch_rate()
                };
                self.target_rate = self.rate;
                self.transition_to(position, sr, DeckTransition::Jump);
                return;
            }
        }
        let mut next = if self.sync {
            self.grid_position_at(
                self.grid_beat_at(position, sr, bpm)
                    + f64::from(self.sync_bpm) / (60.0 * f64::from(sr)),
                sr,
                bpm,
            )
        } else {
            position + step
        };
        let mut beats = self.grid_beats_between(position, next, sr, bpm);
        if let Some((true, start, len)) = self.controls.slip_loop.filter(|(_, _, len)| *len > 1.0) {
            let end = start + len;
            if next >= end {
                let excess = next - end;
                let cycles = (excess / len).floor();
                next = start + excess.rem_euclid(len);
                beats = self.grid_beats_between(position, end, sr, bpm)
                    + cycles * self.grid_beats_between(start, end, sr, bpm)
                    + self.grid_beats_between(start, next, sr, bpm);
            }
        }
        self.controls.slip_forward = Some(next);
        self.controls.slip_beat += beats.max(0.0);
    }
}

#[cfg(test)]
mod tests;
