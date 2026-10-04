//! Deck replacement decisions retain the reviewed target while its old audio plays.
use super::*;
use crate::engine::{load_receipt::{Media, Receipt, State}, performance::DeckApproval};

#[derive(Default)]
pub(super) struct Panel {
    review: Option<Review>,
    eject: Option<(u8, Receipt)>,
}
struct Review {
    deck: u8,
    word: u64,
    current: String,
    selected: Option<Selection>,
}
impl App {
    /// Check admission for an ordinary or reviewed load.
    /// Takes its target and optional approval; returns whether source preparation may start without changing the current deck.
    pub(super) fn deck_load_allows(&self, deck: u8, approval: Option<&DeckApproval>) -> bool {
        if let Some(approval) = approval {
            let receipt = Receipt::new().with_deck_approval(approval.clone());
            self.performance_allows(&Command::DeckLoadRequested { deck, media: Media::Builtin(255), receipt })
        } else {
            self.performance_allows(&Command::DeckLoadSelected { deck })
        }
    }
    /// Render deck locks and deliberate replacement controls.
    /// Takes the active safety toolbar; reports renderer acknowledgments and captures the displayed source for review.
    pub(super) fn deck_load_controls(&mut self, ui: &mut Ui) {
        if let Some((deck, receipt)) = &self.deck_load_panel.eject {
            match receipt.state() {
                State::Current => {
                    if let Some(load) = self.loads[*deck as usize].as_mut() { load.phase = Phase::Superseded; }
                    self.status = format!("Deck {} ejected", (b'A' + *deck) as char);
                },
                State::Protected | State::Unavailable | State::Superseded => self.status = format!("Deck {} eject refused; current audio is preserved", (b'A' + *deck) as char),
                _ => {},
            }
            if !matches!(receipt.state(), State::Pending | State::Applying) { self.deck_load_panel.eject = None; }
        }
        ui.horizontal_wrapped(|ui| {
            for deck in 0..DECKS {
                let mut locked = self.engine.cmd.performance().deck_load_word(deck) & 1 != 0;
                if ui.checkbox(&mut locked, crate::localization::format("Lock playing deck {}", &[((b'A' + deck as u8) as char).to_string()])).help(ui, HelpControl::DeckLoadLock).clicked() {
                    if let Err(error) = self.engine.cmd.performance().set_deck_load_lock(deck, locked) { self.status = error.to_string(); }
                }
            }
            let deck = self.load_target();
            if ui.button(tr!("Review load override…")).help(ui, HelpControl::DeckLoadReview).clicked() {
                let selected = self.selected_library_item().map(|item| Selection { title: item.title.clone(), source: item.source.clone() });
                if selected.is_some() { self.review_deck_load(deck, selected); }
                else { self.status = "Choose a library track before reviewing a load override".into(); }
            }
            if ui.button(tr!("Review eject…")).help(ui, HelpControl::DeckLoadReview).clicked() { self.review_deck_load(deck, None); }
        });
    }
    /// Capture the exact displayed deck for review.
    /// Takes its index and captured replacement, or none for eject; opens a decision without sending a load or invalidating a decode.
    fn review_deck_load(&mut self, deck: usize, selected: Option<Selection>) {
        let Some(current) = self.snap.decks.get(deck) else { return };
        self.deck_load_panel.review = Some(Review { deck: deck as u8, word: current.load_gate_word,
            current: current.title.clone(), selected });
    }
    /// Confirm or cancel one source-bound replacement.
    /// Takes the native context; only confirmation starts preparation, and the renderer repeats source and safety checks on completion.
    pub(super) fn deck_load_decision(&mut self, ctx: &egui::Context) {
        let Some(review) = &self.deck_load_panel.review else { return };
        keyboard::block_for_dialog(ctx);
        let mut confirm = false;
        let mut cancel = false;
        egui::Window::new(tr!("Review deck replacement")).id(egui::Id::new("deck-load-review")).collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(format!("Deck {} · current: {}", (b'A' + review.deck) as char, review.current));
            if let Some(selected) = &review.selected {
                ui.label(format!("Replace with: {}", selected.title));
                ui.label(tr!("Current audio keeps playing until the replacement is ready. A changed source or safety state refuses this review."));
            } else { ui.label(tr!("Eject this exact deck source. Audio stops only after renderer confirmation.")); }
            confirm = ui.button(if review.selected.is_some() { tr!("Confirm load on reviewed deck") } else { tr!("Confirm eject on reviewed deck") }).help(ui, HelpControl::DeckLoadReview).clicked();
            cancel = ui.button(tr!("Keep current deck audio")).help(ui, HelpControl::DeckLoadCancel).clicked();
        });
        if cancel { self.deck_load_panel.review = None; }
        else if confirm {
            let review = self.deck_load_panel.review.take().unwrap();
            match self.engine.cmd.performance().approve_deck_load(review.deck as usize, review.word) {
                Err(error) => self.status = format!("Deck review refused: {error}; review its current source again"),
                Ok(approval) => {
                    if let Some(selected) = review.selected { self.load_source_approved(review.deck, Some(&selected), Some(approval)); }
                    else {
                        let receipt = Receipt::new().with_deck_approval(approval);
                        if self.submit(Command::DeckLoadRequested { deck: review.deck, media: Media::Unload, receipt: receipt.clone() }) {
                            self.supersede_load(review.deck);
                            if let Some(loader) = &self.loader { let _ = loader.invalidate(review.deck); }
                            self.status = format!("Deck {} eject queued; waiting for the renderer", (b'A' + review.deck) as char);
                            self.deck_load_panel.eject = Some((review.deck, receipt));
                        }
                    }
                }
            }
        }
    }
}
