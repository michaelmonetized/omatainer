use super::{Control, State};
use crate::engine::{
    cue_metadata::Style,
    saved_loops::{Bank, Slot},
    DeckRt, DeckTransition, RtEngine,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Action {
    Save,
    Recall { activate: bool },
    Style { style: Style },
    Move { position: u8 },
    Delete,
    Cue { pad: u8 },
    UnlinkCue { pad: u8 },
}
impl Action {
    /// Validate a bounded display-order edit.
    /// Takes this slot action; returns whether every supplied index is supported.
    pub(crate) fn valid(self) -> bool {
        match self { Self::Move { position } => position < 8, Self::Cue { pad } | Self::UnlinkCue { pad } => pad < 8, _ => true }
    }
}
impl State {
    /// Capture all loop slots in decoded source seconds.
    /// Takes the native source rate; returns names, fixed IDs and selected order without allocation.
    pub(in crate::engine) fn saved_loops(&self, rate: u32) -> Bank {
        let rate = f64::from(rate.max(1));
        Bank {
            slots: std::array::from_fn(|i| {
                self.loops[i].map(|(start, length)| Slot {
                    start: start / rate,
                    length: length / rate,
                    style: self.loop_styles[i],
                })
            }),
            order: self.loop_order,
            selected: self.selected as u8 + 1,
            cue_loops: self.cue_loops,
        }
    }
    /// Restore prepared slots without starting transport or retaining old-source regions.
    /// Takes the complete bank, source rate and decoded extent; keeps only regions that fit this exact source.
    pub(in crate::engine) fn restore_saved_loops(&mut self, bank: Bank, rate: u32, frames: usize) {
        self.loops.fill(None);
        self.cue_loops.fill(None);
        self.loop_styles.fill(Style::default());
        self.loop_order = [1, 2, 3, 4, 5, 6, 7, 8];
        self.selected = 0;
        if !bank.valid() {
            return;
        }
        self.loop_order = bank.order;
        self.selected = usize::from(bank.selected - 1);
        self.cue_loops = bank.cue_loops;
        let rate = f64::from(rate);
        for (i, slot) in bank.slots.into_iter().enumerate() {
            if let Some(slot) = slot.filter(|slot| {
                slot.start * rate >= 0.0
                    && slot.length * rate + 1e-6 >= 64.0
                    && (slot.start + slot.length) * rate <= frames as f64 + 1e-6
            }) {
                self.loops[i] = Some((slot.start * rate, slot.length * rate));
                self.loop_styles[i] = slot.style;
            }
        }
    }
    /// Move through display order while keeping slot IDs fixed.
    /// Takes direction; selects the adjacent displayed slot, wrapping at the ends.
    pub(super) fn next_loop(&mut self, forward: bool) {
        let position = self
            .loop_order
            .iter()
            .position(|id| usize::from(*id) == self.selected + 1)
            .unwrap_or(0);
        self.selected =
            usize::from(self.loop_order[(position + if forward { 1 } else { 7 }) % 8] - 1);
    }
}
impl DeckRt {
    /// Refuse stale media, temporary performance loops and invalid regions before history capture.
    /// Takes a saved-slot command; returns whether it can act on the current exact source.
    pub(in crate::engine) fn saved_loop_current(&self, control: Control) -> bool {
        let Control::SavedLoop {
            media_key,
            id,
            action,
        } = control
        else {
            return false;
        };
        let Some(audio) = &self.audio else {
            return false;
        };
        if !control.valid() || media_key != self.history_key || self.controls.saved_loop.is_some() {
            return false;
        }
        match action {
            Action::Save => {
                self.loop_start.is_finite()
                    && self.loop_len.is_finite()
                    && self.loop_start >= 0.0
                    && self.loop_len >= 64.0
                    && self.loop_start + self.loop_len <= audio.frames() as f64
            }
            Action::Move { .. } => true,
            Action::UnlinkCue { pad } => self.controls.cue_loops[usize::from(pad)] == Some(id),
            _ => self.controls.loops[usize::from(id - 1)].is_some(),
        }
    }
}
impl RtEngine {
    /// Apply one guarded fixed-slot transaction and publish its durable preparation.
    /// Takes deck, stable ID and action; preserves other decks, playback ownership and callback heap bounds.
    pub(super) fn saved_loop_action(&mut self, deck: usize, id: u8, action: Action) {
        if matches!(action, Action::Recall { activate: true }) && self.decks[deck].controls.loops[usize::from(id - 1)].is_some() { self.deck_sync_manipulation(deck); }
        let d = &mut self.decks[deck];
        let before = (
            d.controls.loop_history(),
            d.loop_on,
            d.loop_start,
            d.loop_len,
            d.pos,
        );
        d.controls.cancel_pending();
        d.controls.edit = 0;
        d.controls.edit_ticks = None;
        d.controls.auto_button = None;
        let slot = usize::from(id - 1);
        match action {
            Action::Save => {
                d.controls.loops[slot] = Some((d.loop_start, d.loop_len));
                d.controls.selected = slot;
            }
            Action::Recall { activate } => {
                let Some((start, length)) = d.controls.loops[slot] else {
                    return;
                };
                d.controls.selected = slot;
                d.loop_start = start;
                d.loop_len = length;
                d.loop_on = activate;
                d.transition_to(
                    if activate { start } else { d.pos },
                    self.sr,
                    DeckTransition::Jump,
                );
            }
            Action::Style { style } => d.controls.loop_styles[slot] = style,
            Action::Move { position } => {
                let old = d
                    .controls
                    .loop_order
                    .iter()
                    .position(|value| *value == id)
                    .unwrap();
                let position = usize::from(position);
                if position > old {
                    d.controls.loop_order[old..=position].rotate_left(1);
                } else if position < old {
                    d.controls.loop_order[position..=old].rotate_right(1);
                }
            }
            Action::Cue { pad } => {
                let Some((start, _)) = d.controls.loops[slot] else { return; };
                let cue = usize::from(pad);
                d.hotcues[cue] = crate::engine::HotCue { set: true, pos: start };
                d.controls.cue_loops[cue] = Some(id);
            },
            Action::UnlinkCue { pad } => d.controls.cue_loops[usize::from(pad)] = None,
            Action::Delete => {
                d.controls.loops[slot] = None;
                d.controls.loop_styles[slot] = Style::default();
                if d.controls.selected == slot {
                    d.clear_loop();
                    d.transition_to(d.pos, self.sr, DeckTransition::Jump);
                }
            }
        }
        d.sync_cue_loop_positions();
        if before
            != (
                d.controls.loop_history(),
                d.loop_on,
                d.loop_start,
                d.loop_len,
                d.pos,
            )
        {
            d.publish_preparation();
            self.project.edited();
        }
    }
}
