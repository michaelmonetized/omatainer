use super::*;

pub(super) enum Action {
    Load(Selection),
    Eject,
}
pub(super) struct Review {
    deck: u8,
    expected: u64,
    approval: crate::engine::performance::DeckApproval,
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
        if !target.load_locked || !target.media_active {
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
            expected: target.load_gate_word,
            approval: match self
                .engine
                .cmd
                .performance()
                .approve_deck_load(deck as usize, target.load_gate_word)
            {
                Ok(approval) => approval,
                Err(error) => {
                    self.status = error.to_string();
                    return true;
                }
            },
            title: target.title.clone(),
            action,
            acknowledged: false,
        });
        true
    }
    /// Check ordinary load admission or a captured source review.
    /// Takes its deck and approval; returns shared producer admission.
    pub(super) fn deck_load_allows(
        &self,
        deck: u8,
        approval: Option<&crate::engine::performance::DeckApproval>,
    ) -> bool {
        if let Some(approval) = approval {
            self.performance_allows(&Command::DeckLoadRequested {
                deck,
                media: Media::Builtin(0),
                receipt: Receipt::new().with_deck_approval(approval.clone()),
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
            .is_none_or(|deck| deck.load_gate_word != review.expected)
            || self.engine.cmd.performance().status().protected
        {
            self.status =
                "Deck changed or performance protection entered; review its current track again"
                    .into();
            return;
        }
        keyboard::block_for_dialog(ctx);
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
                        egui::Button::new(tr!("Confirm deck replacement"))).help(ui, HelpControl::DeckLoadReview).clicked();
                    cancel = ui.button(tr!("Keep playing track")).help(ui, HelpControl::DeckLoadCancel).clicked();
                });
            });
        if commit {
            match review.action {
                Action::Load(selection) => {
                    self.load_source_approved(review.deck, Some(&selection), Some(review.approval))
                }
                Action::Eject => {
                    let receipt = Receipt::new().with_deck_approval(review.approval);
                    let mut state = LoadState::new(None, Phase::Queued);
                    state.receipt = Some(receipt.clone());
                    if !self.submit(Command::DeckLoadRequested {
                        deck: review.deck,
                        media: Media::Unload,
                        receipt,
                    }) {
                        state.phase = Phase::Failed("Reviewed eject was not accepted".into());
                    }
                    self.set_load_state(review.deck, state);
                }
            }
        } else if open && !cancel {
            self.deck_load_review = Some(review);
        }
    }
}

#[cfg(test)]
mod tests;
