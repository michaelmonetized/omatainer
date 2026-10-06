use super::{
    super::{DeckRt, DeckTransition, RtEngine},
    Control,
};

pub(super) mod media_key {
    use serde::{Deserialize, Deserializer, Serializer};
    /// Serialize an exact media identity.
    /// Takes the source key and JSON serializer; returns a decimal string without floating-point loss.
    pub fn serialize<S: Serializer>(key: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&key.to_string())
    }
    /// Read an exact media identity.
    /// Takes the producer-side deserializer; returns the canonical unsigned decimal key or refuses malformed values.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        let text = String::deserialize(deserializer)?;
        let key = text.parse::<u64>().map_err(serde::de::Error::custom)?;
        if text != key.to_string() {
            return Err(serde::de::Error::custom("Use the exact decimal media key"));
        }
        Ok(key)
    }
}

impl DeckRt {
    /// Resolve a complete loop edit against its original media.
    /// Takes source-time bounds or a musical length/move and fallback clock; returns valid source-frame endpoints without mutation.
    pub(in crate::engine) fn loop_edit_bounds(
        &self,
        control: Control,
        sr: f32,
        bpm: f32,
    ) -> Option<(f64, f64)> {
        let audio = self.audio.as_ref()?;
        let frames = audio.frames() as f64;
        if frames < 64.0 || self.controls.saved_loop.is_some() {
            return None;
        }
        let (key, start, end) = match control {
            Control::LoopBounds {
                media_key,
                start_seconds,
                end_seconds,
            } => (
                media_key,
                start_seconds * f64::from(audio.sr),
                end_seconds * f64::from(audio.sr),
            ),
            Control::LoopMove { media_key, beats } => {
                if self.loop_len < 64.0
                    || self.loop_start < 0.0
                    || self.loop_start + self.loop_len > frames
                {
                    return None;
                }
                let span = self.grid_beats_between(
                    self.loop_start,
                    self.loop_start + self.loop_len,
                    sr,
                    bpm,
                );
                let start = self.grid_beat_at(self.loop_start, sr, bpm) + beats;
                let (start, end) = self.fit_loop_beats(start, span, frames, sr, bpm)?;
                (media_key, start, end)
            }
            Control::LoopLength { media_key, beats } => {
                let start = if self.loop_len >= 64.0 {
                    self.loop_start
                } else {
                    self.pos
                };
                let (start, end) =
                    self.fit_loop_beats(self.grid_beat_at(start, sr, bpm), beats, frames, sr, bpm)?;
                (media_key, start, end)
            }
            _ => return None,
        };
        (key != 0
            && key == self.history_key
            && control.valid()
            && start.is_finite()
            && end.is_finite()
            && start >= 0.0
            && end <= frames
            && end - start >= 64.0)
            .then_some((start, end))
    }

    /// Fit a complete musical loop inside its source.
    /// Takes desired start beat, beat span, source extent and fallback clock; shifts the entire region at track boundaries or refuses a length that cannot fit.
    fn fit_loop_beats(
        &self,
        start: f64,
        span: f64,
        frames: f64,
        sr: f32,
        bpm: f32,
    ) -> Option<(f64, f64)> {
        let first = self.grid_beat_at(0.0, sr, bpm);
        let last = self.grid_beat_at(frames, sr, bpm);
        if !start.is_finite() || !span.is_finite() || span <= 0.0 || span > last - first {
            return None;
        }
        let beat = start.clamp(first, last - span);
        let start = self.grid_position_at(beat, sr, bpm).clamp(0.0, frames);
        let end = self
            .grid_position_at(beat + span, sr, bpm)
            .clamp(0.0, frames);
        Some((start, end))
    }

    /// Apply one validated loop without changing transport ownership.
    /// Takes endpoints, musical-position preservation and the fallback clock; preserves source cues and transitions the audible playhead through existing DSP reset/blend paths.
    fn set_loop_bounds(&mut self, start: f64, end: f64, musical: bool, sr: f32, bpm: f32) {
        let position = if self.loop_on {
            if musical && self.pos >= self.loop_start && self.pos < self.loop_start + self.loop_len
            {
                let phase = self.grid_beats_between(self.loop_start, self.pos, sr, bpm);
                self.grid_position_at(self.grid_beat_at(start, sr, bpm) + phase, sr, bpm)
            } else {
                self.pos
            }
        } else {
            self.pos
        };
        self.loop_start = start;
        self.loop_len = end - start;
        let position = if self.loop_on {
            start + (position - start).rem_euclid(self.loop_len)
        } else {
            position
        };
        self.transition_to(position, sr, DeckTransition::Jump);
        self.publish_preparation();
    }
}

impl RtEngine {
    /// Apply precise source-qualified loop work.
    /// Takes exact deck and validated control; updates the selected controller bank and durable preparation owner.
    pub(super) fn edit_deck_loop(&mut self, deck: usize, control: Control) {
        let Some((start, end)) = self.decks[deck].loop_edit_bounds(control, self.sr, self.bpm)
        else {
            return;
        };
        let d = &mut self.decks[deck];
        if (d.loop_start, d.loop_start + d.loop_len) == (start, end) {
            return;
        }
        d.set_loop_bounds(
            start,
            end,
            matches!(control, Control::LoopMove { .. }),
            self.sr,
            self.bpm,
        );
        self.remember_controller_loop(deck);
        self.project.edited();
    }
}
