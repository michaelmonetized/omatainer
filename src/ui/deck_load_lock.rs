use super::*;

pub(super) enum Action {
    Load(Selection),
    Eject,
}
pub(super) struct Review {
    deck: u8,
    expected: u64,
    title: String,
    action: Action,
    acknowledged: bool,
}
impl App {
    /// Capture a protected deck and the chosen replacement.
    /// Takes a deck and selection; returns whether a native review now owns the load.
    pub(super) fn review_locked_load(&mut self, deck: u8, selection: Option<Selection>) -> bool {
        let Some(selection) = selection else {
            return false;
        };
        self.review_locked_deck(deck, Action::Load(selection))
    }
    /// Capture a protected deck for deliberate eject.
    /// Takes its index; returns whether a native review now owns the eject.
    pub(super) fn review_locked_eject(&mut self, deck: u8) -> bool {
        self.review_locked_deck(deck, Action::Eject)
    }
    fn review_locked_deck(&mut self, deck: u8, action: Action) -> bool {
        let snapshot = self.engine.snapshot();
        let Some(target) = snapshot.decks.get(deck as usize) else {
            return false;
        };
        if !target.load_locked || (!target.playing && !target.touching) {
            return false;
        }
        if self.engine.cmd.performance().status().protected {
            self.status =
                "Performance mode requires pausing and releasing this deck before replacement"
                    .into();
            return true;
        }
        self.deck_load_review = Some(Review {
            deck,
            expected: target.media_key,
            title: target.title.clone(),
            action,
            acknowledged: false,
        });
        true
    }
    /// Check ordinary load admission or consent for a single current track.
    /// Takes the deck and optional reviewed media key; returns producer admission status.
    pub(super) fn deck_load_allowed(&self, deck: u8, expected: Option<u64>) -> bool {
        if let Some(expected) = expected {
            self.performance_allows(&Command::DeckLoadRequested {
                deck,
                media: Media::Builtin(0),
                receipt: Receipt::with_override(None, Some(expected)),
            })
        } else {
            self.performance_allows(&Command::DeckLoadSelected { deck })
        }
    }
    pub(super) fn deck_load_confirmation(&mut self, ctx: &egui::Context) {
        let Some(mut review) = self.deck_load_review.take() else {
            return;
        };
        let snapshot = self.engine.snapshot();
        if snapshot
            .decks
            .get(review.deck as usize)
            .is_none_or(|deck| deck.media_key != review.expected)
            || self.engine.cmd.performance().status().protected
        {
            self.status =
                "Deck changed or performance protection entered; review its current track again"
                    .into();
            return;
        }
        let mut open = true;
        let mut commit = false;
        let mut cancel = false;
        egui::Window::new(tr!("Review protected deck replacement"))
            .id(egui::Id::new("deck-load-review"))
            .open(&mut open).collapsible(false).resizable(false)
            .show(ctx, |ui| {
                ui.label(crate::localization::format("Deck {} currently plays: {}", &[
                    ((b'A' + review.deck) as char).to_string(), review.title.clone(),
                ]));
                match &review.action {
                    Action::Load(selection) => {
                        ui.label(crate::localization::format("Replacement: {}", &[selection.title.clone()]));
                        ui.label(tr!("The current track stays loaded while the replacement decodes. Successful application stops this deck. Failed or canceled work preserves its current track."));
                    }
                    Action::Eject => { ui.label(tr!("Eject stops this deck and removes its current track.")); }
                }
                ui.checkbox(&mut review.acknowledged, tr!("I intend to replace this playing track"));
                ui.horizontal(|ui| {
                    commit = ui.add_enabled(review.acknowledged,
                        egui::Button::new(tr!("Confirm deck replacement"))).clicked();
                    cancel = ui.button(tr!("Keep playing track")).clicked();
                });
            });
        if commit {
            match review.action {
                Action::Load(selection) => self.load_source_authorized(
                    review.deck,
                    Some(&selection),
                    Some(review.expected),
                ),
                Action::Eject => {
                    if self.submit(Command::DeckEjectConfirmed {
                        deck: review.deck,
                        expected: review.expected,
                    }) {
                        self.status = "Reviewed deck eject queued; waiting for the renderer".into();
                    }
                }
            }
        } else if open && !cancel {
            self.deck_load_review = Some(review);
        }
    }
}

#[cfg(test)]
mod tests;
