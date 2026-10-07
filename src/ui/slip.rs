use super::*;
use crate::engine::deck_controls::{Control, QUANTIZE_DIVISIONS};

#[derive(Default)]
pub(super) struct Panel {
    pub open: bool,
}

impl App {
    /// Configure slip playback and show its predicted source return.
    /// Takes GUI context; exposes independent confirmed enablement, release timing and active or pending shadow position.
    pub(super) fn slip_ui(&mut self, ctx: &egui::Context) {
        if !self.slip.open {
            return;
        }
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        let mut actions = Vec::new();
        egui::Window::new("Slip playback").id(egui::Id::new("slip-playback")).open(&mut open).resizable(false).show(ctx,|ui|{
            ui.label("Keep a background playhead during scratch, held cue and temporary-loop gestures. The final release returns to that timeline.");
            for deck in 0..DECKS {ui.push_id(deck,|ui|{
                let label=(b'A'+deck as u8) as char;let snapshot=&self.snap.decks[deck];let state=snapshot.controls;
                ui.separator();ui.heading(format!("Deck {label}"));
                let mut enabled=state.slip;let mut release=state.slip_release;
                let response=ui.checkbox(&mut enabled,"Enable slip playback");
                response.widget_info(||egui::WidgetInfo::selected(egui::WidgetType::Checkbox,true,enabled,format!("Deck {label}: Enable slip playback")));
                accessibility::focus(ui,&response);help::annotate(ui,&response,HelpControl::SlipPlayback);
                let text=|release:Option<u8>|release.map_or_else(||"Immediate".into(),|index|format!("{} beats",QUANTIZE_DIVISIONS[usize::from(index)]));
                let response=egui::ComboBox::from_id_salt("slip-release").selected_text(text(release)).show_ui(ui,|ui|{
                    ui.selectable_value(&mut release,None,"Immediate");for(index,beats)in QUANTIZE_DIVISIONS.into_iter().enumerate(){ui.selectable_value(&mut release,Some(index as u8),format!("{beats} beats"));}
                }).response;
                response.widget_info(||egui::WidgetInfo::labeled(egui::WidgetType::ComboBox,true,format!("Deck {label}: Slip release timing")));accessibility::focus(ui,&response);help::annotate(ui,&response,HelpControl::SlipPlayback);
                if enabled!=state.slip || release!=state.slip_release {actions.push((deck as u8,enabled,release));}
                if let Some(position)=state.slip_return.or(state.slip_position) {
                    let seconds=position/f64::from(snapshot.source_sample_rate.max(1));ui.label(format!("Background return: {seconds:.3} source seconds"));
                    ui.label(if state.slip_due.is_some(){"Waiting for the selected release beat"}else{"Nested gesture active"});
                }else{ui.label(if state.slip{"Armed; no temporary gesture"}else{"Slip disabled"});}
                ui.label("Ordinary pause, explicit seeking, source replacement and safety stop cancel the background timeline. Slip starts disabled in a new app.");
            });}
        });
        self.slip.open = open;
        for (deck, enabled, division) in actions {
            self.send(Command::DeckControl {
                source: 0,
                deck,
                control: Control::SlipSettings { enabled, division },
            });
        }
    }
}

#[cfg(test)]
mod tests;
