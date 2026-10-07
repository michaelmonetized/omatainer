use super::{Mode, Press, Release};
use crate::engine::{
    deck_controls::{Button, Control},
    Command, RtEngine,
};

#[derive(Clone, Copy, Debug)]
enum Action {
    Hold(Button),
    Sampler(u8),
}
#[derive(Clone, Copy, Debug)]
struct Owner {
    source: u64,
    key: u32,
    deck: u8,
    action: Option<Action>,
}
pub(in crate::engine) struct State {
    owners: [Option<Owner>; crate::engine::control::MAX_COMMANDS],
}
impl Default for State {
    fn default() -> Self {
        Self {
            owners: [None; crate::engine::control::MAX_COMMANDS],
        }
    }
}
impl RtEngine {
    /// Resolve one fixed pad through the current deck mode.
    /// Takes a validated onset; captures exact physical key, deck and action before applying the existing handlers.
    pub(in crate::engine) fn deck_pad_press(&mut self, press: Press) {
        if !press.valid() {
            return;
        }
        if self
            .deck_pad_inputs
            .owners
            .iter()
            .flatten()
            .any(|owner| owner.source == press.source && owner.key == press.key)
        {
            return;
        }
        let deck = usize::from(press.deck);
        let mode = press.mode.unwrap_or_else(|| {
            Mode::from_index(self.decks[deck].controls.status().pad_mode).unwrap_or(Mode::HotCue)
        });
        if mode.index() != self.decks[deck].controls.status().pad_mode {
            self.deck_control(
                press.source,
                press.deck,
                Control::PadMode { mode: mode.index() },
            );
        }
        let Some(index) = self.deck_pad_inputs.owners.iter().position(Option::is_none) else {
            return;
        };
        let pad = press.id - 1;
        let action = match mode {
            Mode::HotCue if !press.shifted => Some(Action::Hold(Button::HotCue(pad))),
            Mode::Roll => Some(Action::Hold(Button::Roll(pad))),
            Mode::Slice => Some(Action::Hold(Button::Slice(pad))),
            Mode::Sampler | Mode::VelocitySampler if !press.shifted => {
                Some(Action::Sampler(press.deck * 8 + pad))
            }
            _ => None,
        };
        self.deck_pad_inputs.owners[index] = Some(Owner {
            source: press.source,
            key: press.key,
            deck: press.deck,
            action,
        });
        match (mode, action) {
            (_, Some(Action::Hold(button))) => self.deck_control_owned(
                press.source,
                press.deck,
                Control::Hold { button, on: true },
                Some(press.key),
            ),
            (_, Some(Action::Sampler(pad))) => self.surface_sampler_owned(
                press.source,
                Some(press.key),
                pad,
                true,
                if mode == Mode::VelocitySampler {
                    press.pressure
                } else {
                    1.0
                },
            ),
            (Mode::HotCue, None) => self.apply(Command::DeckHotCue {
                deck: press.deck,
                pad,
                del: true,
            }),
            (Mode::Sampler | Mode::VelocitySampler, None) => self.apply(Command::SamplerSlotStop {
                pad: press.deck * 8 + pad,
            }),
            (Mode::SavedLoop, None) => self.apply(Command::DeckControl {
                source: press.source,
                deck: press.deck,
                control: Control::HotLoop {
                    pad,
                    clear: press.shifted,
                },
            }),
            (Mode::AutoLoop, None) => self.apply(Command::DeckControl {
                source: press.source,
                deck: press.deck,
                control: Control::AutoLoopPad { pad },
            }),
            (Mode::ManualLoop, None) => self.apply(Command::DeckControl {
                source: press.source,
                deck: press.deck,
                control: Control::ManualPad { pad },
            }),
            _ => {}
        }
    }
    /// Release the original action without looking up a changed deck, mode or bank.
    /// Takes the original source and raw key; ignores a cancelled or already retired action.
    pub(in crate::engine) fn deck_pad_release(&mut self, release: Release) {
        let Some(index) = self.deck_pad_inputs.owners.iter().position(|owner| {
            owner.is_some_and(|owner| owner.source == release.source && owner.key == release.key)
        }) else {
            return;
        };
        let owner = self.deck_pad_inputs.owners[index].take().unwrap();
        self.finish_deck_pad(owner);
    }
    /// Finish this deck's pad-owned gestures before selecting another mode.
    /// Takes exact deck index; leaves ordinary held controls and other decks untouched and retains physical release ownership.
    pub(in crate::engine) fn cancel_deck_pads(&mut self, deck: usize) {
        for index in 0..self.deck_pad_inputs.owners.len() {
            if let Some(owner) =
                self.deck_pad_inputs.owners[index].filter(|owner| usize::from(owner.deck) == deck)
            {
                self.deck_pad_inputs.owners[index].as_mut().unwrap().action = None;
                self.finish_deck_pad(owner);
            }
        }
    }
    /// Finish a captured action using its original owner.
    /// Takes the bounded owner receipt; releases only its exact held gesture or sampler input.
    fn finish_deck_pad(&mut self, owner: Owner) {
        match owner.action {
            Some(Action::Hold(button)) => self.deck_control_owned(
                owner.source,
                owner.deck,
                Control::Hold { button, on: false },
                Some(owner.key),
            ),
            Some(Action::Sampler(pad)) => {
                self.surface_sampler_owned(owner.source, Some(owner.key), pad, false, 0.0)
            }
            None => {}
        }
    }
}
impl State {
    /// Forget renderer ownership after the admission safety epoch retires its reservations.
    /// Takes this fixed state; permits fresh post-recovery presses without restoring earlier gestures.
    pub(in crate::engine) fn clear(&mut self) {
        self.owners.fill(None);
    }
}
