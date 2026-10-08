use super::*;
use crate::engine::midi_tools::composition::{Articulation, Voicing};

fn integer(ui: &mut Ui, label: &str, value: &mut i16, low: i16, high: i16) {
    let mut number_value = f64::from(*value);
    if number(
        ui,
        label,
        &mut number_value,
        f64::from(low),
        f64::from(high),
    ) {
        *value = number_value.round() as i16;
    }
}
fn register(ui: &mut Ui, params: &mut Parameters) {
    ui.horizontal_wrapped(|ui| {
        value(
            ui,
            "Generator lowest pitch",
            &mut params.composition.register[0],
            0,
            127,
        );
        value(
            ui,
            "Generator highest pitch",
            &mut params.composition.register[1],
            0,
            127,
        );
        value(
            ui,
            "Generator MIDI channel",
            &mut params.composition.channel,
            0,
            15,
        );
        value(
            ui,
            "Generator minimum velocity",
            &mut params.velocity[0],
            1,
            127,
        );
        value(
            ui,
            "Generator maximum velocity",
            &mut params.velocity[1],
            1,
            127,
        );
    });
}

/// Show reversible composition controls.
/// Takes native UI, current transformation settings and theme; returns whether the user requested a seeded melody reroll.
pub(super) fn show(ui: &mut Ui, tools: &mut Tools, theme: &Theme) -> bool {
    let mut reroll = false;
    let params = &mut tools.params;
    check(
        ui,
        "Replace selected source notes",
        &mut params.composition.replace,
    );
    if params.kind != Kind::Articulate {
        ui.horizontal_wrapped(|ui| {
            number(
                ui,
                "Generator start beat",
                &mut params.composition.start,
                0.0,
                262_144.0,
            );
            number(
                ui,
                "Generator length beats",
                &mut params.composition.length,
                1.0 / 1024.0,
                262_144.0,
            );
        });
        register(ui, params);
        check(
            ui,
            "Allow borrowed or chromatic generator notes",
            &mut params.include_chromatic,
        );
    }
    ui.horizontal_wrapped(|ui| {
        number(
            ui,
            "Generator gate minimum",
            &mut params.composition.gate[0],
            0.01,
            1.0,
        );
        number(
            ui,
            "Generator gate maximum",
            &mut params.composition.gate[1],
            0.01,
            1.0,
        );
    });
    match params.kind {
        Kind::Chords => {
            ui.horizontal_wrapped(|ui| {
                value(
                    ui,
                    "Chord voice count",
                    &mut params.composition.voices,
                    3,
                    4,
                );
                value(
                    ui,
                    "Chord octave spread",
                    &mut params.composition.spread,
                    0,
                    3,
                );
                check(
                    ui,
                    "Minimize voice movement without crossing",
                    &mut params.composition.voice_leading,
                );
                let response = egui::ComboBox::from_id_salt("chord-voicing")
                    .selected_text(match params.composition.voicing {
                        Voicing::Closed => "Closed",
                        Voicing::Open => "Open",
                        Voicing::DropTwo => "Drop two",
                    })
                    .show_ui(ui, |ui| {
                        for (voicing, label) in [
                            (Voicing::Closed, "Closed"),
                            (Voicing::Open, "Open"),
                            (Voicing::DropTwo, "Drop two"),
                        ] {
                            ui.selectable_value(&mut params.composition.voicing, voicing, label);
                        }
                    });
                response.response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Chord voicing")
                });
            });
            let list = egui::ScrollArea::vertical()
                .id_salt("chord-progression-rows")
                .max_height(220.0)
                .show_rows(ui, 30.0, params.composition.chords.len(), |ui, range| {
                    for index in range {
                        accessibility::scope(ui, &format!("Chord {}", index + 1), |ui| {
                            ui.horizontal_wrapped(|ui| {
                                let chord = &mut params.composition.chords[index];
                                ui.label(format!("Chord {}", index + 1));
                                integer(ui, "Chord degree", &mut chord.degree, -128, 128);
                                value(
                                    ui,
                                    "Chord inversion",
                                    &mut chord.inversion,
                                    0,
                                    params.composition.voices - 1,
                                );
                                let mut alteration = i16::from(chord.alteration);
                                integer(ui, "Borrowed semitones", &mut alteration, -11, 11);
                                chord.alteration = alteration as i8;
                            })
                        });
                    }
                });
            accessibility::scrollbars(ui, "Chord progression", &list);
            ui.horizontal_wrapped(|ui| {
                if enabled(
                    ui,
                    "Add progression chord",
                    params.composition.chords.len() < 64,
                )
                .clicked()
                {
                    params
                        .composition
                        .chords
                        .push(*params.composition.chords.last().unwrap());
                }
                if enabled(
                    ui,
                    "Remove last progression chord",
                    params.composition.chords.len() > 1,
                )
                .clicked()
                {
                    params.composition.chords.pop();
                }
            });
            ui.label("Degrees start at zero for the tonic. Each row occupies an equal part of the chosen length; inversion, voicing, spread and the register shape its notes. Voice leading searches ordered voices and keeps the smallest total pitch movement. Chromatic alterations need explicit permission.");
        }
        Kind::Melody => {
            ui.horizontal_wrapped(|ui| {
                number(ui, "Melody step beats", &mut params.grid, 1.0 / 1024.0, 4.0);
                number(
                    ui,
                    "Melody note density",
                    &mut params.composition.density,
                    0.0,
                    1.0,
                );
                number(
                    ui,
                    "Melody pitch variation",
                    &mut params.composition.pitch_variation,
                    0.0,
                    1.0,
                );
            });
            let label = ui.label("Melody seed");
            let response = ui
                .add(
                    egui::TextEdit::singleline(&mut tools.seed)
                        .char_limit(20)
                        .desired_width(180.0),
                )
                .labelled_by(label.id);
            response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, true, "Transformation seed")
            });
            reroll = enabled(ui, "Reroll melodic candidate", !tools.busy()).clicked();
            curve(ui, tools, theme);
            ui.label("Draw a pitch contour across the chosen length. The saved scale and pitch register bound the notes; density, duration, pitch variation and velocity use the explicit seed. Reroll regenerates from the captured original, so candidates never accumulate hidden notes.");
        }
        Kind::Articulate => {
            let response = egui::ComboBox::from_id_salt("articulation-kind")
                .selected_text(match params.composition.articulation {
                    Articulation::Arpeggio => "Arpeggio",
                    Articulation::Strum => "Strum",
                    Articulation::Grace => "Grace notes",
                    Articulation::Flam => "Flam",
                    Articulation::Glissando => "Glissando",
                    Articulation::Repeat => "Repeated chops",
                    Articulation::Legato => "Legato lengths",
                })
                .show_ui(ui, |ui| {
                    for (kind, label) in [
                        (Articulation::Arpeggio, "Arpeggio"),
                        (Articulation::Strum, "Strum"),
                        (Articulation::Grace, "Grace notes"),
                        (Articulation::Flam, "Flam"),
                        (Articulation::Glissando, "Glissando"),
                        (Articulation::Repeat, "Repeated chops"),
                        (Articulation::Legato, "Legato lengths"),
                    ] {
                        ui.selectable_value(&mut params.composition.articulation, kind, label);
                    }
                });
            response.response.widget_info(|| {
                egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, "Articulation type")
            });
            ui.horizontal_wrapped(|ui| {
                number(
                    ui,
                    "Articulation step beats",
                    &mut params.grid,
                    1.0 / 1024.0,
                    4.0,
                );
                check(
                    ui,
                    "Descending articulation",
                    &mut params.composition.descending,
                );
                let mut interval = i16::from(params.composition.interval);
                integer(ui, "Ornament semitone interval", &mut interval, -24, 24);
                params.composition.interval = interval as i8;
                value(
                    ui,
                    "Ornament grace velocity",
                    &mut params.velocity[0],
                    1,
                    127,
                );
            });
            ui.label("Select notes first. Arpeggios cycle each onset chord by pitch order; strums delay its voices. Grace notes precede the source, flams add a quiet first hit, glissandi walk to the chosen interval, and repeated chops use the step and gate. Legato extends toward the next selected onset on that channel. Same-voice overlaps, collapsed ticks or an out-of-range note refuse the complete preview.");
        }
        _ => {}
    }
    reroll
}

#[cfg(test)]
mod tests;
