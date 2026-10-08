use super::*;
use crate::engine::scene::{Empty, Properties, Signature};

fn property_controls(ui: &mut Ui, properties: &mut Properties, bpm: f32, meter: Signature) {
    let mut own_tempo = properties.tempo_micros.is_some();
    if ui.checkbox(&mut own_tempo, "Use scene tempo").changed() {
        properties.tempo_micros =
            own_tempo.then(|| (60_000_000.0 / f64::from(bpm.clamp(40.0, 240.0))).round() as u32);
    }
    if let Some(micros) = &mut properties.tempo_micros {
        let mut value = 60_000_000.0 / f64::from(*micros);
        let response = ui.add(
            egui::DragValue::new(&mut value)
                .range(40.0..=240.0)
                .speed(0.1)
                .max_decimals(3)
                .prefix("Scene tempo ")
                .suffix(" BPM"),
        );
        if let Some(next) = accessibility::numeric(
            ui,
            &response,
            "Scene tempo",
            value as f32,
            40.0,
            240.0,
            0.1,
            "BPM",
        ) {
            value = f64::from(next);
        }
        *micros = (60_000_000.0 / value.clamp(40.0, 240.0)).round() as u32;
    }
    let mut own_meter = properties.meter.is_some();
    if ui
        .checkbox(&mut own_meter, "Use scene time signature")
        .changed()
    {
        properties.meter = own_meter.then_some(meter);
    }
    if let Some(meter) = &mut properties.meter {
        let response = ui.add(
            egui::DragValue::new(&mut meter.numerator)
                .range(1..=255)
                .speed(1.0)
                .prefix("Scene meter numerator "),
        );
        if let Some(next) = accessibility::numeric(
            ui,
            &response,
            "Scene meter numerator",
            f32::from(meter.numerator),
            1.0,
            255.0,
            1.0,
            "",
        ) {
            meter.numerator = next.round() as u8;
        }
        egui::ComboBox::from_id_salt("scene-meter-denominator")
            .selected_text(format!(
                "Scene meter denominator {}",
                1_u16 << meter.denominator_power
            ))
            .show_ui(ui, |ui| {
                for power in 0..=7 {
                    if ui
                        .selectable_value(
                            &mut meter.denominator_power,
                            power,
                            format!("Scene denominator {}", 1_u16 << power),
                        )
                        .clicked()
                    {
                        ui.close();
                    }
                }
            });
    }
    egui::ComboBox::from_id_salt("scene-empty-slots")
        .selected_text(match properties.empty {
            Empty::Stop => "Empty slots: stop tracks",
            Empty::Keep => "Empty slots: keep tracks playing",
        })
        .show_ui(ui, |ui| {
            if ui
                .selectable_value(
                    &mut properties.empty,
                    Empty::Stop,
                    "Stop tracks for empty slots",
                )
                .clicked()
            {
                ui.close();
            }
            if ui
                .selectable_value(
                    &mut properties.empty,
                    Empty::Keep,
                    "Keep tracks playing for empty slots",
                )
                .clicked()
            {
                ui.close();
            }
        });
    egui::ComboBox::from_id_salt("scene-launch-grid")
        .selected_text(format!("Scene launch timing: {}", properties.grid.label()))
        .show_ui(ui, |ui| {
            for grid in crate::engine::clip_launch::Grid::ALL {
                if ui
                    .selectable_value(
                        &mut properties.grid,
                        grid,
                        format!("Scene timing {}", grid.label()),
                    )
                    .clicked()
                {
                    ui.close();
                }
            }
        });
    ui.label("All participating tracks switch at the scene's launch boundary. A chosen scene tempo or meter replaces the song's tempo map from that launch; inherited timing keeps the current clock.");
}

pub(super) fn edit(
    ui: &mut Ui,
    properties: &mut Properties,
    bpm: f32,
    meter: Signature,
    busy: bool,
) -> bool {
    let mut save = false;
    egui::CollapsingHeader::new("Scene launch properties").show(ui, |ui| {
        property_controls(ui, properties, bpm, meter);
        save = ui
            .add_enabled(
                !busy && properties.valid(),
                egui::Button::new("Save scene launch properties"),
            )
            .clicked();
    });
    save
}

pub(in crate::ui) fn describe(properties: Properties) -> String {
    let tempo = properties
        .bpm()
        .map_or_else(|| "inherit tempo".into(), |bpm| format!("{bpm:.3} BPM"));
    let meter = properties.meter.map_or_else(
        || "inherit meter".into(),
        |meter| format!("{}/{}", meter.numerator, 1_u16 << meter.denominator_power),
    );
    format!(
        "{tempo}; {meter}; empty slots {}; launch {}",
        match properties.empty {
            Empty::Stop => "stop tracks",
            Empty::Keep => "keep playing",
        },
        properties.grid.label()
    )
}
