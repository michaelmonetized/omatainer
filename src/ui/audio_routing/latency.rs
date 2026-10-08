use super::*;
use crate::engine::audio::routing::latency::{preview, Status};

/// Edit retained latency reports and preview their exact causal delays.
/// Takes the native panel, draft, stable session, output rate and current status; changes only the reviewed draft without audio devices or histories.
pub(super) fn controls(
    ui: &mut Ui,
    model: &mut Model,
    layout: &Layout,
    rate: u32,
    current: Status,
) {
    let mut enabled = model.latency.is_some();
    if ui
        .checkbox(&mut enabled, "Compensate reported latency")
        .changed()
    {
        model.latency = enabled.then(LatencyConfiguration::default);
    }
    ui.label("Enter measured source/output offsets and latency actually reported by a processor. These values align other paths; they do not add lookahead to a processor or measure connected hardware.");
    ui.label("The reserve covers retained default sends and declared input offsets, including sources that are currently silent.");
    let labels = model.clone();
    let mut groups = endpoints(&labels, layout, true);
    for group in endpoints(&labels, layout, false) {
        if !groups.contains(&group) {
            groups.push(group);
        }
    }
    if let Some(config) = &mut model.latency {
        if let Some(value) = number(
            ui,
            "Delay reserve (ms)",
            config.reserve_micros as f32 / 1000.0,
            1.0,
            2000.0,
            1.0,
        ) {
            config.reserve_micros = (value * 1000.0).round() as u32;
        }
        ui.checkbox(
            &mut config.low_latency_monitor,
            "Immediate headphone monitoring",
        );
        if config.low_latency_monitor {
            ui.colored_label(ui.visuals().warn_fg_color, "Headphones bypass the added cue delay. Their cue and program sources can be out of time. Program outputs and recording retain compensation.");
        }
        let mut remove = None;
        for (index, report) in config.reports.iter_mut().enumerate() {
            ui.push_id(index, |ui| {
                ui.horizontal(|ui| {
                    choose(
                        ui,
                        "Latency source",
                        &mut report.group,
                        &groups,
                        &labels,
                        layout,
                    );
                    if ui.button("Remove latency report").clicked() {
                        remove = Some(index);
                    }
                });
                let name = endpoint(report.group, &labels, layout);
                if let Some(value) = number(
                    ui,
                    &format!("{name}: Measured offset (ms)"),
                    report.external_micros as f32 / 1000.0,
                    0.0,
                    config.reserve_micros as f32 / 1000.0,
                    0.1,
                ) {
                    report.external_micros = (value * 1000.0).round() as u32;
                }
                if matches!(
                    report.group,
                    Group::Input(_) | Group::Output(_) | Group::Record(_)
                ) {
                    report.processing_micros = 0;
                } else if let Some(value) = number(
                    ui,
                    &format!("{name}: Reported processing delay (ms)"),
                    report.processing_micros as f32 / 1000.0,
                    0.0,
                    config.reserve_micros as f32 / 1000.0,
                    0.1,
                ) {
                    report.processing_micros = (value * 1000.0).round() as u32;
                }
            });
        }
        if let Some(index) = remove {
            config.reports.remove(index);
        }
        let next = groups
            .iter()
            .find(|group| !config.reports.iter().any(|report| report.group == **group))
            .copied();
        if ui
            .add_enabled(
                next.is_some() && config.reports.len() < 1024,
                egui::Button::new("Add latency report"),
            )
            .clicked()
        {
            config.reports.push(LatencyReport {
                group: next.unwrap(),
                external_micros: 0,
                processing_micros: 0,
            });
        }
    }
    match preview(model, layout, rate) {
        Ok((status, taps)) if status.enabled => {
            ui.label(format!("Draft program delay: {} frames ({:.3} ms) · reserve: {} frames · history: {:.2} MiB", status.program_frames, f64::from(status.program_frames) * 1000.0 / f64::from(rate), status.reserve_frames, status.history_bytes as f64 / 1048576.0));
            ui.collapsing("Delays at each tap", |ui| {
                for (group, taps) in taps {
                    ui.label(format!(
                        "{}: pre FX {} · post FX {} · post mixer {} frames",
                        endpoint(group, model, layout),
                        taps[0],
                        taps[1],
                        taps[2]
                    ));
                }
            });
        }
        Ok(_) => {
            ui.label("Draft latency compensation is disabled.");
        }
        Err(error) => {
            ui.colored_label(ui.visuals().warn_fg_color, error);
        }
    }
    if current.enabled {
        ui.label(format!("Running program delay: {} frames · headphone delay: {} frames · warming: {} frames · transition: {} frames", current.program_frames, current.monitor_frames, current.priming_frames, current.transition_frames));
    }
}
