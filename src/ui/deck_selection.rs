//! Keep accepted GUI selection distinct from a lagging renderer snapshot.
use super::*;

pub(super) struct Selection {
    next_request: u64,
    pending: Option<(u64, usize)>,
    pointer_order: Option<usize>,
}
impl Selection {
    pub(super) fn begin_pointer_frame(&mut self) { self.pointer_order = None; }

    pub(super) fn new(applied_request: u64) -> Self {
        Self { next_request: applied_request, pending: None, pointer_order: None }
    }
}

impl App {
    pub(super) fn load_target(&self) -> usize {
        self.deck_selection.pending
            .filter(|(request, _)| *request > self.snap.selected_deck_request)
            .map(|(_, deck)| deck)
            .unwrap_or(self.snap.selected_deck)
            .min(DECKS - 1)
    }

    fn select_deck(&mut self, deck: usize) {
        if deck >= DECKS || self.load_target() == deck { return; }
        let Some(request) = self.deck_selection.next_request
            .max(self.snap.selected_deck_request).checked_add(1) else {
                self.status = "Deck selection request limit reached".into();
                return;
            };
        if self.submit(Command::SelectDeckRequested { deck, request }) {
            self.deck_selection.next_request = request;
            self.deck_selection.pending = Some((request, deck));
        }
    }

    pub(super) fn select_deck_from_pointer(&mut self, ui: &Ui, rect: Rect, deck: usize) {
        // Respect clipping and topmost layer; hovering, release outside a
        // different deck, and overlapping dialogs cannot steal selection.
        if !ui.is_enabled() { return; }
        let rect = rect.intersect(ui.clip_rect());
        let mut before = usize::MAX;
        loop {
            let press = ui.input(|input| input.events.iter().enumerate().take(before).rev().find_map(|(index, event)| match event {
                egui::Event::PointerButton { pos, pressed: true, .. } if rect.contains(*pos) => Some((index, *pos)),
                _ => None,
            }));
            let Some((index, pos)) = press else { break; };
            if self.deck_selection.pointer_order.is_some_and(|previous| index <= previous) { break; }
            before = index;
            if ui.ctx().layer_id_at(pos) == Some(ui.layer_id()) {
                // Widget traversal order must not reverse several presses
                // delivered in one frame. Ignore covered presses individually:
                // an earlier visible press may still be this frame's winner.
                self.deck_selection.pointer_order = Some(index);
                self.select_deck(deck);
                break;
            }
        }
    }

    pub(super) fn deck_selectors(&mut self, ui: &mut Ui) {
        let queued = self.deck_selection.pending
            .is_some_and(|(request, _)| request > self.snap.selected_deck_request);
        ui.label(if queued { tr!("load target (queued)") } else { tr!("load target") });
        for deck in 0..DECKS {
            let selected=self.load_target()==deck;
            let label=(b'A'+deck as u8) as char;
            let response=ui.selectable_label(selected,crate::localization::format("Deck {}", &[label.to_string()])).help(ui,HelpControl::DeckSelect);
            accessibility::button(ui,&response,&crate::localization::format("Load target deck {0}", &[label.to_string()]),Some(selected));
            if response.clicked() {self.select_deck(deck);}
        }
    }
}

#[cfg(test)]
mod tests;
