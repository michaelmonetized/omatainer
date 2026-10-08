use super::*;
use crate::engine::input_monitor::Mode;

impl App {
    /// Edit the selected track's input policy.
    /// Takes its native panel and retained slot; submits identity-scoped monitoring, arm and cue controls.
    pub(super) fn input_monitoring_controls(&mut self, ui: &mut Ui, slot: usize) {
        let Some(track) = self.snap.tracks.get(slot) else {
            return;
        };
        let mode = track.input_monitor;
        let mut armed = track.armed;
        let mut pfl = track.pfl;
        let enabled = track.input_enabled;
        ui.separator();
        ui.label(tr!("Audio input monitoring"));
        ui.horizontal_wrapped(|ui| {
            for choice in [Mode::In, Mode::Auto, Mode::Off] {
                let response = ui.selectable_label(mode == Some(choice), choice.label());
                accessibility::button(
                    ui,
                    &response,
                    &format!("Input monitoring: {}", choice.label()),
                    Some(mode == Some(choice)),
                );
                help::annotate(ui, &response, HelpControl::InputMonitoring);
                if response.clicked() {
                    self.send(Command::TrackMonitor {
                        track: slot as u8,
                        mode: choice,
                    });
                }
            }
            let response = ui.checkbox(&mut armed, tr!("Arm input track"));
            help::annotate(ui, &response, HelpControl::InputMonitoring);
            if response.changed() {
                self.send(Command::TrackArm {
                    track: slot as u8,
                    value: armed,
                });
            }
            let response = ui.checkbox(&mut pfl, tr!("Cue input track"));
            help::annotate(ui, &response, HelpControl::Headphones);
            if response.changed() {
                self.send(Command::TrackPfl {
                    track: slot as u8,
                    value: pfl,
                });
            }
        });
        if mode.is_none() {
            ui.label(tr!(
                "Retained legacy mix. Choose a monitoring mode to replace it."
            ));
        }
        ui.label(if enabled {
            tr!("Routed input requested; availability and gaps appear in Audio routing.")
        } else {
            tr!("Routed input monitoring is off.")
        });
        ui.label(tr!("Choose input and record sources in Audio routing. Off supports direct hardware monitoring. In suppresses audio clips. Auto monitors an armed track while no clip is sounding or a recording is active. The input buffer cushion and current effects add latency; this path adds no compensation delay."));
    }
}

#[cfg(test)]
mod tests;
