use super::*;
use crate::engine::audio_clip::Fades;

/// Edit a bounded stereo fade envelope in native controls.
/// Takes UI, copyable settings and quarter-note span; returns whether durations, curves or automatic edges changed through pointer, keyboard or assistive input.
pub(super) fn controls(ui: &mut Ui, fades: &mut Fades, duration: f64) -> bool {
    let before = *fades;
    ui.horizontal_wrapped(|ui| {
        for (label, value) in [
            ("Fade in beats", &mut fades.fade_in),
            ("Fade out beats", &mut fades.fade_out),
        ] {
            ui.label(label);
            let response = ui.add(
                egui::DragValue::new(value)
                    .range(0.0..=duration)
                    .speed(0.01),
            );
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_label(label);
                node.set_numeric_value(*value);
                node.set_min_numeric_value(0.0);
                node.set_max_numeric_value(duration);
                node.set_numeric_value_step(0.01);
                if response.enabled() {
                    node.add_action(egui::accesskit::Action::SetValue);
                }
            });
            if response.enabled() {
                ui.input(|input| {
                    for request in input
                        .accesskit_action_requests(response.id, egui::accesskit::Action::SetValue)
                    {
                        if let Some(egui::accesskit::ActionData::NumericValue(next)) = request.data
                        {
                            if next.is_finite() {
                                *value = next.clamp(0.0, duration);
                            }
                        }
                    }
                });
            }
        }
        for (label, value) in [
            ("Fade in curve", &mut fades.in_curve),
            ("Fade out curve", &mut fades.out_curve),
        ] {
            let response = ui.add(
                egui::DragValue::new(value)
                    .range(-1.0..=1.0)
                    .speed(0.01)
                    .prefix(format!("{label} ")),
            );
            if let Some(next) =
                accessibility::numeric(ui, &response, label, *value, -1.0, 1.0, 0.01, "")
            {
                *value = next;
            }
        }
        ui.checkbox(&mut fades.automatic, "Automatic 4 ms edge fades");
    });
    ui.label("Fade lengths use quarter-note beats. Curve 0 is linear; -1 rises early and +1 rises late. Explicit fades replace the automatic edge on that side.");
    if !fades.valid(duration) {
        ui.colored_label(
            Color32::LIGHT_RED,
            "Fade lengths must fit without overlapping inside this clip.",
        );
    }
    *fades != before
}

/// Draw the audible envelope over the frequency waveform.
/// Takes painter, visible clip rectangle, envelope, musical duration, preview tempo and reverse direction; emits a bounded curve sharing the renderer's gain evaluator.
pub(super) fn paint(
    painter: &egui::Painter,
    rect: Rect,
    fades: Fades,
    duration: f64,
    beats_per_second: f64,
    reverse: bool,
) {
    if rect.width() <= 0.0 || !duration.is_finite() || duration <= 0.0 || !fades.valid(duration) {
        return;
    }
    paint_gain(painter, rect, reverse, |f| {
        let t = (f * duration).min(duration - f64::EPSILON * duration.max(1.0));
        fades.gain(t, duration, beats_per_second)
    });
}

/// Draw a shared or placement envelope over its complete clip rectangle.
/// Takes painter, rectangle, direction and a normalized-position gain reader; emits a bounded line clipped by the timeline painter.
pub(super) fn paint_gain(
    painter: &egui::Painter,
    rect: Rect,
    reverse: bool,
    gain: impl Fn(f64) -> f32,
) {
    let points = (0..=128)
        .map(|n| {
            let f = f64::from(n) / 128.0;
            let gain = gain(f);
            Pos2::new(
                rect.left() + rect.width() * if reverse { 1.0 - f as f32 } else { f as f32 },
                rect.bottom() - gain * rect.height(),
            )
        })
        .collect();
    painter.add(egui::Shape::line(
        points,
        Stroke::new(1.5_f32, Color32::WHITE),
    ));
}
