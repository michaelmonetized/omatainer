use super::*;
use crate::engine::{
    audio::routing::model::{Direction, Model},
    monitor::{Control, Source},
    CommandPort,
};

/// Select a separate headphone alias in the routing draft.
/// Takes a native UI and saved model; retains the exact stereo choice until the normal review/apply operation.
pub(super) fn output(ui: &mut Ui, model: &mut Model) {
    let before = model.monitor_output;
    let response = egui::ComboBox::from_id_salt("headphone-output-alias")
        .selected_text(
            model
                .monitor_output
                .and_then(|id| model.port(id, Direction::Output))
                .map_or("Headphone output: device default", |port| {
                    port.alias.as_str()
                }),
        )
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut model.monitor_output, None, "Device default");
            for port in model
                .ports
                .iter()
                .filter(|port| port.direction == Direction::Output && port.channels.len() == 2)
            {
                ui.selectable_value(
                    &mut model.monitor_output,
                    Some(port.id),
                    format!(
                        "Headphones: {} (outputs {}/{})",
                        port.alias,
                        port.channels[0] + 1,
                        port.channels[1] + 1
                    ),
                );
            }
        })
        .response
        .help(ui, HelpControl::Headphones);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::ComboBox,
            response.enabled(),
            "Headphone output",
        )
    });
    ui.ctx().accesskit_node_builder(response.id, |node| {
        node.set_value(
            model
                .monitor_output
                .and_then(|id| model.port(id, Direction::Output))
                .map_or("Device default", |port| port.alias.as_str()),
        )
    });
    accessibility::focus(ui, &response);
    if before != model.monitor_output {
        model.version = 2;
    }
    ui.label(tr!("Choose a separate stereo alias. Program routes cannot use its channels. Apply routing to activate this choice."));
}

/// Control the independent headphone bus.
/// Takes renderer-confirmed state and command admission; submits level, source, blend, PFL, split and finite stopped routing checks.
pub(super) fn controls(
    ui: &mut Ui,
    snapshot: &Snapshot,
    commands: &CommandPort,
    history: &undo::History,
) {
    let status = snapshot.monitor;
    ui.label(match status.channels {
        Some(pair) => format!(
            "Headphones: outputs {}/{} · {}",
            pair[0] + 1,
            pair[1] + 1,
            if status.available {
                "available"
            } else {
                "unavailable"
            }
        ),
        None => "Choose a headphone output in the routing draft".into(),
    });
    let submit = |ui: &mut Ui, control| {
        if let Err(error) = commands.send(history.wrap(Command::Monitor(control))) {
            ui.colored_label(ui.visuals().warn_fg_color, error.to_string());
        }
    };
    ui.horizontal_wrapped(|ui| {
        for (source, label) in [
            (Source::Pfl, "Selected deck cues (PFL)"),
            (Source::DeckMix, "Deck A/B mix"),
        ] {
            if ui
                .selectable_label(status.source == source, label)
                .help(ui, HelpControl::Headphones)
                .clicked()
            {
                submit(ui, Control::Source(source));
            }
        }
        for (deck, name) in [(0, "Headphones: Cue A"), (1, "Headphones: Cue B")] {
            let mut value = snapshot.decks[deck].pfl;
            if ui.checkbox(&mut value, name).changed() {
                submit(
                    ui,
                    Control::Pfl {
                        deck: deck as u8,
                        enabled: value,
                    },
                );
            }
        }
        let mut split = status.split;
        if ui
            .checkbox(&mut split, tr!("Split cue"))
            .help(ui, HelpControl::Headphones)
            .changed()
        {
            submit(ui, Control::Split(split));
        }
    });
    let mut volume = status.volume;
    if ui
        .add(egui::Slider::new(&mut volume, 0.0..=1.0).text(tr!("Headphone level")))
        .changed()
    {
        submit(ui, Control::Volume(volume));
    }
    match status.source {
        Source::Pfl => {
            let mut blend = status.blend;
            if ui
                .add(egui::Slider::new(&mut blend, 0.0..=1.0).text(tr!("Cue → master blend")))
                .changed()
            {
                submit(ui, Control::Blend(blend));
            }
        }
        Source::DeckMix => {
            let mut mix = status.mix;
            if ui
                .add(egui::Slider::new(&mut mix, 0.0..=1.0).text(tr!("Deck A → B mix")))
                .changed()
            {
                submit(ui, Control::Mix(mix));
            }
            let mut master = status.master;
            if ui.checkbox(&mut master, tr!("Monitor master")).changed() {
                submit(ui, Control::Master(master));
            }
        }
    }
    ui.label(format!(
        "Headphone meters: left {:.5} · right {:.5}",
        status.meters[0], status.meters[1]
    ));
    let stopped = !snapshot.playing
        && !snapshot.recording
        && !snapshot
            .decks
            .iter()
            .any(|deck| deck.playing || deck.touching)
        && !commands.performance().status().protected;
    ui.horizontal_wrapped(|ui| {
        for (channel, label) in [
            (0, "Check left headphone at −40 dBFS"),
            (1, "Check right headphone at −40 dBFS"),
        ] {
            if ui
                .add_enabled(status.available && stopped, egui::Button::new(label))
                .help(ui, HelpControl::Headphones)
                .clicked()
            {
                submit(ui, Control::Tone(channel));
            }
        }
        if status.tone.is_some() && ui.button(tr!("Cancel headphone check")).clicked() {
            submit(ui, Control::CancelTone);
        }
    });
    ui.label(tr!(
        "Checks stop after one second. Headphone controls leave the program mix unchanged."
    ));
}

#[cfg(test)]
mod tests;
