use super::*;
use crate::engine::deck_controls::{Control, QUANTIZE_DIVISIONS};

impl App {
    /// Configure phrase slicing and show every original source interval.
    /// Takes the named deck snapshot; draws independent domain, repeat and onset settings with confirmed active/pending markers.
    pub(super) fn slicer_controls(
        &mut self,
        ui: &mut Ui,
        deck: usize,
        snap: &crate::engine::DeckSnap,
        width: f32,
    ) {
        let state = snap.controls;
        let slicer = state.slicer;
        let mut repeating = slicer.repeating;
        let mut domain = state.slice_domain;
        let mut repeat = state.slice_quant;
        let mut division = slicer.division;
        ui.columns(2, |columns| {
            let ui = &mut columns[0];
            let response = egui::ComboBox::from_id_salt("slicer-behavior")
                .width((width - 3.0) / 2.0)
                .truncate()
                .selected_text(if repeating { "Repeating" } else { "Moving" })
                .show_ui(ui, |ui| {
                    for (value, label) in [(false, "Moving"), (true, "Repeating")] {
                        let response = ui.selectable_value(&mut repeating, value, label);
                        accessibility::button(
                            ui,
                            &response,
                            &format!("Slicer {label}"),
                            Some(repeating == value),
                        );
                        if response.clicked() {
                            ui.close();
                        }
                    }
                })
                .response;
            accessibility::button(ui, &response, "Slicer behavior", None);
            help::annotate(ui, &response, HelpControl::Slicer);
            let ui = &mut columns[1];
            let response = egui::ComboBox::from_id_salt("slicer-domain")
                .width((width - 3.0) / 2.0)
                .truncate()
                .selected_text(format!("{} beats", 1 << domain))
                .show_ui(ui, |ui| {
                    for index in 1..=6 {
                        let response = ui.selectable_value(
                            &mut domain,
                            index,
                            format!("{} beats", 1 << index),
                        );
                        accessibility::button(
                            ui,
                            &response,
                            &format!("Slicer domain {} beats", 1 << index),
                            Some(domain == index),
                        );
                        if response.clicked() {
                            ui.close();
                        }
                    }
                })
                .response;
            accessibility::button(ui, &response, "Slicer domain", None);
            help::annotate(ui, &response, HelpControl::Slicer);
        });
        ui.columns(2, |columns| {
            let ui = &mut columns[0];
            let response = egui::ComboBox::from_id_salt("slicer-repeat")
                .width((width - 3.0) / 2.0)
                .truncate()
                .selected_text(format!("Repeat 1/{}", 1 << repeat))
                .show_ui(ui, |ui| {
                    for index in 0..=3 {
                        let response =
                            ui.selectable_value(&mut repeat, index, format!("1/{}", 1 << index));
                        accessibility::button(
                            ui,
                            &response,
                            &format!("Slicer repeat 1/{}", 1 << index),
                            Some(repeat == index),
                        );
                        if response.clicked() {
                            ui.close();
                        }
                    }
                })
                .response;
            accessibility::button(ui, &response, "Slicer repeat length", None);
            help::annotate(ui, &response, HelpControl::Slicer);
            let text = |division: Option<u8>| {
                division.map_or_else(
                    || "Immediate".into(),
                    |i| format!("{} beats", QUANTIZE_DIVISIONS[usize::from(i)]),
                )
            };
            let ui = &mut columns[1];
            let response = egui::ComboBox::from_id_salt("slicer-onset")
                .width((width - 3.0) / 2.0)
                .truncate()
                .selected_text(text(division))
                .show_ui(ui, |ui| {
                    for value in std::iter::once(None).chain((0..6).map(Some)) {
                        let label = text(value);
                        let response = ui.selectable_value(&mut division, value, &label);
                        accessibility::button(
                            ui,
                            &response,
                            &format!("Slicer trigger {label}"),
                            Some(division == value),
                        );
                        if response.clicked() {
                            ui.close();
                        }
                    }
                })
                .response;
            accessibility::button(ui, &response, "Slicer trigger timing", None);
            help::annotate(ui, &response, HelpControl::Slicer);
        });
        if repeating != slicer.repeating
            || domain != state.slice_domain
            || repeat != state.slice_quant
            || division != slicer.division
        {
            self.send(Command::DeckControl {
                source: self.deck_pad_inputs.source,
                deck: deck as u8,
                control: Control::SlicerSettings {
                    repeating,
                    domain,
                    repeat,
                    division,
                },
            });
        }
        if let Some(bounds) = slicer.bounds {
            let rate = f64::from(snap.source_sample_rate.max(1));
            ui.add(
                egui::Label::new(
                    RichText::new(format!(
                        "Phrase {:.3}–{:.3} s",
                        bounds[0] / rate,
                        bounds[8] / rate
                    ))
                    .small(),
                )
                .truncate(),
            );
            let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
            let width = rect.width() / 8.0;
            for pad in 0..8 {
                let active = slicer.active == Some(pad);
                let pending = slicer.pending == Some(pad);
                let color = if pending {
                    Color32::from_rgb(255, 160, 40)
                } else if active {
                    Color32::from_rgb(240, 215, 40)
                } else {
                    Color32::from_gray(55)
                };
                let cell = Rect::from_min_size(
                    rect.min + Vec2::new(f32::from(pad) * width, 0.0),
                    Vec2::new(width - 1.0, 24.0),
                );
                ui.painter().rect_filled(cell, 0.0, color);
                ui.painter().text(
                    cell.center(),
                    egui::Align2::CENTER_CENTER,
                    (pad + 1).to_string(),
                    egui::FontId::proportional(12.0),
                    Color32::WHITE,
                );
            }
            let description = (0..8)
                .map(|i| {
                    format!(
                        "Slice {}: {:.3}–{:.3} source seconds",
                        i + 1,
                        bounds[i] / rate,
                        bounds[i + 1] / rate
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            accessibility::button(ui, &response, "Slicer source boundaries", None);
            accessibility::status(ui, &response, &description);
            response.on_hover_text(description);
            if let Some(pad) = slicer.pending {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "Slice {} waiting for beat {:.3}",
                            pad + 1,
                            slicer.pending_beat.unwrap_or(0.0)
                        ))
                        .small(),
                    )
                    .truncate(),
                );
            } else if let Some(pad) = slicer.active {
                ui.add(
                    egui::Label::new(RichText::new(format!("Slice {} active", pad + 1)).small())
                        .truncate(),
                );
            } else {
                ui.add(egui::Label::new(RichText::new("Slice ready").small()).truncate());
            }
        } else {
            ui.add(
                egui::Label::new(RichText::new("Phrase: play to show source").small()).truncate(),
            );
            let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, Color32::from_gray(55));
            accessibility::button(ui, &response, "Slicer source boundaries", None);
            accessibility::status(
                ui,
                &response,
                "Source boundaries appear when this deck plays.",
            );
            ui.add(egui::Label::new(RichText::new("Slice ready").small()).truncate());
        }
    }
}

#[cfg(test)]
mod tests;
