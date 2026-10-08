use super::{master_fx::MasterSlot, FxKind};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Kind {
    #[default]
    Filter,
    Echo,
    Room,
}
impl Kind {
    pub const ALL: [Self; 3] = [Self::Filter, Self::Echo, Self::Room];
    /// Name the applied channel effect.
    /// Takes this effect; returns its native display and feedback name.
    pub fn name(self) -> &'static str {
        match self { Self::Filter => "Filter", Self::Echo => "Echo", Self::Room => "Room" }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::Filter => "Left removes highs; right removes lows.",
            Self::Echo => "Left repeats after ¼ beat; right after ¾ beat, capped at two seconds. Depth grows toward the edges.",
            Self::Room => "Left is a short room; right is a longer room. Depth grows toward the edges.",
        }
    }
    pub fn is_filter(&self) -> bool { *self == Self::Filter }
    /// Resolve a saved controller selection.
    /// Takes a stable effect ID; returns its effect or rejects an unknown ID.
    pub fn from_id(id: u16) -> Option<Self> { Self::ALL.get(usize::from(id)).copied() }
    fn index(self) -> usize { match self { Self::Filter => 0, Self::Echo => 1, Self::Room => 2 } }
}

pub(crate) struct Channel {
    processor: MasterSlot,
    weights: [f32; 3],
    sides: [i8; 2],
    input_gate: [f32; 2],
    kind: Kind,
    start: [f32; 3],
    remaining: u32,
    length: u32,
}
impl Channel {
    /// Prepare independent stereo channel histories.
    /// Takes output rate; returns bounded delay/reverb storage before the audio callback, or allocation failure.
    pub fn new(rate: f32) -> Result<Self, std::collections::TryReserveError> {
        Ok(Self { processor: MasterSlot::try_new(rate)?, weights: [1.0, 0.0, 0.0], sides: [0; 2], input_gate: [0.0; 2], kind: Kind::Filter, start: [1.0, 0.0, 0.0], remaining: 0, length: 1 })
    }
    /// Retire histories when the audio stream stops.
    /// Takes no input; resets fixed generations without allocating or clearing delay buffers.
    pub fn reset(&mut self) {
        self.processor.reset(FxKind::Echo);
        self.processor.reset(FxKind::Reverb);
        self.sides = [0; 2];
        self.input_gate = [0.0; 2];
    }
    fn wet(&mut self, input: [f32; 2], position: f32, index: usize, samples_per_beat: f64, rate: f32) -> [f32; 2] {
        let distance = ((position - 0.5).abs() - 0.03).max(0.0) / 0.47;
        let side = if distance <= f32::EPSILON { 0 } else if position < 0.5 { -1 } else { 1 };
        let kind = if index == 0 { FxKind::Echo } else { FxKind::Reverb };
        if self.sides[index] != side {
            self.processor.reset(kind);
            self.sides[index] = side;
            self.input_gate[index] = 0.0;
        }
        if side == 0 { return input; }
        self.input_gate[index] = (self.input_gate[index] + 1.0 / (rate * 0.005).ceil().max(1.0)).min(1.0);
        let distance = distance.min(1.0);
        let start = (distance * 5.0).min(1.0);
        let activation = start * start * (3.0 - 2.0 * start);
        let wet = 0.6 * distance * activation;
        for channel in 0..2 {
            if index == 0 {
                self.processor.echo[channel].fb = 0.25 + 0.3 * distance;
            } else {
                self.processor.reverb[channel].decay(if side < 0 { 0.15 + 0.25 * distance } else { 0.35 + 0.35 * distance });
            }
        }
        let beats = if side < 0 { 0.25 } else { 0.75 };
        let frames = (samples_per_beat * beats).clamp(1.0, f64::from(rate) * 2.0 - 2.0);
        self.processor.configure(1.0, frames / 0.75);
        let tail = self.processor.process(input.map(|value| value * self.input_gate[index]), kind, 1.0);
        std::array::from_fn(|channel| input[channel] * (1.0 - wet) + tail[channel] * wet)
    }
    /// Render the chosen one-knob effect without changing deck ownership.
    /// Takes dry/legacy-filtered stereo frames, effect, smoothed knob, musical clock and output rate; returns a convex five-millisecond type transition with an exact dry center.
    pub fn process(&mut self, input: [f32; 2], filtered: [f32; 2], kind: Kind, position: f32, samples_per_beat: f64, rate: f32) -> [f32; 2] {
        let selected = kind.index();
        if kind != self.kind {
            self.kind = kind;
            self.start = self.weights;
            self.length = (rate * 0.005).ceil().max(1.0) as u32;
            self.remaining = self.length;
        }
        if self.remaining > 0 {
            self.remaining -= 1;
            let fraction = self.remaining as f32 / self.length as f32;
            let mut other = 0.0;
            for index in 0..3 {
                if index != selected {
                    self.weights[index] = self.start[index] * fraction;
                    other += self.weights[index];
                }
            }
            self.weights[selected] = 1.0 - other;
        }
        for index in 0..2 {
            if self.weights[index + 1] == 0.0 && self.sides[index] != 0 {
                self.processor.reset(if index == 0 { FxKind::Echo } else { FxKind::Reverb });
                self.sides[index] = 0;
                self.input_gate[index] = 0.0;
            }
        }
        if self.weights == [1.0, 0.0, 0.0] { return filtered; }
        let mut frames = [filtered, input, input];
        for index in 1..3 {
            if self.weights[index] > 0.0 {
                frames[index] = self.wet(input, position, index - 1, samples_per_beat, rate);
            }
        }
        if frames.iter().all(|frame| *frame == input) { return input; }
        std::array::from_fn(|channel| (0..3).map(|index| frames[index][channel] * self.weights[index]).sum())
    }
}

#[cfg(test)]
mod tests;
