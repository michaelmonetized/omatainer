use super::*;
use crate::engine::deck_controls::{Control, QuantizedAction, Status, QUANTIZE_DIVISIONS};

impl App {
    /// Show a deck's applied quantization and pending onset.
    /// Takes exact deck, renderer status and grid availability; submits a bounded division choice without changing session launches.
    pub(super) fn deck_quantization(
        &mut self,
        ui: &mut Ui,
        deck: u8,
        status: &Status,
        prepared: bool,
    ) {
        ui.push_id(("deck-quantization", deck), |ui| {
            let mut division = status
                .quantize_division
                .min((QUANTIZE_DIVISIONS.len() - 1) as u8);
            let response = egui::ComboBox::from_id_salt("division")
                .selected_text(format!(
                    "Quantize: {} beats{}",
                    QUANTIZE_DIVISIONS[usize::from(division)],
                    if status.quantize { "" } else { " · off" }
                ))
                .show_ui(ui, |ui| {
                    for (i, beats) in QUANTIZE_DIVISIONS.iter().enumerate() {
                        ui.selectable_value(&mut division, i as u8, format!("{beats} beats"));
                    }
                })
                .response;
            response.widget_info(|| {
                egui::WidgetInfo::labeled(
                    egui::WidgetType::ComboBox,
                    response.enabled(),
                    format!("Deck {}: Quantize division", (b'A' + deck) as char),
                )
            });
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_value(format!(
                    "{} beats",
                    QUANTIZE_DIVISIONS[usize::from(division)]
                ))
            });
            accessibility::focus(ui, &response);
            help::annotate(ui, &response, HelpControl::Quantize);
            if division != status.quantize_division {
                self.send(Command::DeckControl {
                    source: 0,
                    deck,
                    control: Control::Quantize {
                        enabled: status.quantize,
                        division,
                    },
                });
            }
            if status.quantize && !prepared {
                ui.label("Quantize uses the source BPM until a manual grid is prepared.");
            }
            if let Some(pending) = status.pending {
                let action = match pending.action {
                    QuantizedAction::HotCue { pad } => format!("Cue {}", pad + 1),
                    QuantizedAction::HotCueOnly { pad } => format!("Cue {} only", pad + 1),
                    QuantizedAction::CueLoop { pad, id } => format!("Cue {} + saved loop {id}", pad + 1),
                    QuantizedAction::SavedLoop { id } => format!("Saved loop {id}"),
                    QuantizedAction::LoopIn => "Loop in".into(),
                    QuantizedAction::LoopOut => "Loop out".into(),
                    QuantizedAction::Reloop | QuantizedAction::NewLoop => "Reloop".into(),
                };
                ui.label(format!(
                    "Queued {action} · beat {:.3} · {:.6} s",
                    pending.beat, pending.source_seconds
                ));
            }
        });
    }
}

#[cfg(test)]
mod tests;
