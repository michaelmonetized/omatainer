use super::*;
use crate::engine::deck_sync::{Leader, Mode};

impl App {
    /// Show deliberate sync modes and one shared leader.
    /// Takes the native deck and its published state; submits ordinary renderer commands without starting transport.
    pub(super) fn deck_sync_controls(
        &mut self,
        ui: &mut Ui,
        deck: u8,
        snap: &crate::engine::DeckSnap,
    ) {
        let following = self.snap.sync_leader.and_then(Leader::deck) != Some(usize::from(deck));
        let status = if !following && snap.sync {
            "Leader: stored tempo"
        } else if !following {
            "Leader: pitch controls tempo"
        } else if snap.sync_aligned {
            "Aligned"
        } else if matches!(snap.sync_mode, Mode::Beat | Mode::Bar) {
            "Armed; waiting for forward playback"
        } else if snap.sync_mode == Mode::Tempo
            && self.snap.sync_leader.is_some()
            && !self.snap.sync_leader_ready
        {
            "Tempo only; leader stopped"
        } else if snap.sync_mode == Mode::Tempo {
            "Tempo only"
        } else {
            "Sync off"
        };
        let settings = ui.button("Sync settings");
        egui::Popup::menu(&settings)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_max_width(360.0);
                ui.label("Sync leader");
                ui.horizontal(|ui| {
                    for leader in [Leader::Transport, Leader::DeckA, Leader::DeckB] {
                        let option = ui.selectable_label(
                            self.snap.sync_leader == Some(leader),
                            leader.label(),
                        );
                        accessibility::button(
                            ui,
                            &option,
                            &format!("Sync leader {}", leader.label()),
                            None,
                        );
                        help::annotate(ui, &option, HelpControl::DeckSyncLeader);
                        if option.clicked() {
                            self.send(Command::DeckSyncLeader(leader));
                            ui.close();
                        }
                    }
                });
                ui.separator();
                ui.label("Sync mode");
                ui.add_enabled_ui(following, |ui| {
                    ui.horizontal(|ui| {
                        for mode in Mode::ALL {
                            let option = ui.selectable_label(snap.sync_mode == mode, mode.label());
                            accessibility::button(
                                ui,
                                &option,
                                &format!("Sync mode {}", mode.label()),
                                None,
                            );
                            help::annotate(ui, &option, HelpControl::DeckSyncMode);
                            if option.clicked() {
                                self.send(Command::DeckSyncMode { deck, mode });
                                ui.close();
                            }
                        }
                    });
                    let response = ui.button("Re-arm beat");
                    accessibility::button(ui, &response, "Re-arm beat sync", None);
                    help::annotate(ui, &response, HelpControl::DeckSyncMode);
                    if response.clicked() {
                        self.send(Command::DeckSyncMode {
                            deck,
                            mode: Mode::Beat,
                        });
                        ui.close();
                    }
                });
                let response = ui.label(format!("{status} · {:.2} BPM", snap.sync_target_bpm));
                accessibility::focus(ui, &response);
                help::annotate(ui, &response, HelpControl::DeckSyncMode);
            });
        accessibility::button(ui, &settings, "Sync settings", None);
        help::annotate(ui, &settings, HelpControl::DeckSyncMode);
        settings.on_hover_text(format!("{status} · {:.2} BPM", snap.sync_target_bpm));
    }
}

#[cfg(test)]
mod tests;
