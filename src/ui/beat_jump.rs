use super::*;
use crate::engine::deck_controls::{Control, BEAT_JUMP_SIZES};

impl App {
    /// Show independent, renderer-confirmed beat jump controls.
    /// Takes a native UI, exact deck and published size; submits ordinary bounded performance commands.
    pub(super) fn deck_beat_jump(&mut self, ui: &mut Ui, deck: u8, size: u8) {
        ui.push_id(("beat-jump", deck), |ui| ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            let response = ui.small_button("◀");
            accessibility::button(ui, &response, "Beat jump backward", None);
            help::annotate(ui, &response, HelpControl::BeatJump);
            if response.clicked() { self.send(Command::DeckControl { source: 0, deck, control: Control::BeatJump { forward: false } }); }
            let mut index = size.min((BEAT_JUMP_SIZES.len() - 1) as u8);
            let combo = egui::ComboBox::from_id_salt("size").width(72.0)
                .selected_text(format!("{} beats", BEAT_JUMP_SIZES[usize::from(index)]))
                .show_ui(ui, |ui| {
                    for (i, beats) in BEAT_JUMP_SIZES.iter().enumerate() {
                        ui.selectable_value(&mut index, i as u8, format!("{beats} beats"));
                    }
                });
            combo.response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, combo.response.enabled(), format!("Deck {}: Beat jump size", (b'A' + deck) as char)));
            help::annotate(ui, &combo.response, HelpControl::BeatJump);
            accessibility::focus(ui, &combo.response);
            if index != size { self.send(Command::DeckControl { source: 0, deck, control: Control::BeatJumpSize { index } }); }
            let response = ui.small_button("▶");
            accessibility::button(ui, &response, "Beat jump forward", None);
            help::annotate(ui, &response, HelpControl::BeatJump);
            if response.clicked() { self.send(Command::DeckControl { source: 0, deck, control: Control::BeatJump { forward: true } }); }
        }));
    }
}

#[cfg(test)]
mod tests;
