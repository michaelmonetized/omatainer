//! Reviewed gain drafts tied to one loaded source receipt.
use super::*;
use crate::engine::{
    beatgrid::{GridEditAck, GridEditState},
    load_receipt::State,
};
use crate::track_gain::{Policy, Resolved};

pub(super) struct Editor {
    deck: usize,
    receipt: Receipt,
    mode: u8,
    manual_db: f32,
    target_dbfs: f32,
    peak_dbfs: f32,
    pending: Option<GridEditAck>,
    message: String,
}
impl Editor {
    fn policy(&self) -> Policy {
        match self.mode {
            1 => Policy::Manual { db: self.manual_db },
            2 => Policy::Auto {
                target_dbfs: self.target_dbfs,
                peak_dbfs: self.peak_dbfs,
            },
            _ => Policy::Off,
        }
    }
}
impl App {
    pub(super) fn open_track_gain(&mut self, deck: usize) {
        let Some(snap) = self.snap.decks.get(deck) else {
            return;
        };
        let Some(receipt) = self
            .cue_receipt(snap.receipt_key)
            .filter(|receipt| receipt.state() == State::Current)
        else {
            self.status = tr!("Wait for a loaded track before reviewing source gain").into();
            return;
        };
        let policy = receipt
            .preparation()
            .map(|(_, preparation)| preparation.source_gain)
            .unwrap_or_default();
        let mut editor = Editor {
            deck,
            receipt,
            mode: 0,
            manual_db: snap.source_gain_db,
            target_dbfs: -18.0,
            peak_dbfs: -3.0,
            pending: None,
            message: String::new(),
        };
        match policy {
            Policy::Off => {}
            Policy::Manual { db } => {
                editor.mode = 1;
                editor.manual_db = db;
            }
            Policy::Auto {
                target_dbfs,
                peak_dbfs,
            } => {
                editor.mode = 2;
                editor.target_dbfs = target_dbfs;
                editor.peak_dbfs = peak_dbfs;
            }
        }
        self.track_gain = Some(editor);
    }
    pub(super) fn track_gain_ui(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.track_gain.take() else {
            return;
        };
        let snap = self
            .snap
            .decks
            .get(editor.deck)
            .cloned()
            .unwrap_or_default();
        if snap.receipt_key != editor.receipt.snapshot_key()
            || editor.receipt.state() != State::Current
        {
            self.status = tr!("Track gain review closed because the loaded source changed").into();
            return;
        }
        if let Some(ack) = &editor.pending {
            match ack.state() {
                GridEditState::Applied => {
                    editor.message = tr!("Source gain applied. Library save status is reported separately. The fader stayed unchanged.").into();
                    editor.pending = None;
                }
                GridEditState::Rejected => {
                    editor.message = tr!("Source gain was refused. Stop and settle this track before retrying; the draft remains available.").into();
                    editor.pending = None;
                }
                GridEditState::Pending if !self.engine.cmd.is_connected() => {
                    editor.message = tr!("Audio engine disconnected before gain confirmation; the outcome is unknown.").into();
                    editor.pending = None;
                }
                GridEditState::Pending => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30))
                }
            }
        }
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        let mut close = ctx.input(|input| input.key_pressed(Key::Escape));
        let mut apply = false;
        egui::Window::new(tr!("Track source gain")).id(egui::Id::new("track-gain-review")).open(&mut open).resizable(true).vscroll(true).default_width(530.0).show(ctx, |ui| {
            ui.label(&snap.title);
            ui.label(crate::localization::format("Current source trim: {} dB; current fader: {}%", &[crate::localization::number(f64::from(snap.source_gain_db), 2), crate::localization::number(f64::from(snap.gain) * 100.0, 1)]));
            let level = editor.receipt.source_level();
            if let Some(level) = level { ui.label(crate::track_gain::description(level)); }
            else { ui.label(tr!("No current decoded level; reload this source before using Auto")); }
            ui.horizontal_wrapped(|ui| {
                for (mode, label) in [(0, "Source gain off"), (1, "Manual source gain"), (2, "Auto source gain")] {
                    let response = ui.selectable_value(&mut editor.mode, mode, crate::localization::text_dynamic(label));
                    accessibility::button(ui, &response, label, None);
                    help::annotate(ui, &response, HelpControl::SourceGain);
                }
            });
            ui.add_enabled_ui(editor.pending.is_none(), |ui| {
                if editor.mode == 1 {
                    let response = ui.add(egui::Slider::new(&mut editor.manual_db, -120.0..=12.0).step_by(0.1).text(tr!("Manual source trim (dB)")).custom_formatter(|value, _| crate::localization::number(value, 1)).custom_parser(crate::localization::parse_number));
                    help::annotate(ui, &response, HelpControl::SourceGain);
                } else if editor.mode == 2 {
                    let response = ui.add(egui::Slider::new(&mut editor.target_dbfs, -40.0..=-6.0).step_by(0.1).text(tr!("Target RMS (dBFS)")).custom_formatter(|value, _| crate::localization::number(value, 1)).custom_parser(crate::localization::parse_number));
                    help::annotate(ui, &response, HelpControl::SourceGain);
                    let response = ui.add(egui::Slider::new(&mut editor.peak_dbfs, -12.0..=-1.0).step_by(0.1).text(tr!("Source sample-peak limit (dBFS)")).custom_formatter(|value, _| crate::localization::number(value, 1)).custom_parser(crate::localization::parse_number));
                    help::annotate(ui, &response, HelpControl::SourceGain);
                }
            });
            let resolved = Resolved::prepare(editor.policy(), level);
            match resolved {
                Ok(gain) => {
                    ui.label(crate::localization::format("Reviewed source trim: {} dB", &[crate::localization::number(f64::from(gain.db()), 2)]));
                    if let Some(peak) = level.and_then(|level| level.peak_dbfs) { ui.label(crate::localization::format("Estimated source sample peak after trim: {} dBFS", &[crate::localization::number(peak + f64::from(gain.db()), 2)])); }
                },
                Err(error) => { ui.colored_label(self.theme.red, crate::localization::text_dynamic(error)); },
            }
            ui.label(tr!("The peak limit applies before the fader, EQ and effects. It cannot repair clipped audio. Analysis does not change the applied trim."));
            ui.label(tr!("Stop this track and let its output settle before Apply. The saved policy is recalled on a deliberate load using that load’s decoded level."));
            ui.horizontal_wrapped(|ui| {
                let allowed = editor.pending.is_none() && resolved.is_ok() && !snap.source_gain_active && !self.snap.performance.protected;
                let response = ui.add_enabled(allowed, egui::Button::new(tr!("Apply source gain")));
                accessibility::button(ui, &response, "Apply source gain", None);
                help::annotate(ui, &response, HelpControl::SourceGain);
                apply = response.clicked();
                let response = ui.button(tr!("Cancel source gain review"));
                accessibility::button(ui, &response, "Cancel source gain review", None);
                close |= response.clicked();
            });
            if !editor.message.is_empty() { ui.label(&editor.message); }
        });
        if open && !close {
            if apply {
                let gain =
                    Resolved::prepare(editor.policy(), editor.receipt.source_level()).unwrap();
                let ack = GridEditAck::new();
                if self.submit(Command::DeckSourceGain {
                    deck: editor.deck as u8,
                    gain,
                    receipt: editor.receipt.clone(),
                    ack: ack.clone(),
                }) {
                    editor.pending = Some(ack);
                    editor.message = tr!("Source gain queued; waiting for audio confirmation. Closing does not cancel an accepted edit.").into();
                } else {
                    editor.message =
                        tr!("Gain edit was not accepted; the draft remains available.").into();
                }
            }
            self.track_gain = Some(editor);
        }
    }
}

#[cfg(test)]
mod tests;
