mod input;
use super::*;
use crate::engine::midi::{
    clock::{Config, Status},
    routing::Endpoint,
};

pub(super) fn edit(ui: &mut Ui, config: &mut Config, status: Option<&Status>) {
    ui.separator();
    egui::CollapsingHeader::new("MIDI clock output").show(ui, |ui| edit_open(ui, config, status));
}

fn edit_open(ui: &mut Ui, config: &mut Config, status: Option<&Status>) {
    ui.checkbox(&mut config.enabled, "Send song MIDI clock")
        .help(ui, HelpControl::MidiClockOutput);
    ui.label("Clock follows the song/session audio transport at 24 pulses per quarter note, including its tempo changes. DJ decks have separate transports. Only the selected outputs receive clock and Start/Continue/Stop.");
    let mut compensation = config.compensation_ms as f32;
    let response = ui.add(
        egui::DragValue::new(&mut compensation)
            .range(-500.0..=500.0)
            .speed(1.0)
            .prefix("Clock compensation ")
            .suffix(" ms"),
    );
    if let Some(next) = accessibility::numeric(
        ui,
        &response,
        "MIDI clock compensation",
        compensation,
        -500.0,
        500.0,
        1.0,
        "ms",
    ) {
        compensation = next;
    }
    config.compensation_ms = compensation.round() as i32;
    help::annotate(ui, &response, HelpControl::MidiClockOutput);
    ui.label("Positive compensation delays MIDI. Negative compensation advances it within the audio output's available lookahead. Timing beyond that lookahead is reported as late. Stop the song before changing enabled clock outputs or compensation.");
    let outputs = status.map_or(&[][..], |status| status.outputs.as_slice());
    if let Some(status) = status {
        if status.truncated {
            ui.label("Output list is truncated; exact names and IDs remain available.");
        }
    }
    let mut remove = None;
    for (index, port) in config.ports.iter_mut().enumerate() {
        ui.push_id(("clock-port", index), |ui| {
            egui::CollapsingHeader::new(format!("Clock output {}", index + 1))
                .default_open(true)
                .show(ui, |ui| {
                    midi_routing::port(ui, &format!("Clock output {}", index + 1), port, outputs);
                    if ui
                        .button(format!("Remove clock output {}", index + 1))
                        .help(ui, HelpControl::MidiClockOutput)
                        .clicked()
                    {
                        remove = Some(index);
                    }
                });
        });
    }
    if let Some(index) = remove {
        config.ports.remove(index);
    }
    if ui
        .add_enabled(
            config.ports.len() < 8,
            egui::Button::new("Add clock output"),
        )
        .help(ui, HelpControl::MidiClockOutput)
        .clicked()
    {
        config.ports.push(Endpoint {
            name: String::new(),
            id: None,
        });
    }
    if let Err(error) = config.validate() {
        ui.colored_label(ui.visuals().warn_fg_color, error);
    }
}

impl App {
    pub(super) fn midi_clock_status_ui(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        ui.separator();
        ui.heading("Song clock sync");
        let status=self.snap.midi_clock_input;
        let mut config=status.config;
        let devices=self.engine.cmd.midi_learn().view().devices;
        input::edit_input(ui,&mut config,status,&devices,self.engine.cmd.clock_output());
        if config!=status.config {self.send(Command::ClockFollow(config));}
        let counters = self.engine.cmd.clock_output().counters();
        let state = if counters.running {
            "Song running; clock scheduled"
        } else if counters.enabled {
            "Armed; song stopped"
        } else {
            "Off"
        };
        ui.label(format!(
            "{state} · {} clocks scheduled · {} accepted by outputs",
            counters.scheduled, counters.sent
        ));
        ui.label(format!(
            "{} clocks more than 1 ms late · worst observed delay {:.3} ms · {} queue overflows",
            counters.late,
            counters.max_late_ns as f64 / 1_000_000.0,
            counters.overflow
        ));
        ui.label(if counters.backend_timing {
            "Clock uses the audio backend's scheduled playback time."
        } else {
            "Audio playback time is unavailable; clock uses a one-buffer callback estimate."
        });
        if let Some(error) = counters.error {
            ui.colored_label(ui.visuals().warn_fg_color, error);
        }
        if let Some(status) = self.engine.midi.clock_status() {
            ui.label("Clock destinations:");
            if status.actual.is_empty() {
                ui.label("None");
            }
            for port in &status.actual {
                ui.label(
                    port.id
                        .as_ref()
                        .map_or_else(|| port.name.clone(), |id| format!("{} [{id}]", port.name)),
                );
            }
            if let Some(error) = &status.error {
                ui.colored_label(ui.visuals().warn_fg_color, error);
            }
            if status.applied.as_ref() != &self.settings.profile().midi_clock {
                ui.label("Saved clock settings differ from the applied clock policy.");
            }
            if status.pending {
                ui.label("Clock output change pending.");
                if ui
                    .button("Cancel pending MIDI clock change")
                    .help(ui, HelpControl::MidiClockOutput)
                    .clicked()
                {
                    self.engine.midi.cancel_clock();
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(40));
            }
            if ui
                .add_enabled(!status.pending, egui::Button::new("Retry saved MIDI clock"))
                .help(ui, HelpControl::MidiClockOutput)
                .clicked()
            {
                self.settings.message = match self
                    .engine
                    .midi
                    .configure_clock(self.settings.profile().midi_clock.clone())
                {
                    Ok(()) => "MIDI clock output change queued".into(),
                    Err(error) => error,
                };
            }
        } else {
            ui.label("MIDI clock output owner unavailable in this session; saved settings apply after restart.");
        }
        if ui
            .button("Edit clock outputs in Preferences")
            .help(ui, HelpControl::MidiClockOutput)
            .clicked()
        {
            self.settings.open = true;
        }
        if counters.enabled {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}

#[cfg(test)]
mod tests;
