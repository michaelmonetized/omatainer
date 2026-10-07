use super::*;
use crate::engine::deck_controls::{Control, BEAT_JUMP_SIZES};

mod saved;
pub(super) struct Settings {
    bank_key: u64,
    bank_styles: [crate::engine::cue_metadata::Style; 8],
    bank_names: [String; 8],
    bank_cues: [u8; 8],
    length: u8,
    movement: u8,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            bank_key: 0,
            bank_styles: [Default::default(); 8],
            bank_names: std::array::from_fn(|_| String::new()),
            bank_cues: std::array::from_fn(|cue| cue as u8),
            length: 5,
            movement: 3,
        }
    }
}

impl App {
    /// Edit source-qualified loop boundaries in the native deck panel.
    /// Takes its exact deck and renderer snapshot; submits bounded whole-region changes while the waveform continues showing applied boundaries.
    pub(super) fn deck_loop_editor(
        &mut self,
        ui: &mut Ui,
        deck: u8,
        snap: &crate::engine::DeckSnap,
    ) {
        ui.push_id(("loop-editor", deck), |ui| {
            let label = format!("Deck {}: Loop editor", (b'A' + deck) as char);
            let section = egui::CollapsingHeader::new("Loop editor").show(ui, |ui| {
                ui.add_enabled_ui(snap.media_key != 0 && snap.source_sample_rate > 0 && snap.frames >= 64.0
                    && snap.controls.roll.is_none() && snap.controls.slice.is_none()
                    && !self.project.committing() && self.project.dialog_is_closed(), |ui| {
                    let rate = f64::from(snap.source_sample_rate.max(1));
                    let mut start = snap.loop_start / rate;
                    let mut end = (snap.loop_start + snap.loop_len) / rate;
                    ui.label(format!("Applied: {start:.6}–{end:.6} s{}", if snap.loop_on { " · active" } else { "" }));
                    let response = ui.small_button(if snap.loop_on { "Disable loop" } else { "Enable loop" });
                    accessibility::button(ui, &response, if snap.loop_on { "Disable loop" } else { "Enable loop" }, Some(snap.loop_on));
                    help::annotate(ui, &response, HelpControl::LoopEditor);
                    if response.clicked() { self.send(Command::DeckControl { source: 0, deck, control: Control::LoopToggle }); }
                    let mut bounds = false;
                    for (name, value) in [("Loop start seconds", &mut start), ("Loop end seconds", &mut end)] {
                        ui.horizontal(|ui| {
                            ui.label(if name.starts_with("Loop start") { "In" } else { "Out" });
                            let response = ui.add(egui::DragValue::new(value).speed(1.0 / rate)
                                .range(0.0..=snap.frames / rate).fixed_decimals(6).suffix(" s"));
                            let field = format!("Deck {}: {name}", (b'A' + deck) as char);
                            response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::DragValue, response.enabled(), &field));
                            help::annotate(ui, &response, HelpControl::LoopEditor);
                            accessibility::focus(ui, &response);
                            bounds |= response.changed();
                            for (direction, sign) in [("earlier", -1.0), ("later", 1.0)] {
                                let response = ui.small_button(if sign < 0.0 { "−" } else { "+" });
                                accessibility::button(ui, &response, &format!("{name} one frame {direction}"), None);
                                help::annotate(ui, &response, HelpControl::LoopEditor);
                                if response.clicked() { *value += sign / rate; bounds = true; }
                            }
                        });
                    }
                    if bounds {
                        self.send(Command::DeckControl { source: 0, deck, control: Control::LoopBounds {
                            media_key: snap.media_key, start_seconds: start, end_seconds: end,
                        } });
                    }
                    let settings = &mut self.loop_settings[usize::from(deck)];
                    size(ui, deck, "Loop length", &mut settings.length);
                    let length = f64::from(BEAT_JUMP_SIZES[usize::from(settings.length)]);
                    let response = ui.small_button("Set loop length");
                    accessibility::button(ui, &response, "Set loop length", None);
                    help::annotate(ui, &response, HelpControl::LoopEditor);
                    if response.clicked() { self.send(Command::DeckControl { source: 0, deck, control: Control::LoopLength { media_key: snap.media_key, beats: length } }); }
                    let settings = &mut self.loop_settings[usize::from(deck)];
                    size(ui, deck, "Loop move size", &mut settings.movement);
                    let movement = f64::from(BEAT_JUMP_SIZES[usize::from(settings.movement)]);
                    ui.horizontal(|ui| for (name, sign) in [("Move loop backward", -1.0), ("Move loop forward", 1.0)] {
                        let response = ui.small_button(if sign < 0.0 { "◀ loop" } else { "loop ▶" });
                        accessibility::button(ui, &response, name, None);
                        help::annotate(ui, &response, HelpControl::LoopEditor);
                        if response.clicked() { self.send(Command::DeckControl { source: 0, deck, control: Control::LoopMove { media_key: snap.media_key, beats: movement * sign } }); }
                    });
                    self.saved_loops_editor(ui, deck, snap);
                    ui.label("Track edges shift the complete loop. Lengths that cannot fit are refused. Minimum: 64 source frames.");
                });
            });
            section.header_response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::CollapsingHeader, section.header_response.enabled(), &label));
            help::annotate(ui, &section.header_response, HelpControl::LoopEditor);
        });
    }
}

/// Choose a bounded musical edit size.
/// Takes its native panel, exact deck, name and local selection; updates only the draft size until a performance button is pressed.
fn size(ui: &mut Ui, deck: u8, name: &str, index: &mut u8) {
    let response = egui::ComboBox::from_id_salt(name)
        .selected_text(format!(
            "{name}: {} beats",
            BEAT_JUMP_SIZES[usize::from(*index)]
        ))
        .show_ui(ui, |ui| {
            for (i, beats) in BEAT_JUMP_SIZES.iter().enumerate() {
                ui.selectable_value(index, i as u8, format!("{beats} beats"));
            }
        })
        .response;
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::ComboBox,
            response.enabled(),
            format!("Deck {}: {name}", (b'A' + deck) as char),
        )
    });
    accessibility::focus(ui, &response);
    help::annotate(ui, &response, HelpControl::LoopEditor);
}

#[cfg(test)]
mod tests;
