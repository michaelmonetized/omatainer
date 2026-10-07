use super::{
    super::{DeckRt, RtEngine},
    Button, Command, Control, State,
};
use serde::Serialize;

pub(crate) const DIVISIONS: [f64; 6] = [0.125, 0.25, 0.5, 1.0, 2.0, 4.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuantizedAction {
    HotCue { pad: u8 },
    LoopIn,
    LoopOut,
    Reloop,
    NewLoop,
    SavedLoop { id: u8 },
}
#[derive(Clone, Copy, Debug, Serialize)]
pub struct PendingStatus {
    pub action: QuantizedAction,
    pub beat: f64,
    pub source_seconds: f64,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Pending {
    pub status: PendingStatus,
    pub owner: Option<(u64, Button)>,
    media_key: u64,
    position: f64,
    cue: Option<f64>,
    saved_loop: Option<(f64, f64)>,
    cancellation: u64,
    routing: (u64, u64),
}

impl State {
    /// Cancel a deferred performance gesture.
    /// Takes this deck state; clears queued work unless its due action is already being applied.
    pub(in crate::engine) fn cancel_pending(&mut self) {
        if !self.quantize_dispatching {
            self.pending = None;
        }
    }
}

impl DeckRt {
    /// Snap a source position to this deck's selected musical division.
    /// Takes source frames and fallback clock; returns the nearest bounded position through every manual tempo anchor.
    fn division_position(&self, position: f64, sr: f32, bpm: f32) -> f64 {
        let division = DIVISIONS[usize::from(self.controls.quantize_division)];
        let beat = (self.grid_beat_at(position, sr, bpm) / division).round() * division;
        self.grid_position_at(beat, sr, bpm).clamp(
            0.0,
            self.audio
                .as_ref()
                .map_or(f64::MAX, |audio| audio.frames() as f64),
        )
    }

    /// Quantize a cue without rewriting previously saved source positions.
    /// Takes source frames and fallback clock; returns unchanged frames until this deck explicitly enables quantization.
    pub(in crate::engine) fn cue_quantized_position(
        &self,
        position: f64,
        sr: f32,
        bpm: f32,
    ) -> f64 {
        if self.controls.quantize == Some(true) {
            self.division_position(position, sr, bpm).min(
                self.audio
                    .as_ref()
                    .map_or(f64::MAX, |audio| audio.frames().saturating_sub(1) as f64),
            )
        } else {
            position
        }
    }

    /// Resolve loop rounding while preserving older session defaults.
    /// Takes source frames, session policy and fallback clock; explicit deck settings take precedence over legacy loop rounding.
    pub(in crate::engine) fn loop_quantized_position(
        &self,
        position: f64,
        global: bool,
        sr: f32,
        bpm: f32,
    ) -> f64 {
        match self.controls.quantize {
            Some(true) => self.division_position(position, sr, bpm),
            Some(false) => position,
            None if global => self.grid_snap(position, sr, bpm),
            None => position,
        }
    }

    /// Check whether playback can own a musical onset.
    /// Takes this deck; returns false while stopped, scratching, reversing or using a temporary pad loop.
    fn quantized_transport(&self) -> bool {
        self.playing
            && self.audio.is_some()
            && !self.touching
            && !self.controls.braking
            && self.rate >= 0.0
            && !self.controls.held(Button::Reverse)
            && !self.controls.held(Button::Bleep)
            && self.controls.saved_loop.is_none()
            && !(self.follows_spindle()
                && self
                    .spindle
                    .as_ref()
                    .is_some_and(super::super::spindle::Playback::scratching))
    }
}

impl RtEngine {
    /// Queue one musical onset before capturing Undo history.
    /// Takes an ordinary controller command; returns true only when it replaces this deck's bounded pending action.
    pub(in crate::engine) fn defer_quantized_deck_command(&mut self, command: &Command) -> bool {
        let (deck, action) = match command {
            Command::DeckHotCue {
                deck,
                pad,
                del: false,
            } if *pad < 8 => (*deck, QuantizedAction::HotCue { pad: *pad }),
            Command::DeckLoopIn { deck } => (*deck, QuantizedAction::LoopIn),
            Command::DeckLoopOut { deck } => (*deck, QuantizedAction::LoopOut),
            Command::DeckReloop { deck } => (*deck, QuantizedAction::NewLoop),
            Command::DeckControl {
                deck,
                control: Control::Reloop,
                ..
            } => (*deck, QuantizedAction::Reloop),
            Command::DeckControl { deck, control: control @ Control::SavedLoop { id, action: super::SavedLoopAction::Recall { activate: true }, .. }, .. } if self.decks.get(usize::from(*deck)).is_some_and(|deck| deck.saved_loop_current(*control)) => (*deck, QuantizedAction::SavedLoop { id: *id }),
            _ => return false,
        };
        let Some(d) = self.decks.get_mut(usize::from(deck)) else {
            return false;
        };
        if d.controls.quantize_dispatching
            || d.controls.quantize != Some(true)
            || !d.quantized_transport()
            || d.history_key == 0
        {
            return false;
        }
        let cue = match action {
            QuantizedAction::HotCue { pad } => {
                let cue = &d.hotcues[usize::from(pad)];
                if !cue.set {
                    return false;
                }
                Some(cue.pos)
            }
            QuantizedAction::Reloop if d.loop_len <= 1.0 => return false,
            _ => None,
        };
        let division = DIVISIONS[usize::from(d.controls.quantize_division)];
        let beat = d.grid_beat_at(d.pos, self.sr, self.bpm);
        let next = ((beat / division) - 1e-9).ceil() * division;
        let position = d.grid_position_at(next, self.sr, self.bpm);
        let extent = d.audio.as_ref().unwrap().frames() as f64;
        if !position.is_finite()
            || position <= d.pos + 1e-6
            || position >= extent
            || d.loop_on && (position < d.loop_start || position >= d.loop_start + d.loop_len)
        {
            d.controls.pending = None;
            return false;
        }
        let cancellation = self.midi_learning.cancellation_epoch();
        let (_, generation, epoch) = self.midi_routing.output_state();
        if cancellation == u64::MAX || generation == u64::MAX || epoch == u64::MAX {
            d.controls.pending = None;
            return true;
        }
        d.controls.pending = Some(Pending {
            status: PendingStatus {
                action,
                beat: next,
                source_seconds: position / f64::from(d.audio.as_ref().unwrap().sr),
            },
            owner: d.controls.quantize_owner_hint,
            media_key: d.history_key,
            position,
            cue,
            saved_loop: match action { QuantizedAction::SavedLoop { id } => d.controls.loops[usize::from(id - 1)], _ => None },
            cancellation,
            routing: (generation, epoch),
        });
        true
    }

    /// Validate deferred work against live transport and source ownership.
    /// Takes the deck and captured action; returns false after a source reset, routing change, replacement or cue mutation.
    fn quantized_pending_current(&self, deck: usize, pending: Pending) -> bool {
        let d = &self.decks[deck];
        let (_, generation, epoch) = self.midi_routing.output_state();
        d.controls.quantize == Some(true)
            && d.quantized_transport()
            && d.history_key == pending.media_key
            && self.midi_learning.cancellation_epoch() == pending.cancellation
            && pending.routing == (generation, epoch)
            && match pending.status.action {
                QuantizedAction::HotCue { pad } => {
                    d.hotcues[usize::from(pad)].set
                        && Some(d.hotcues[usize::from(pad)].pos) == pending.cue
                }
                QuantizedAction::SavedLoop { id } => d.controls.loops[usize::from(id - 1)] == pending.saved_loop,
                _ => true,
            }
    }

    /// Retire invalid onsets even when the audio block has no frames.
    /// Takes this renderer; clears stale queued status without allocation or producer locks.
    pub(in crate::engine) fn quantized_deck_maintain(&mut self) {
        for deck in 0..self.decks.len() {
            if self.decks[deck]
                .controls
                .pending
                .is_some_and(|pending| !self.quantized_pending_current(deck, pending))
            {
                self.decks[deck].controls.pending = None;
            }
        }
    }

    /// Dispatch a due onset before rendering its first output sample.
    /// Takes exact deck index; applies its ordinary command once, retaining existing history and DSP transition handling.
    pub(in crate::engine) fn quantized_deck_tick(&mut self, deck: usize) {
        let Some(pending) = self.decks[deck].controls.pending else {
            return;
        };
        if !self.quantized_pending_current(deck, pending) {
            self.decks[deck].controls.pending = None;
            return;
        }
        if self.decks[deck].pos + 1e-6 < pending.position {
            return;
        }
        self.decks[deck].controls.pending = None;
        if pending
            .owner
            .is_some_and(|owner| !self.decks[deck].controls.owners.contains(&Some(owner)))
        {
            return;
        }
        let command = match pending.status.action {
            QuantizedAction::HotCue { pad } => Command::DeckHotCue {
                deck: deck as u8,
                pad,
                del: false,
            },
            QuantizedAction::SavedLoop { id } => Command::DeckControl { source: 0, deck: deck as u8, control: Control::SavedLoop { media_key: pending.media_key, id, action: super::SavedLoopAction::Recall { activate: true } } },
            QuantizedAction::LoopIn => Command::DeckLoopIn { deck: deck as u8 },
            QuantizedAction::LoopOut => Command::DeckLoopOut { deck: deck as u8 },
            QuantizedAction::NewLoop => Command::DeckReloop { deck: deck as u8 },
            QuantizedAction::Reloop => Command::DeckControl {
                source: 0,
                deck: deck as u8,
                control: Control::Reloop,
            },
        };
        self.decks[deck].controls.quantize_dispatching = true;
        self.apply(command);
        self.decks[deck].controls.quantize_dispatching = false;
        if !matches!(pending.status.action, QuantizedAction::HotCue { .. }) {
            self.remember_controller_loop(deck);
        }
    }
}
