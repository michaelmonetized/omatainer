use super::*;

pub(super) enum Drag {
    Notes {
        origin: Pos2,
        pitch: u8,
        original: Vec<MidiNote>,
        resize: bool,
    },
    Marquee {
        origin: Pos2,
        extend: BTreeSet<NoteId>,
    },
}
pub(super) fn pitches(draft: &Draft) -> Vec<u8> {
    let mut used = [false; 128];
    for note in &draft.notes {
        if let Some(slot) = used.get_mut(note.pitch as usize) {
            *slot = true;
        }
    }
    let mut rows: Vec<_> = (0..=draft.view_high)
        .rev()
        .filter(|pitch| {
            let used = used[*pitch as usize] || *pitch == draft.cursor.pitch;
            let degree = (*pitch + 12 - draft.root) % 12;
            match draft.fold {
                1 => used,
                2 => used || [0, 2, 4, 5, 7, 9, 11].contains(&degree),
                3 => used || [0, 2, 3, 5, 7, 8, 10].contains(&degree),
                _ => true,
            }
        })
        .collect();
    if rows.is_empty() {
        rows.push(draft.view_high);
    }
    rows
}
fn pitch_at(pos: Pos2, body: Rect, rows: &[u8], height: f32) -> u8 {
    let row = ((pos.y - body.top()).max(0.0) / height).floor() as usize;
    rows[row.min(rows.len().saturating_sub(1))]
}
fn note_rect(note: &MidiNote, body: Rect, rows: &[u8], draft: &Draft, row_pixels: f32) -> Option<Rect> {
    let row = rows.iter().position(|p| *p == note.pitch)?;
    let left = body.left() + (note.start as f64 - draft.view_beat) as f32 * draft.beat_pixels;
    let top = body.top() + row as f32 * row_pixels;
    Some(Rect::from_min_size(
        Pos2::new(left, top + 1.0),
        Vec2::new(
            (note.len * draft.beat_pixels).max(5.0),
            row_pixels - 2.0,
        ),
    ))
}
pub(super) fn show(ui: &mut Ui, theme: &Theme, draft: &mut Draft, timing: Option<&crate::engine::midi_data::Conductor>) -> Result<(), String> {
    let row_pixels = draft.row_pixels.max(theme.text_size(10.0) * 1.25 + 4.0);
    let marker_size = theme.target_size(12.0);
    let ruler_height = theme.text_size(10.0) + marker_size * 2.0 + 12.0;
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(140.0), (290.0_f32).max(ruler_height + row_pixels * 4.0)),
        Sense::click_and_drag(),
    );
    let body = Rect::from_min_max(rect.min + Vec2::new((94.0_f32).max(theme.text_size(10.0) * 8.0), ruler_height), rect.max);
    if body.width() < 20.0 {
        return Ok(());
    }
    accessibility::button(ui, &response, "MIDI piano roll grid", None);
    help::annotate(ui, &response, HelpControl::MidiNotes);
    accessibility::status(
        ui,
        &response,
        &format!(
            "Time begins at {:.6} beats. Top pitch {}. {} {}",
            draft.view_beat,
            pitch_name(draft.view_high),
            GRIDS[draft.grid].0,
            draft.selected_label()
        ),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme.bg);
    let rows = pitches(draft);
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let shift = ui.input(|i| i.modifiers.shift);
    let mut note_hit = false;
    let mut focused = response.has_focus();
    let visible = (body.height() / row_pixels).ceil() as usize;
    for (index, pitch) in rows.iter().take(visible).enumerate() {
        let y = body.top() + index as f32 * row_pixels;
        let black = [1, 3, 6, 8, 10].contains(&(pitch % 12));
        painter.rect_filled(
            Rect::from_min_size(
                Pos2::new(body.left(), y),
                Vec2::new(body.width(), row_pixels),
            ),
            0.0,
            if black { theme.bg_dark } else { theme.bg },
        );
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(0.5_f32, theme.fg_dim),
        );
        painter.text(
            Pos2::new(rect.left() + 4.0, y + row_pixels / 2.0),
            egui::Align2::LEFT_CENTER,
            pitch_name(*pitch),
            FontId::monospace(theme.text_size(10.0)),
            theme.fg,
        );
    }
    let step = if GRIDS[draft.grid].1 > 0.0 {
        GRIDS[draft.grid].1
    } else {
        1.0
    };
    // Ruler work is bounded by visible pixels, independent of a 65536-bar clip.
    let grid_step = step * (4.0 / (step as f32 * draft.beat_pixels)).ceil().max(1.0) as f64;
    let first = (draft.view_beat / grid_step).ceil() * grid_step;
    let visible_end = draft.view_beat + body.width() as f64 / draft.beat_pixels as f64;
    let count = ((visible_end - first) / grid_step).ceil().max(0.0) as usize;
    for index in 0..=count.min(1024) {
        let beat = first + index as f64 * grid_step;
        let x = body.left() + (beat - draft.view_beat) as f32 * draft.beat_pixels;
        let (bar_number, within) = timing.map_or(((beat / 4.0).floor() as u32 + 1, beat.rem_euclid(4.0) as f32), |map| { let (bar, within, _) = map.position(beat); (bar, within) });
        let bar = within.abs() < 1e-6;
        painter.line_segment(
            [Pos2::new(x, body.top()), Pos2::new(x, body.bottom())],
            Stroke::new(if bar { 1.0_f32 } else { 0.5_f32 }, theme.fg_dim),
        );
        if bar || grid_step * draft.beat_pixels as f64 >= (45.0_f64).max(f64::from(theme.text_size(10.0) * 7.0)) {
            painter.text(
                Pos2::new(x + 2.0, rect.top() + 7.0),
                egui::Align2::LEFT_TOP,
                format!(
                    "{}:{:.2}",
                    bar_number,
                    within + 1.0
                ),
                FontId::monospace(theme.text_size(10.0)),
                theme.fg,
            );
        }
    }
    // Distinct marker lanes keep clip and loop bounds independently draggable.
    if let Some(map) = timing {
        for (beat, bar) in map.bar_boundaries(draft.view_beat, visible_end, 1024) {
            let x = body.left() + (beat - draft.view_beat) as f32 * draft.beat_pixels;
            painter.line_segment([Pos2::new(x, body.top()), Pos2::new(x, body.bottom())], Stroke::new(1.0_f32, theme.fg_dim));
            painter.text(Pos2::new(x + 2.0, rect.top() + 7.0), egui::Align2::LEFT_TOP, format!("{bar}:1"), FontId::monospace(theme.text_size(10.0)), theme.fg);
        }
    }
    for (index, label, value) in [
        (0, "Clip start marker", draft.region.start),
        (1, "Clip end marker", draft.region.end),
        (2, "Loop start marker", draft.region.loop_start),
        (3, "Loop end marker", draft.region.loop_end),
    ] {
        let x = body.left() + (value - draft.view_beat) as f32 * draft.beat_pixels;
        if x < body.left() || x > body.right() {
            continue;
        }
        let y = rect.top() + theme.text_size(10.0) + 8.0 + marker_size * if index < 2 { 0.5 } else { 1.5 };
        let marker_rect = Rect::from_center_size(Pos2::new(x, y), Vec2::splat(marker_size));
        let marker = ui.interact(
            marker_rect,
            response.id.with(("marker", index)),
            Sense::click_and_drag(),
        );
        let mut next = value;
        if marker.dragged() {
            if let Some(pos) = marker.interact_pointer_pos() {
                next = draft
                    .snap(draft.view_beat + (pos.x - body.left()) as f64 / draft.beat_pixels as f64)
                    as f64;
            }
        }
        if let Some(value) = accessibility::numeric(
            ui,
            &marker,
            label,
            value as f32,
            0.0,
            262_144.0,
            if step > 0.0 { step as f32 } else { 1.0 / 64.0 },
            " beats",
        ) {
            next = value as f64;
        }
        if next != value {
            match index {
                0 => draft.region.start = next,
                1 => draft.region.end = next,
                2 => draft.region.loop_start = next,
                _ => draft.region.loop_end = next,
            };
            draft.dirty = true;
        }
        painter.line_segment(
            [Pos2::new(x, y + 5.0), Pos2::new(x, body.bottom())],
            Stroke::new(1.0_f32, if index < 2 { theme.yellow } else { theme.cyan }),
        );
        painter.rect_filled(
            marker_rect,
            2.0,
            if index < 2 { theme.yellow } else { theme.cyan },
        );
        marker.on_hover_text(crate::localization::format("{label}: {value:.6} beats", &[format!("{}", label), format!("{:.6}", value)]));
    }
    for index in 0..draft.notes.len() {
        let note = &draft.notes[index];
        let id = note.id;
        let Some(note_rect) = note_rect(note, body, &rows, draft, row_pixels).filter(|r| r.intersects(body))
        else {
            continue;
        };
        let hit_rect = note_rect.intersect(body);
        let note_response = ui.interact(hit_rect, response.id.with(id), Sense::click_and_drag());
        note_hit |= pointer.is_some_and(|p| hit_rect.contains(p));
        focused |= note_response.has_focus();
        let selected = draft.selected.contains(&id);
        let label = format!(
            "{} · start {:.6} beats · length {:.6} · velocity {} · {}",
            pitch_name(note.pitch),
            note.start,
            note.len,
            note.vel,
            if note.muted { "muted" } else { "audible" }
        );
        accessibility::button(ui, &note_response, &label, Some(selected));
        accessibility::status(ui, &note_response, &label);
        let color = if note.muted {
            theme.fg_dim
        } else if selected {
            theme.yellow
        } else {
            theme.accent
        };
        painter
            .with_clip_rect(body)
            .rect_filled(note_rect, 2.0, color);
        if selected { painter.with_clip_rect(body).rect_stroke(note_rect.shrink(1.0), 2.0, Stroke::new(2.0_f32, theme.bg), egui::StrokeKind::Inside); }
        if note.muted { painter.with_clip_rect(body).line_segment([note_rect.left_bottom(),note_rect.right_top()],Stroke::new(1.5_f32,theme.bg)); }
        if note_rect.width() > 36.0 {
            painter.with_clip_rect(body).text(
                note_rect.left_center() + Vec2::new(3.0, 0.0),
                egui::Align2::LEFT_CENTER,
                format!("{}", note.pitch),
                FontId::monospace(theme.text_size(10.0)),
                theme.bg,
            );
        }
        if note_response.clicked() {
            draft.select(id, shift);
        }
        if note_response.drag_started() {
            if !draft.selected.contains(&id) {
                draft.select(id, shift);
            }
            if let Some(origin) = ui.input(|i| i.pointer.press_origin()) {
                let edge = (note_rect.width() * 0.25).clamp(1.0, 8.0);
                let resize = origin.x >= note_rect.right() - edge;
                draft.drag = Some(Drag::Notes {
                    origin,
                    pitch: pitch_at(origin, body, &rows, row_pixels),
                    original: draft.notes.clone(),
                    resize,
                });
            }
        }
        note_response.on_hover_text(crate::localization::format("{label}. Drag body to move; drag right edge to resize. Shift-click extends selection.", &[format!("{}", label)]));
    }
    if response.clicked() && !note_hit && pointer.is_some_and(|p| body.contains(p)) {
        let pos = pointer.unwrap();
        response.request_focus();
        draft.cursor.pitch = pitch_at(pos, body, &rows, row_pixels);
        draft.cursor.start =
            draft.snap(draft.view_beat + (pos.x - body.left()) as f64 / draft.beat_pixels as f64);
        if draft.draw {
            draft.add()?;
        } else if !shift {
            draft.selected.clear();
        }
    }
    if response.drag_started() && !draft.draw {
        if let Some(origin) = ui
            .input(|i| i.pointer.press_origin())
            .filter(|p| body.contains(*p))
        {
            draft.drag = Some(Drag::Marquee {
                origin,
                extend: if shift {
                    draft.selected.clone()
                } else {
                    BTreeSet::new()
                },
            });
        }
    }
    if let (Some(drag), Some(pos)) = (&draft.drag, pointer) {
        match drag {
            Drag::Notes {
                origin,
                pitch,
                original,
                resize,
            } => {
                let delta = draft.snap_delta((pos.x - origin.x) as f64 / draft.beat_pixels as f64);
                let transpose = pitch_at(pos, body, &rows, row_pixels) as i16 - *pitch as i16;
                let valid = original
                    .iter()
                    .filter(|n| draft.selected.contains(&n.id))
                    .all(|n| {
                        if *resize {
                            (0.0..=262_144.0).contains(&(n.len + delta))
                        } else {
                            (0.0..=262_144.0).contains(&(n.start + delta))
                                && (0..=127).contains(&(n.pitch as i16 + transpose))
                        }
                    });
                if valid {
                    for (note, old) in draft.notes.iter_mut().zip(original) {
                        if !draft.selected.contains(&note.id) {
                            continue;
                        }
                        if *resize {
                            note.len = old.len + delta;
                        } else {
                            note.start = old.start + delta;
                            note.pitch = (old.pitch as i16 + transpose) as u8;
                        }
                    }
                    draft.dirty |= delta != 0.0 || (!resize && transpose != 0);
                }
            }
            Drag::Marquee { origin, extend } => {
                let selection_rect = Rect::from_two_pos(*origin, pos).intersect(body);
                painter.rect_stroke(
                    selection_rect,
                    0.0,
                    Stroke::new(1.0_f32, theme.yellow),
                    egui::StrokeKind::Inside,
                );
                draft.selected = extend.clone();
                for note in &draft.notes {
                    if note_rect(note, body, &rows, draft, row_pixels)
                        .is_some_and(|r| r.intersects(selection_rect))
                    {
                        draft.selected.insert(note.id);
                    }
                }
            }
        }
    }
    if ui.input(|i| i.pointer.any_released()) {
        draft.drag = None;
    }
    if focused && !keyboard::text_is_focused(ui.ctx()) {
        if let Some(id) = ui.memory(|m| m.focused()) {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                )
            });
        }
        let step = if GRIDS[draft.grid].1 > 0.0 {
            GRIDS[draft.grid].1 as f32
        } else {
            1.0 / 64.0
        };
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::A)) {
            draft.selected = draft.notes.iter().map(|n| n.id).collect();
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::D)) {
            draft.duplicate()?;
        }
        if ui.input_mut(|i| {
            i.consume_key(egui::Modifiers::NONE, Key::Delete)
                || i.consume_key(egui::Modifiers::NONE, Key::Backspace)
        }) {
            draft.delete();
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::M)) {
            draft.transform(0.0, 0, 0.0, true)?;
        }
        for (key, length) in [(Key::ArrowLeft, -step), (Key::ArrowRight, step)] {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, key)) {
                draft.transform(0.0, 0, length, false)?;
            }
        }
        for (key, beat, transpose) in [
            (Key::ArrowLeft, -step, 0),
            (Key::ArrowRight, step, 0),
            (Key::ArrowDown, 0.0, -1),
            (Key::ArrowUp, 0.0, 1),
        ] {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key)) {
                draft.transform(beat, transpose, 0.0, false)?;
            }
        }
    }
    Ok(())
}
impl Draft {
    fn snap_delta(&self, value: f64) -> f32 {
        let step = GRIDS[self.grid].1;
        (if step > 0.0 {
            (value / step).round() * step
        } else {
            value
        }) as f32
    }
}
