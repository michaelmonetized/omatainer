//! Manual source-time grid drafts. Only Apply submits one receipt-qualified
//! renderer edit; preview geometry never changes transport, cues or analysis.
use super::*;
use crate::engine::beatgrid::{Grid, GridEditAck, GridEditState, MAX_BPM, MIN_BPM};
use crate::engine::load_receipt::State;

pub(super) struct Editor {
    deck: usize,
    receipt: Receipt,
    draft: Option<Grid>,
    origin: String,
    tempo: String,
    error: Option<String>,
    message: String,
    pending: Option<Pending>,
}
struct Pending {
    grid: Option<Grid>,
    ack: GridEditAck,
}
impl Editor {
    #[cfg(test)]
    pub(super) fn evidence(&self) -> serde_json::Value {
        serde_json::json!({"draft": self.draft, "pending": self.pending.is_some(), "error": self.error})
    }
    fn from_snapshot(deck: usize, snap: &crate::engine::DeckSnap, receipt: Receipt) -> Self {
        let applied = receipt.preparation().and_then(|(_, p)| p.grid);
        let seed = applied.or_else(|| Grid::new(0.0, snap.source_bpm as f64).ok());
        Self {
            deck,
            receipt,
            draft: seed,
            origin: seed.map_or_else(|| "0".into(), |g| g.downbeat().to_string()),
            tempo: seed.map_or_else(String::new, |g| g.bpm().to_string()),
            error: None,
            pending: None,
            message: if applied.is_some() {
                "Editing a copy of the applied manual grid."
            } else if seed.is_some() {
                "Unverified tempo seed. Confirm the downbeat before Apply."
            } else {
                "No usable tempo hint. Enter 20–400 BPM to create a preview."
            }
            .into(),
        }
    }
    fn parse(&mut self) {
        let value = self
            .origin
            .trim()
            .parse::<f64>()
            .ok()
            .zip(self.tempo.trim().parse::<f64>().ok())
            .ok_or("Enter finite downbeat seconds and a tempo from 20 to 400 BPM")
            .and_then(|(origin, tempo)| Grid::new(origin, tempo));
        match value {
            Ok(grid) => {
                self.draft = Some(grid);
                self.error = None;
                self.message = "Preview only — Apply commits the manual grid.".into();
            }
            Err(error) => self.error = Some(error.into()),
        }
    }
    fn transform(&mut self, grid: Result<Grid, &'static str>) {
        match grid {
            Ok(grid) => {
                self.draft = Some(grid);
                self.origin = grid.downbeat().to_string();
                self.tempo = grid.bpm().to_string();
                self.error = None;
                self.message = "Preview only — Apply commits the manual grid.".into();
            }
            Err(error) => self.error = Some(error.into()),
        }
    }
}
impl App {
    pub(super) fn open_grid_editor(&mut self, deck: usize) {
        let Some(snap) = self.snap.decks.get(deck) else {
            return;
        };
        let Some(receipt) = self
            .cue_receipt(snap.receipt_key)
            .filter(|r| r.state() == State::Current && r.preparation().is_some())
        else {
            self.status = "Wait for a loaded track before editing its beatgrid".into();
            return;
        };
        if receipt.grid_is_locked() {self.status="This track’s grid is locked. Review and save an unlock in Preparation locks before editing.".into();return;}
        self.grid_editor = Some(Editor::from_snapshot(deck, snap, receipt));
    }
    pub(super) fn grid_editor_ui(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.grid_editor.take() else {
            return;
        };
        let snap = self
            .snap
            .decks
            .get(editor.deck)
            .cloned()
            .unwrap_or_default();
        if snap.receipt_key != editor.receipt.snapshot_key()
            || editor.receipt.state() != State::Current
        {
            self.status = "Beatgrid editor closed because the loaded track changed".into();
            return;
        }
        // Acquire this request's completion before observing preparation.
        // Applied is published after the renderer preparation, so the latter
        // cannot be an older value mistaken for a subsequent edit. If Pending
        // changes during this frame, confirmation simply waits one more frame.
        let request_state = editor.pending.as_ref().map(|pending| pending.ack.state());
        let Some((_, preparation)) = editor.receipt.preparation() else {
            self.grid_editor = Some(editor);
            ctx.request_repaint();
            return;
        };
        let applied = preparation.grid;
        if let Some(pending) = &editor.pending {
            match request_state.unwrap() {
                GridEditState::Applied => {
                    editor.message = if applied == pending.grid {
                        "Manual grid applied. Library durability is reported separately below."
                    } else {
                        "Grid edit was applied, then changed by a later edit or Undo. The current applied grid is shown above; this draft is retained."
                    }.into();
                    editor.pending = None;
                }
                GridEditState::Rejected => {
                    editor.message = "Renderer rejected this grid edit. The draft is retained; inspect the current grid and History before retrying.".into();
                    editor.pending = None;
                }
                GridEditState::Pending if !self.engine.cmd.is_connected() => {
                    editor.message = "Audio engine disconnected before confirming this grid request; outcome is unknown. The draft is retained.".into();
                    editor.pending = None;
                }
                GridEditState::Pending => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(30))
                }
            }
        }
        keyboard::block_for_dialog(ctx);
        let mut open = true;
        let mut close = false;
        let mut apply = false;
        // Escape owns this draft window even while editing a field; it must not
        // also reach transport or another ordinary shortcut in the ending frame.
        let escape = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
        if escape {
            return;
        }
        let title = format!("Deck {} beatgrid", (b'A' + editor.deck as u8) as char);
        let available = ctx.screen_rect().shrink(8.0);
        egui::Window::new(&title).id(egui::Id::new("beatgrid-editor"))
            .open(&mut open).collapsible(false).resizable(true)
            .default_pos(available.left_top() + Vec2::new(16.0, 32.0))
            .default_width(460.0_f32.min(available.width()))
            .min_width(220.0_f32.min(available.width()))
            .max_width(available.width()).max_height(available.height()).constrain_to(available)
            .show(ctx, |ui| {
                accessibility::scope(ui, &title, |ui| {
                    ui.label(RichText::new(&snap.title).strong());
                    ui.label(match applied {
                        Some(grid) => { let __omatainer_args = (&(grid.bpm()),&(grid.downbeat()),); crate::localization::format("Applied manual grid: {:.3} BPM · downbeat {:.6} s", &[format!("{:.3}", __omatainer_args.0), format!("{:.6}", __omatainer_args.1)]) },
                        None => tr!("Applied grid: none. Analysis BPM remains a separate hint.").into(),
                    });
                    let scroll = egui::ScrollArea::vertical().id_salt("grid-editor-body")
                        .max_height((available.height() - 180.0).max(70.0))
                        .show(ui, |ui| {
                            ui.label(tr!("Solid = applied · dashed = preview · colored markers = absolute cues"));
                            ui.label(tr!("Audio envelope: summed low/mid/high mean magnitudes, not raw sample peaks."));
                            preview(ui, &self.theme, &snap, applied, editor.draft);
                            let playhead = source_seconds(&snap);
                            // One stable parent ID even when a newly valid tempo
                            // makes the beat label appear between native events.
                            ui.push_id("grid-playhead-status", |ui| {
                                if let Some(beat) = editor.draft.and_then(|g| g.beat_at(playhead)) {
                                    ui.label(crate::localization::format("Playhead: {playhead:.6} s · preview beat {beat:.3}", &[format!("{:.6}", playhead), format!("{:.3}", beat)]));
                                }
                            });
                            ui.label(tr!("Beat 0 is the first downbeat. Earlier beats are negative pickups. Uniform four-beat bars; changing meters and tempo maps are not supported here."));
                            ui.label(tr!("Manual edits do not replace analyzed BPM or move stored cue positions."));
                            ui.add_enabled_ui(editor.pending.is_none() && !self.project.committing() && self.project.dialog_is_closed(), |ui| {
                                ui.label(tr!("Downbeat position in source seconds"));
                                let origin = ui.add(egui::TextEdit::singleline(&mut editor.origin).id_salt("grid-origin").char_limit(32).desired_width(220.0));
                                origin.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, origin.enabled(), format!("{title}: Downbeat seconds")));
                                accessibility::focus(ui, &origin); help::annotate(ui, &origin, HelpControl::GridOrigin);
                                ui.label(tr!("Stretch tempo (BPM), anchored at the downbeat"));
                                let tempo = ui.add(egui::TextEdit::singleline(&mut editor.tempo).id_salt("grid-tempo").char_limit(32).desired_width(160.0));
                                tempo.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::TextEdit, tempo.enabled(), format!("{title}: Stretch tempo BPM")));
                                accessibility::focus(ui, &tempo); help::annotate(ui, &tempo, HelpControl::GridStretch);
                                if origin.changed() || tempo.changed() { editor.parse(); }
                                if button(ui, "Set downbeat at playhead", "Set downbeat at playhead", HelpControl::GridSet, true).clicked() {
                                    let bpm = editor.tempo.trim().parse::<f64>().unwrap_or(f64::NAN);
                                    editor.transform(Grid::new(playhead, bpm));
                                }
                                ui.horizontal_wrapped(|ui| {
                                    for (label, delta) in [("Slip −10 ms", -0.010), ("Slip −1 ms", -0.001), ("Slip +1 ms", 0.001), ("Slip +10 ms", 0.010)] {
                                        if button(ui, label, label, HelpControl::GridSlip, editor.draft.is_some() && editor.error.is_none()).clicked() {
                                            editor.transform(editor.draft.unwrap().slip(delta));
                                        }
                                    }
                                });
                                ui.horizontal_wrapped(|ui| {
                                    let grid = editor.draft.filter(|_| editor.error.is_none());
                                    if button(ui, "Half tempo", "Half tempo", HelpControl::GridHalf, grid.is_some_and(|g| g.bpm() / 2.0 >= MIN_BPM)).clicked() {
                                        editor.transform(grid.unwrap().half_tempo());
                                    }
                                    if button(ui, "Double tempo", "Double tempo", HelpControl::GridDouble, grid.is_some_and(|g| g.bpm() * 2.0 <= MAX_BPM)).clicked() {
                                        editor.transform(grid.unwrap().double_tempo());
                                    }
                                    if button(ui, "Reset grid", "Reset manual grid", HelpControl::GridReset, true).clicked() {
                                        editor.draft = None; editor.error = None;
                                        editor.message = "Reset preview: Apply removes the manual grid. Cues and analysis are retained.".into();
                                    }
                                });
                            });
                            if let Some(error) = &editor.error { ui.colored_label(self.theme.red, crate::localization::format("{error}. Preview retains its last valid draft.", &[format!("{}", error)])); }
                            ui.label(&editor.message);
                            ui.label({ let __omatainer_args = (&(self.cue_storage_status(&editor.receipt).replace("cues", "grid")),); crate::localization::format("Applied grid storage: {}", &[format!("{}", __omatainer_args.0)]) });
                        });
                    accessibility::scrollbars(ui, "Beatgrid controls", &scroll);
                    ui.separator();
                    ui.horizontal_wrapped(|ui| {
                        let pending = editor.pending.is_some();
                        if button(ui, if pending { "Close grid editor" } else { "Cancel / close grid editor" }, "Cancel or close grid editor", HelpControl::GridCancel, true).clicked() { close = true; }
                        let changed = editor.draft != applied;
                        let enabled = !editor.receipt.grid_is_locked() && changed && editor.error.is_none() && !pending && !self.project.committing() && self.project.dialog_is_closed();
                        apply = button(ui, "Apply grid", "Apply grid", HelpControl::GridApply, enabled).clicked();
                    });
                });
            });
        // Resolve the window's close button and Cancel before admitting Apply,
        // including multiple accessibility actions delivered in one frame.
        if open && !close {
            if apply {
                let ack = GridEditAck::new();
                if self.submit(Command::DeckGrid {
                    deck: editor.deck as u8,
                    grid: editor.draft,
                    receipt: editor.receipt.clone(),
                    ack: ack.clone(),
                }) {
                    editor.pending = Some(Pending {
                        grid: editor.draft,
                        ack,
                    });
                    editor.message = "Grid edit queued; waiting for this track's renderer preparation. Closing cannot cancel an accepted edit.".into();
                } else {
                    editor.message = "Grid edit was not accepted. The draft remains available; playback is unchanged by this request.".into();
                }
            }
            self.grid_editor = Some(editor);
        }
    }
}
fn button(
    ui: &mut Ui,
    text: &str,
    name: &str,
    control: HelpControl,
    enabled: bool,
) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(text));
    accessibility::button(ui, &response, name, None);
    help::annotate(ui, &response, control);
    response
}
fn source_seconds(snap: &crate::engine::DeckSnap) -> f64 {
    if snap.source_sample_rate == 0 {
        0.0
    } else {
        snap.pos / snap.source_sample_rate as f64
    }
}
#[derive(Clone, Copy, Debug)]
struct Marker {
    beat: i64,
    seconds: f64,
}
fn markers(grid: Grid, start: f64, end: f64) -> impl Iterator<Item = Marker> {
    let range = grid
        .beat_at(start)
        .zip(grid.beat_at(end))
        .filter(|(a, b)| start.is_finite() && end.is_finite() && end > start && b >= a)
        .map(|(a, b)| (a.ceil() as i64, b.floor() as i64));
    let (first, step, count) = range.map_or((0, 1, 0), |(a, b)| {
        if b < a {
            return (0, 1, 0);
        }
        // Saturating float-to-integer conversion can cover all of i64 for
        // adversarial spans. Wider subtraction keeps that case bounded too.
        let count = (b as i128 - a as i128 + 1) as u128;
        let step = if count <= 128 {
            1
        } else {
            count.div_ceil(128).div_ceil(4) * 4
        };
        // Coarse views retain actual bar boundaries rather than sampling only
        // the same arbitrary off-beat residue at every decimated marker.
        let first = a as i128
            + if step > 1 {
                (-a.rem_euclid(4)).rem_euclid(4) as i128
            } else {
                0
            };
        let count = ((b as i128 - first + 1).max(0) as u128)
            .div_ceil(step)
            .min(128);
        (first, step as i128, count)
    });
    (0..count).filter_map(move |index| {
        let beat = i64::try_from(first + index as i128 * step).ok()?;
        grid.seconds_at(beat as f64)
            .filter(|seconds| *seconds >= start && *seconds <= end)
            .map(|seconds| Marker { beat, seconds })
    })
}
pub(super) fn paint_vertical_grid(
    painter: &egui::Painter,
    rect: Rect,
    theme: &Theme,
    grid: Option<Grid>,
    start: f64,
    end: f64,
) {
    let Some(grid) = grid else { return };
    for marker in markers(grid, start, end) {
        let y = rect.top() + ((marker.seconds - start) / (end - start)) as f32 * rect.height();
        let bar = marker.beat.rem_euclid(4) == 0;
        painter.hline(
            rect.x_range(),
            y,
            st(
                if bar { 1.2 } else { 0.7 },
                theme.marker(theme.fg_dim, theme.bg_darker),
            ),
        );
        if bar {
            painter.text(
                Pos2::new(rect.right() - 2.0, y),
                egui::Align2::RIGHT_BOTTOM,
                format!("bar {}", marker.beat.div_euclid(4) + 1),
                FontId::proportional(theme.text_size(8.0)),
                theme.fg_dim,
            );
        }
    }
}
fn preview(
    ui: &mut Ui,
    theme: &Theme,
    snap: &crate::engine::DeckSnap,
    applied: Option<Grid>,
    draft: Option<Grid>,
) {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(1.0), 130.0),
        Sense::hover(),
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Image,
            true,
            "Beatgrid band-magnitude preview",
        )
    });
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, theme.bg_darker);
    let duration = if snap.source_sample_rate > 0 {
        snap.frames / snap.source_sample_rate as f64
    } else {
        0.0
    };
    let start = (source_seconds(snap) - 3.5).max(0.0);
    let end = (start + 7.0).min(duration);
    if end <= start {
        return;
    }
    let x = |seconds: f64| rect.left() + ((seconds - start) / (end - start)) as f32 * rect.width();
    let points = (rect.width().ceil() as usize).clamp(1, 512);
    if !snap.peaks.is_empty() {
        for point in 0..points {
            let sec = start + point as f64 / points as f64 * (end - start);
            let index =
                ((sec / duration * snap.peaks.len() as f64) as usize).min(snap.peaks.len() - 1);
            // Stored buckets are nonnegative mean absolute low/mid/high
            // magnitudes, not a waveform min/max pair. Include every band in a
            // symmetric, bounded envelope so high-only/equal-band content is visible.
            let magnitude = snap.peaks[index]
                .iter()
                .copied()
                .filter(|v| v.is_finite())
                .map(|v| v.max(0.0))
                .sum::<f32>()
                .min(1.0)
                * 35.0;
            painter.vline(
                x(sec),
                (rect.center().y - magnitude)..=(rect.center().y + magnitude),
                st(1.0, theme.waveform(theme.fg_dim, 0.45)),
            );
        }
    }
    for (grid, is_draft) in [(applied, false), (draft, true)] {
        if let Some(grid) = grid {
            for marker in markers(grid, start, end) {
                let xpos = x(marker.seconds);
                let color = if is_draft { theme.yellow } else { theme.fg_dim };
                if is_draft {
                    for dash in 0..8 {
                        let y = rect.top() + dash as f32 * rect.height() / 8.0;
                        painter.vline(xpos, y..=(y + rect.height() / 16.0), st(1.0, color));
                    }
                } else {
                    painter.vline(xpos, rect.y_range(), st(1.0, color));
                }
                if marker.beat.rem_euclid(4) == 0 {
                    painter.text(
                        Pos2::new(
                            xpos + 2.0,
                            if is_draft {
                                rect.top() + 2.0
                            } else {
                                rect.bottom() - 2.0
                            },
                        ),
                        if is_draft {
                            egui::Align2::LEFT_TOP
                        } else {
                            egui::Align2::LEFT_BOTTOM
                        },
                        format!("bar {}", marker.beat.div_euclid(4) + 1),
                        FontId::proportional(theme.text_size(9.0)),
                        color,
                    );
                }
            }
        }
    }
    for (i, position) in snap.hotcue_positions.iter().enumerate() {
        let Some(seconds) = position
            .map(|p| p / snap.source_sample_rate as f64)
            .filter(|p| (start..=end).contains(p))
        else {
            continue;
        };
        painter.vline(
            x(seconds),
            rect.y_range(),
            st(1.5, cue_editor::color(theme, snap.cue_styles[i], i)),
        );
        painter.text(
            Pos2::new(x(seconds) + 2.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("cue {}", i + 1),
            FontId::proportional(theme.text_size(9.0)),
            theme.fg,
        );
    }
    painter.vline(
        x(source_seconds(snap)),
        rect.y_range(),
        st(2.0, theme.accent),
    );
    accessibility::status(ui, &response, "Read-only summed three-band magnitude envelope. Applied grid is solid; draft grid is dashed. Cue markers retain absolute source positions.");
    help::annotate(ui, &response, HelpControl::GridPreview);
}

#[cfg(test)]
mod tests;
