use super::*;
use crate::engine::midi::{
    clock_input::{Config, LossPolicy, Status},
    learn::Device,
};

/// Edit one explicit song clock source without opening a device.
/// Takes transient configuration, confirmed runtime status, connected identities and guarded outputs; returns edits through normal command admission in the caller.
pub(super) fn edit_input(
    ui: &mut Ui,
    config: &mut Config,
    status: Status,
    devices: &[Device],
    outputs: &crate::engine::midi::clock::Shared,
) {
    ui.heading("External song clock");
    let selected = config
        .source
        .and_then(|source| devices.iter().find(|device| device.source == source));
    let label = match (config.source, selected) {
        (None, _) => "Internal".into(),
        (_, Some(device)) => format!("{} [{}]", device.endpoint.name, device.endpoint.id),
        (Some(source), None) => format!("Disconnected source {source}"),
    };
    let response = egui::ComboBox::from_id_salt("external-song-clock-source")
        .selected_text(label)
        .show_ui(ui, |ui| {
            let response = ui.selectable_value(&mut config.source, None, "Internal");
            accessibility::button(
                ui,
                &response,
                "Clock source Internal",
                Some(config.source.is_none()),
            );
            if response.clicked() {
                ui.close();
            }
            for device in devices {
                let label = format!("{} [{}]", device.endpoint.name, device.endpoint.id);
                let guarded =
                    outputs.guards_transport_port(&device.endpoint.name, &device.endpoint.id);
                let response = ui.add_enabled(
                    !guarded,
                    egui::Button::new(&label).selected(config.source == Some(device.source)),
                );
                accessibility::button(
                    ui,
                    &response,
                    &format!("Clock source {label}"),
                    Some(config.source == Some(device.source)),
                );
                if guarded {
                    response.clone().on_hover_text(
                        "This port receives Omatainer clock output; following its echo is refused.",
                    );
                }
                if response.clicked() {
                    config.source = Some(device.source);
                    ui.close();
                }
            }
        })
        .response;
    accessibility::button(ui, &response, "External clock source", None);
    help::annotate(ui, &response, HelpControl::MidiClockInput);
    ui.checkbox(
        &mut config.follow_transport,
        "Follow MIDI Start, Continue and Stop",
    )
    .help(ui, HelpControl::MidiClockInput);
    ui.horizontal_wrapped(|ui| {
        ui.label("If clock is lost:");
        for (value, label) in [
            (LossPolicy::Freewheel, "Keep playing at last tempo"),
            (LossPolicy::Stop, "Stop song and release clip notes"),
        ] {
            let response = ui.selectable_value(&mut config.loss, value, label);
            accessibility::button(ui, &response, label, Some(config.loss == value));
        }
    });
    let mut timeout = f32::from(config.timeout_ms);
    let response = ui.add(
        egui::DragValue::new(&mut timeout)
            .range(250.0..=2000.0)
            .speed(50.0)
            .prefix("Clock loss after ")
            .suffix(" ms"),
    );
    if let Some(value) = accessibility::numeric(
        ui,
        &response,
        "External clock loss deadline",
        timeout,
        250.0,
        2000.0,
        50.0,
        "ms",
    ) {
        timeout = value;
    }
    config.timeout_ms = timeout.round() as u16;
    help::annotate(ui, &response, HelpControl::MidiClockInput);
    ui.label("Follow 40–240 BPM in this session. Local song Play/Stop, seeking or tempo edits return to Internal. Unsynced DJ decks keep their own transport; choose Transport as their sync leader to follow the song.");
    let state = if status.config.source.is_none() {
        "Internal"
    } else if status.awaiting_tick {
        "Waiting for the next MIDI clock"
    } else if status.needs_transport {
        "Song stopped; send a fresh Start or Continue"
    } else if status.locked {
        "Following selected source"
    } else {
        "Estimating selected source"
    };
    let response = ui.label(format!("{state} · {} accepted clocks · {} inferred missing · {} stale · {} ignored · {} input overflows", status.accepted_ticks, status.inferred_missing_ticks, status.stale, status.ignored, status.overflow));
    accessibility::status(ui, &response, "Confirmed external clock input status");
    if let Some(bpm) = status.estimated_bpm {
        ui.label(format!(
            "{bpm:.3} BPM · jitter {:.3} ms · phase correction {:.5} beats · {} reacquisitions",
            status.jitter_ms, status.phase_error_beats, status.reacquisitions
        ));
    }
    if let Some(bpm) = status.unsupported_bpm {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!("Clock estimate {bpm:.3} BPM; the supported song tempo is 40–240 BPM."),
        );
    }
    if let Some(reason) = status.lost {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            format!(
                "Clock lost: {reason:?}. {}",
                if status.config.loss == LossPolicy::Stop {
                    "Song stopped; send a fresh Start or Continue to resume."
                } else {
                    "Last estimated tempo continues; fresh clocks reacquire smoothly."
                }
            ),
        );
    }
    if status.config.source.is_some() && !status.backend_timing {
        ui.label(
            "Audio playback time is unavailable; following uses a one-buffer callback estimate.",
        );
    }
}

#[cfg(test)]
mod tests;
