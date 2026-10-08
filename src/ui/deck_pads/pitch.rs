use super::*;
use crate::engine::deck_controls::Control;

impl App {
    /// Choose an existing cue and an explicit chromatic range on this deck.
    /// Takes its native panel and confirmed source snapshot; submits source-qualified settings or an original-key reset through ordinary admission.
    pub(super) fn pitch_cue_controls(
        &mut self,
        ui: &mut Ui,
        deck: usize,
        snap: &crate::engine::DeckSnap,
    ) {
        ui.push_id("chromatic-cue-settings", |ui| {
            let chooser = egui::ComboBox::from_id_salt("root").selected_text(format!("Cue {}", snap.controls.pitch_cue + 1)).show_ui(ui, |ui| {
                for cue in 0..8 {
                    let label = format!("Pitch root cue {}", cue + 1);
                    let response = ui.add_enabled(snap.hotcues[cue], egui::Button::selectable(snap.controls.pitch_cue == cue as u8, format!("Cue {} · {}", cue + 1, snap.cue_styles[cue].name.as_str())));
                    accessibility::button(ui, &response, &label, Some(snap.controls.pitch_cue == cue as u8));
                    if response.clicked() {
                        self.send(Command::DeckControl { source: self.deck_pad_inputs.source, deck: deck as u8, control: Control::PitchPads { media_key: snap.media_key, cue: cue as u8, range: snap.controls.pitch_range } });
                        ui.close();
                    }
                }
            }).response;
            accessibility::button(ui, &chooser, "Pitch root", None);
            ui.horizontal_wrapped(|ui| {
                for (range, name) in ["Low", "Center", "High"].into_iter().enumerate() {
                    let response = ui.add_enabled(snap.hotcues[usize::from(snap.controls.pitch_cue)], egui::Button::selectable(snap.controls.pitch_range == range as u8, name));
                    accessibility::button(ui, &response, &format!("Pitch range {}", name.to_lowercase()), Some(snap.controls.pitch_range == range as u8));
                    if response.clicked() { self.send(Command::DeckControl { source: self.deck_pad_inputs.source, deck: deck as u8, control: Control::PitchPads { media_key: snap.media_key, cue: snap.controls.pitch_cue, range: range as u8 } }); }
                }
                let response = ui.add_enabled(snap.media_key != 0, egui::Button::new("Original key"));
                accessibility::button(ui, &response, "Pitch original key", None);
                if response.clicked() { self.send(Command::DeckControl { source: self.deck_pad_inputs.source, deck: deck as u8, control: Control::PitchReset { media_key: snap.media_key } }); }
            });
            ui.small("Pads retrigger the chosen cue immediately. Tempo stays independent; releasing restores the deck key setting.");
            if !snap.hotcues[usize::from(snap.controls.pitch_cue)] { ui.small("Choose an existing cue before playing chromatic pads."); }
            if snap.controls.pitch_pad.is_some() {
                match snap.keylock_mode {
                    crate::engine::keylock::Mode::ScratchBypass => { ui.small("Scratch/reverse bypasses cue pitch until forward playback resumes."); }
                    crate::engine::keylock::Mode::UnsupportedRate => { ui.small("Cue pitch is bypassed in reverse or at this tempo. Resume supported forward playback or choose a nearer pitch."); }
                    _ => {}
                }
            }
        });
    }
}
