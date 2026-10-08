use super::*;
pub(in crate::ui::piano_roll) struct Layer<'a> {
    pub notes: &'a [MidiNote],
    pub source_start: f64,
    pub shared_offset: f64,
    pub owner: &'a str,
    pub track: u8,
    pub scene: u16,
    pub editable: bool,
}
/// Paint captured clip ownership on the focused clip's time ruler.
/// Takes the existing canvas, pitch rows and exact source offsets; returns whether the pointer covers a protected companion note.
pub(super) fn overlay(
    ui: &mut Ui,
    painter: &egui::Painter,
    body: Rect,
    rows: &[u8],
    draft: &Draft,
    focus_offset: f64,
    layers: &[Layer<'_>],
    pointer: Option<Pos2>,
    row_pixels: f32,
) -> (bool, bool) {
    let mut hit = false;
    let mut origin_hit = false;
    let origin = ui.input(|i| i.pointer.press_origin());
    let mut pitch_rows = [usize::MAX; 128];
    for (row, pitch) in rows.iter().enumerate() {
        if let Some(value) = pitch_rows.get_mut(usize::from(*pitch)) {
            *value = row;
        }
    }
    let colors = [
        Color32::from_rgb(72, 163, 246),
        Color32::from_rgb(194, 129, 250),
        Color32::from_rgb(61, 200, 159),
        Color32::from_rgb(249, 166, 84),
    ];
    for layer in layers {
        let color = colors[usize::from(layer.track) % colors.len()];
        for note in layer.notes {
            let Some(&row) = pitch_rows
                .get(usize::from(note.pitch))
                .filter(|row| **row != usize::MAX)
            else {
                continue;
            };
            let start =
                note.source_start() - layer.source_start + layer.shared_offset + draft.region.start
                    - focus_offset;
            let x = body.left() + ((start - draft.view_beat) * f64::from(draft.beat_pixels)) as f32;
            let width = (note.source_duration() * f64::from(draft.beat_pixels)) as f32;
            let rect = Rect::from_min_size(
                Pos2::new(x, body.top() + row as f32 * row_pixels + 2.0),
                Vec2::new(width.max(3.0), (row_pixels - 4.0).max(3.0)),
            );
            if !rect.intersects(body) {
                continue;
            }
            let visible = rect.intersect(body);
            hit |= pointer.is_some_and(|point| visible.contains(point));
            origin_hit |= origin.is_some_and(|point| visible.contains(point));
            painter
                .with_clip_rect(body)
                .rect_filled(rect, 2.0, color.gamma_multiply(0.2));
            painter.with_clip_rect(body).rect_stroke(
                rect,
                2.0,
                Stroke::new(if layer.editable { 2.0_f32 } else { 1.0_f32 }, color),
                egui::StrokeKind::Inside,
            );
            let response = ui.interact(
                visible,
                egui::Id::new(("companion-note", layer.track, layer.scene, note.id)),
                Sense::hover(),
            );
            let label = format!(
                "{} · track {} scene {} · {} · source {:.6} beats · shared {:.6} beats · {}",
                layer.owner,
                layer.track + 1,
                layer.scene + 1,
                pitch_name(note.pitch),
                note.source_start(),
                start - draft.region.start + focus_offset,
                if layer.editable {
                    "Enabled for group edits; focus to edit here"
                } else {
                    "Protected ghost note"
                }
            );
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, false, &label));
            accessibility::status(ui, &response, &label);
            response.on_hover_text(label);
        }
    }
    (hit, origin_hit)
}
