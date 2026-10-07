use super::*;
use crate::engine::{
    cue_metadata::Name,
    deck_controls::{Control, SavedLoopAction},
};

impl App {
    /// Edit persistent slots through the existing source-qualified deck command route.
    /// Takes native UI, exact deck and applied snapshot; submits Save/Recall/Activate/name/order/delete transactions without starting playback.
    pub(super) fn saved_loops_editor(
        &mut self,
        ui: &mut Ui,
        deck: u8,
        snap: &crate::engine::DeckSnap,
    ) {
        ui.separator();
        ui.label("Saved loop slots");
        let settings = &mut self.loop_settings[usize::from(deck)];
        if settings.bank_key != snap.media_key {
            settings.bank_key = snap.media_key;
            settings.bank_styles = snap
                .saved_loops
                .slots
                .map(|slot| slot.map_or_else(Default::default, |slot| slot.style));
            settings.bank_names = settings
                .bank_styles
                .map(|style| style.name.as_str().to_owned());
        }
        let mut edits = Vec::new();
        for (position, id) in snap.saved_loops.order.into_iter().enumerate() {
            let index = usize::from(id - 1);
            let slot = snap.saved_loops.slots[index];
            let style = slot.map_or_else(Default::default, |slot| slot.style);
            if settings.bank_styles[index] != style {
                settings.bank_names[index] = style.name.as_str().to_owned();
                settings.bank_styles[index] = style;
            }
            ui.push_id(("saved-loop", deck, id), |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "{}{}",
                        id,
                        if snap.saved_loops.selected == id {
                            " •"
                        } else {
                            ""
                        }
                    ));
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut settings.bank_names[index])
                            .desired_width(100.0)
                            .char_limit(64),
                    );
                    let label = format!("Deck {}: Saved loop {id} name", (b'A' + deck) as char);
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::TextEdit,
                            response.enabled(),
                            &label,
                        )
                    });
                    accessibility::focus(ui, &response);
                    if let Some(slot) = slot {
                        ui.label(format!(
                            "{:.3}–{:.3} s",
                            slot.start,
                            slot.start + slot.length
                        ));
                    } else {
                        ui.label("Empty");
                    }
                    for (label, action, enabled) in [
                        ("Save", SavedLoopAction::Save, snap.loop_len >= 64.0),
                        (
                            "Recall",
                            SavedLoopAction::Recall { activate: false },
                            slot.is_some(),
                        ),
                        (
                            "Activate",
                            SavedLoopAction::Recall { activate: true },
                            slot.is_some(),
                        ),
                        (
                            "↑",
                            SavedLoopAction::Move {
                                position: position.saturating_sub(1) as u8,
                            },
                            position > 0,
                        ),
                        (
                            "↓",
                            SavedLoopAction::Move {
                                position: (position + 1).min(7) as u8,
                            },
                            position < 7,
                        ),
                        ("Delete", SavedLoopAction::Delete, slot.is_some()),
                    ] {
                        let response = ui.add_enabled(enabled, egui::Button::new(label).small());
                        let name = format!(
                            "{} saved loop {id}",
                            match label {
                                "↑" => "Move up",
                                "↓" => "Move down",
                                other => other,
                            }
                        );
                        accessibility::button(ui, &response, &name, None);
                        help::annotate(ui, &response, HelpControl::LoopEditor);
                        if response.clicked() {
                            edits.push((id, action));
                        }
                    }
                    let response =
                        ui.add_enabled(slot.is_some(), egui::Button::new("Rename").small());
                    accessibility::button(ui, &response, &format!("Rename saved loop {id}"), None);
                    help::annotate(ui, &response, HelpControl::LoopEditor);
                    if response.clicked() {
                        match Name::new(&settings.bank_names[index]) {
                            Ok(name) => edits.push((
                                id,
                                SavedLoopAction::Style {
                                    style: crate::engine::cue_metadata::Style { name, ..style },
                                },
                            )),
                            Err(_) => self.status =
                                "Loop names need at most 64 UTF-8 bytes without control characters"
                                    .into(),
                        }
                    }
                });
            });
        }
        ui.label("Slot IDs stay fixed when reordered. Recall prepares a loop; Activate jumps to its start. Loading restores the selection and armed loop with playback stopped.");
        for (id, action) in edits {
            self.send(Command::DeckControl {
                source: 0,
                deck,
                control: Control::SavedLoop {
                    media_key: snap.media_key,
                    id,
                    action,
                },
            });
        }
    }
}
