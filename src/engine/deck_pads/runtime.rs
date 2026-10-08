use super::{Mode, Press, Release};
use crate::engine::{
    deck_controls::{Button, Control},
    Command, RtEngine,
};

#[derive(Clone, Copy, Debug)]
enum Action {
    Hold(Button),
    Sampler(u8),
    Pitch { cue: u8, pad: u8, semitones: i8, media: u64 },
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
        if mode == Mode::PitchCue && !press.shifted && (self.decks[deck].audio.is_none() || !self.decks[deck].hotcues[usize::from(self.decks[deck].controls.pitch_cue)].set) { return; }
        let action = match mode {
            Mode::PitchCue if !press.shifted => Some(Action::Pitch { cue: self.decks[deck].controls.pitch_cue, pad, semitones: super::semitones(self.decks[deck].controls.pitch_range, pad).unwrap(), media: self.decks[deck].history_key }),
            Mode::HotCue if !press.shifted => Some(Action::Hold(Button::HotCue(pad))),
            Mode::Roll => Some(Action::Hold(Button::Roll(pad))),
            Mode::Slice => Some(Action::Hold(Button::Slice(pad))),
            Mode::Sampler | Mode::VelocitySampler if !press.shifted => {
                Some(Action::Sampler(press.deck * 8 + pad))
            }
            _ => None,
        };
        self.deck_pad_inputs.owners[..=index].rotate_right(1);
        self.deck_pad_inputs.owners[0] = Some(Owner {
            source: press.source,
            key: press.key,
            deck: press.deck,
            action,
        });
        match (mode, action) {
            (_, Some(Action::Pitch { cue, .. })) => {
                self.pitch_cue_hold(press.source, press.key, press.deck, cue, true);
                self.refresh_pitch_pad(deck);
            }
            (Mode::PitchCue, None) => self.deck_control(press.source, press.deck, Control::PitchPads { media_key: self.decks[deck].history_key, cue: pad, range: self.decks[deck].controls.pitch_range }),
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
    /// Follow the newest surviving chromatic pad without changing saved key or tempo settings.
    /// Takes the exact deck; retires stale media/actions and retunes fixed overlap history only when the applied note changes.
    pub(in crate::engine) fn refresh_pitch_pad(&mut self, deck: usize) {
        let d = &self.decks[deck];
        let next = self.deck_pad_inputs.owners.iter().flatten().find_map(|owner| {
            if usize::from(owner.deck) != deck { return None; }
            let Action::Pitch { cue, pad, semitones, media } = owner.action? else { return None; };
            (media == d.history_key && d.controls.owns(owner.source, Some(owner.key), Button::HotCue(cue))).then_some((pad, semitones))
        });
        let d = &mut self.decks[deck];
        let semitones = next.map(|note| note.1);
        d.controls.pitch_pad = next.map(|note| note.0);
        if d.controls.pitch_semitones != semitones {
            d.controls.pitch_semitones = semitones;
            d.retune(self.sr);
        }
    }
    /// Finish a captured action using its original owner.
    /// Takes the bounded owner receipt; releases only its exact held gesture or sampler input.
    fn finish_deck_pad(&mut self, owner: Owner) {
        match owner.action {
            Some(Action::Pitch { cue, .. }) => {
                self.pitch_cue_hold(owner.source, owner.key, owner.deck, cue, false);
                self.refresh_pitch_pad(usize::from(owner.deck));
            }
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
