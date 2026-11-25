use super::{Note, State, Update, Values};
use crate::engine::{midi_edit::NoteId, InputKey};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Drum {
    pub note: Note,
    pub input: Option<InputKey>,
    pub clip: Option<NoteId>,
    pub values: Values,
    alias: u64,
    follows: bool,
    pub step: f64,
    gain: f32,
    alpha: f32,
    high: f32,
    low: f32,
}
impl Drum {
    pub(crate) fn new(
        state: &State,
        source: u64,
        channel: u8,
        input: Option<InputKey>,
        clip: Option<NoteId>,
        sr: f32,
    ) -> Option<Self> {
        let note = state.note(source, channel)?;
        let mut result = Self {
            note,
            input,
            clip,
            values: Values::default(),
            alias: match input {
                Some(InputKey::Midi { source, .. }) => source,
                _ => 0,
            },
            follows: true,
            step: 1.0,
            gain: 1.0,
            alpha: 0.0,
            high: 0.0,
            low: 0.0,
        };
        result.set(state.combined(note), sr);
        Some(result)
    }
    fn set(&mut self, values: Values, sr: f32) {
        self.values = values;
        self.step = 2.0f64.powf(f64::from(values.pitch) / 12.0);
        self.gain = 1.0 + f32::from(values.pressure) / 254.0;
        let cutoff = if values.timbre < 64 {
            20000.0 * 2.0f32.powf((f32::from(values.timbre) - 64.0) / 16.0)
        } else {
            2500.0
        };
        self.alpha = 1.0 - (-std::f32::consts::TAU * cutoff.min(sr * 0.45) / sr).exp();
        self.high = f32::from(values.timbre.saturating_sub(64)) / 126.0;
    }
    /// Follow expression on one finite sample voice.
    /// Takes its native source state, release masks and output rate; returns whether that exact sample must be silenced.
    pub(crate) fn update(&mut self, source: u64, state: &State, update: Update, sr: f32) -> bool {
        if !self.follows || self.note.source != source {
            return false;
        }
        let channel = 1 << self.note.channel;
        if (update.reset | update.silence) & channel != 0 {
            return true;
        }
        if update.release & channel != 0 {
            self.release();
        }
        if self.note.held {
            self.note.member = state.member(self.note.channel);
        }
        let values = state.combined(self.note);
        if values != self.values {
            self.set(values, sr);
        }
        false
    }
    pub(crate) fn release(&mut self) {
        self.note.held = false;
        self.input = None;
    }
    pub(crate) fn freeze_clip(&mut self) {
        if self.clip.is_some() {
            self.release();
            self.follows = false;
            self.clip = None;
        }
    }
    pub(crate) fn owned(&self, source: u64) -> bool {
        (source == 0 && (self.note.source != 0 || self.alias != 0))
            || source != 0 && (self.note.source == source || self.alias == source)
    }
    /// Render pressure and brightness for this sample only.
    /// Takes the original sample value; returns pressure gain and a per-voice low/high response while keeping neutral expression exact.
    pub(crate) fn sample(&mut self, value: f32) -> f32 {
        self.low += self.alpha * (value - self.low);
        let value = if self.values.timbre < 64 {
            self.low
        } else {
            value + self.high * (value - self.low)
        };
        value * self.gain
    }
}
