use super::*;
use crate::engine::{audible::Position, DeckSnap};

use crate::preferences::waveforms::{Config, Zoom};

const SPECTRUM_COLORS: [Color32; 8] = [
    Color32::from_rgb(255, 66, 82), Color32::from_rgb(255, 151, 49),
    Color32::from_rgb(255, 224, 66), Color32::from_rgb(162, 234, 64),
    Color32::from_rgb(49, 218, 151), Color32::from_rgb(48, 209, 246),
    Color32::from_rgb(82, 125, 255), Color32::from_rgb(185, 105, 255),
];
const SPECTRUM_LABELS: [&str; 8] = ["20–60", "60–150", "150–400", "400–1k", "1–2.5k", "2.5–6k", "6–12k", "12–24k Hz"];

/// Paint a continuous, pixel-scaled frequency envelope.
/// Takes a painter, theme, rectangle, source snapshot, orientation and source-time mapping; returns whether detailed analysis is available.
pub(super) fn paint_spectrum(painter: &egui::Painter, theme: &Theme, rect: Rect, snap: &DeckSnap, vertical: bool, seconds_at: impl Fn(f64) -> Option<f64>) -> bool {
    let Some(spectrum) = &snap.spectrum else { return false; };
    let pixels = painter.ctx().pixels_per_point();
    let length = if vertical { rect.height() } else { rect.width() };
    let rows = (length * pixels).ceil().clamp(1.0, 8192.0) as usize;
    let half_width = (if vertical { rect.width() } else { rect.height() }) * 0.46;
    let center = if vertical { rect.center().x } else { rect.center().y };
    let start = if vertical { rect.top() } else { rect.left() };
    let point = |across, along| if vertical { Pos2::new(across, along) } else { Pos2::new(along, across) };
    let mut mesh = egui::Mesh::default();
    mesh.vertices.reserve((rows + 1) * 72);
    mesh.indices.reserve((rows + 1) * 108);
    let mut previous = [[center; 9]; 2];
    for row in 0..=rows {
        let fraction = row as f64 / rows as f64;
        let next = (row + 1).min(rows) as f64 / rows as f64;
        let mut bin = crate::engine::waveform::Bin::default();
        if let (Some(a), Some(b)) = (seconds_at(fraction), seconds_at(next)) {
            let rate = f64::from(snap.source_sample_rate);
            bin = spectrum.range(a * rate, b * rate + 1.0);
        }
        let total = bin.energy.iter().sum::<f32>();
        let mut widths = [[center; 9]; 2];
        for side in 0..2 {
            let direction = if side == 0 { -1.0 } else { 1.0 };
            let peak = bin.peak[side].clamp(0.0, 1.0) * half_width;
            let mut width = 0.0;
            for band in 0..8 {
                width += if total > 0.0 { bin.energy[band] / total } else { f32::from(band == 0) };
                widths[side][band + 1] = center + direction * width * peak;
                if row == 0 { continue; }
                let color = if total > 0.0 { theme.waveform(SPECTRUM_COLORS[band], 0.95) } else { theme.waveform(theme.fg_dim, 0.7) };
                let base = mesh.vertices.len() as u32;
                for pos in [point(previous[side][band], start + (row - 1) as f32 / rows as f32 * length),
                    point(previous[side][band + 1], start + (row - 1) as f32 / rows as f32 * length),
                    point(widths[side][band + 1], start + row as f32 / rows as f32 * length),
                    point(widths[side][band], start + row as f32 / rows as f32 * length)] {
                    mesh.colored_vertex(pos, color);
                }
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base, base + 2, base + 3);
            }
            if row > 0 && (widths[side][8] != center || previous[side][8] != center) {
                let edge = 1.0 / pixels;
                let color = theme.waveform(theme.fg_dim, 0.65);
                let base = mesh.vertices.len() as u32;
                let from = start + (row - 1) as f32 / rows as f32 * length;
                let to = start + row as f32 / rows as f32 * length;
                mesh.colored_vertex(point(previous[side][8], from), color);
                mesh.colored_vertex(point(previous[side][8] + direction * edge, from), Color32::TRANSPARENT);
                mesh.colored_vertex(point(widths[side][8] + direction * edge, to), Color32::TRANSPARENT);
                mesh.colored_vertex(point(widths[side][8], to), color);
                mesh.add_triangle(base, base + 1, base + 2);
                mesh.add_triangle(base, base + 2, base + 3);
            }
        }
        previous = widths;
    }
    painter.add(egui::Shape::mesh(mesh));
    true
}

#[derive(Default)]
pub(super) struct Panel {
    pub settings: Config,
    saving: Option<Config>,
    message: String,
}

impl Panel {
    /// Restore the project's waveform view.
    /// Takes saved settings; returns with local preference-save feedback cleared.
    pub(super) fn install(&mut self, settings: Config) {
        self.settings = settings;
        self.saving = None;
        self.message.clear();
    }
}

impl App {
    /// Show linked spans and explicit saving.
    /// Takes the deck UI and theme; changes local views and returns an explicit save request.
    pub(super) fn waveform_toolbar(&mut self, ui: &mut Ui, theme: &Theme) -> bool {
        if self.waveform.saving.is_some() && !self.settings.busy() {
            let saved = self.waveform.saving.take().unwrap();
            self.waveform.message = if self.settings.profile().waveforms == saved {
                self.settings.message.clone()
            } else {
                format!("Waveform view was not saved: {}", self.settings.message)
            };
        }
        let can_save = self.waveform.settings != self.settings.profile().waveforms
            && !self.settings.busy()
            && !self.settings.blocked
            && self.settings.draft == self.settings.applied
            && self.settings.worker.is_some()
            && !self.engine.safe_mode()
            && !self.engine.cmd.performance().protected();
        let mut save = false;
        ui.horizontal_wrapped(|ui| {
            let linked = ui.checkbox(&mut self.waveform.settings.linked, "Link");
            accessibility::button(
                ui,
                &linked,
                "Linked waveform zoom",
                Some(self.waveform.settings.linked),
            );
            help::annotate(ui, &linked, HelpControl::WaveformZoom);
            if linked.changed() && self.waveform.settings.linked {
                let zoom = self.waveform.settings.zoom[self.snap.selected_deck.min(DECKS - 1)];
                self.waveform.settings.zoom.fill(zoom);
            }
            for deck in 0..DECKS {
                let zoom = self.waveform.settings.zoom[deck];
                let label = zoom.label(
                    self.snap
                        .decks
                        .get(deck)
                        .is_some_and(|snap| snap.grid.is_some()),
                );
                let response =
                    ui.small_button(format!("{} {}", (b'A' + deck as u8) as char, label));
                accessibility::button(
                    ui,
                    &response,
                    &format!("Deck {}: Waveform zoom", (b'A' + deck as u8) as char),
                    None,
                );
                accessibility::status(ui, &response, label);
                help::annotate(ui, &response, HelpControl::WaveformZoom);
                if response.clicked() {
                    self.waveform.settings.set(deck, zoom.next());
                }
            }
            let response = ui.add_enabled(can_save, egui::Button::new("Save"));
            accessibility::button(ui, &response, "Save waveform view", None);
            accessibility::status(ui, &response, &self.waveform.message);
            help::annotate(ui, &response, HelpControl::WaveformZoom);
            save = response.clicked();
        });
        ui.horizontal_wrapped(|ui| {
            for (color, label) in SPECTRUM_COLORS.into_iter().zip(SPECTRUM_LABELS) {
                ui.label(RichText::new(label).color(theme.waveform(color, 1.0)).size(theme.text_size(8.0)))
                    .on_hover_text("Measured frequency energy; bands overlap. Upper range is limited by the source sample rate.");
            }
        });
        if !self.waveform.message.is_empty() {
            ui.label(RichText::new(&self.waveform.message).size(theme.text_size(9.0)));
        }
        save
    }

    /// Save the captured view through the preference owner.
    /// Takes local linked/zoom settings; queues a revision-qualified save without overwriting another draft.
    pub(super) fn save_waveform_view(&mut self) {
        if self.settings.busy()
            || self.settings.blocked
            || self.settings.draft != self.settings.applied
            || self.settings.worker.is_none()
            || self.engine.safe_mode()
            || self.engine.cmd.performance().protected()
        {
            return;
        }
        let mut next = self.settings.applied.clone();
        next.profiles.get_mut(&next.active).unwrap().waveforms = self.waveform.settings;
        if let Err(error) = next.validate() {
            self.waveform.message = error;
            return;
        }
        self.settings
            .request(crate::preferences::worker::Job::Save {
                preferences: next,
                revision: self.settings.revision.clone(),
            });
        self.waveform.saving = self.settings.busy().then_some(self.waveform.settings);
        self.waveform.message = self.settings.message.clone();
    }
}

#[derive(Clone, Copy)]
pub(super) struct Playhead {
    pub frames: f64,
    pub estimated_output: bool,
}
/// Select a source-qualified output position.
/// Takes a deck snapshot and optional queued-frame position; returns the backend estimate or explicit renderer fallback.
pub(super) fn playhead(snap: &DeckSnap, output: Option<Position>) -> Playhead {
    let output = output.filter(|position| {
        position.media_key != 0
            && position.media_key == snap.media_key
            && position.source_frame.is_finite()
            && position.source_frame >= 0.0
            && position.source_frame <= snap.frames
    });
    Playhead {
        frames: output.map_or(snap.pos, |position| position.source_frame),
        estimated_output: output.is_some(),
    }
}

pub(super) struct Window {
    pub start: f64,
    pub end: f64,
    pub seconds: f64,
    pub duration: f64,
    pub grid: Option<crate::engine::beatgrid::Grid>,
}
impl Window {
    /// Locate a bounded view around a source position.
    /// Takes snapshot, source frames and zoom; returns musical coordinates for saved grids or source seconds without one.
    pub fn new(snap: &DeckSnap, position: f64, zoom: Zoom) -> Option<Self> {
        let duration = if snap.source_sample_rate > 0 {
            snap.frames / f64::from(snap.source_sample_rate)
        } else {
            f64::from(snap.duration)
        };
        if !snap.frames.is_finite()
            || snap.frames <= 0.0
            || !duration.is_finite()
            || duration <= 0.01
            || !position.is_finite()
        {
            return None;
        }
        let seconds = position.clamp(0.0, snap.frames) / snap.frames * duration;
        let coordinate = snap
            .grid
            .as_ref()
            .map_or(Some(seconds), |grid| grid.beat_at(seconds))?;
        let span = if snap.grid.is_some() {
            zoom.beats()
        } else {
            zoom.seconds()
        };
        Some(Self {
            start: coordinate - span * 0.5,
            end: coordinate + span * 0.5,
            seconds,
            duration,
            grid: snap.grid,
        })
    }
    pub fn seconds_at(&self, coordinate: f64) -> Option<f64> {
        self.grid
            .as_ref()
            .map_or(Some(coordinate), |grid| grid.seconds_at(coordinate))
    }
    pub fn coordinate_at(&self, seconds: f64) -> Option<f64> {
        self.grid
            .as_ref()
            .map_or(Some(seconds), |grid| grid.beat_at(seconds))
    }
    pub fn fraction_at(&self, seconds: f64) -> Option<f32> {
        Some(((self.coordinate_at(seconds)? - self.start) / (self.end - self.start)) as f32)
    }
}

/// Describe grid-derived phrase position.
/// Takes the visible source position and optional saved grid; returns eight-bar phrase, bar and beat counts or missing-grid status.
pub(super) fn phrase(snap: &DeckSnap, position: f64) -> String {
    let seconds = if snap.source_sample_rate > 0 {
        position / f64::from(snap.source_sample_rate)
    } else {
        return "No beatgrid".into();
    };
    let Some(beat) = snap
        .grid
        .as_ref()
        .and_then(|grid| grid.beat_at(seconds))
        .filter(|beat| beat.abs() < i64::MAX as f64)
    else {
        return "No beatgrid".into();
    };
    let beat = beat.floor() as i64;
    format!(
        "Phrase {} · bar {}/8 · beat {}/4",
        beat.div_euclid(32) + 1,
        beat.rem_euclid(32).div_euclid(4) + 1,
        beat.rem_euclid(4) + 1
    )
}

/// Compare phase against the selected deck.
/// Takes both snapshots and positions plus the reference deck; returns a signed nearest-beat difference or an unavailable grid.
pub(super) fn phase(decks: &[DeckSnap], positions: &[Playhead; DECKS], reference: usize) -> String {
    let reference = reference.min(DECKS - 1);
    let other = 1 - reference;
    let beat = |index: usize| {
        let deck = decks.get(index)?;
        (deck.source_sample_rate > 0).then_some(())?;
        deck.grid
            .as_ref()?
            .beat_at(positions[index].frames / f64::from(deck.source_sample_rate))
    };
    let Some(offset) = beat(other)
        .zip(beat(reference))
        .map(|(other, reference)| (other - reference + 0.5).rem_euclid(1.0) - 0.5)
    else {
        return "Set both beatgrids to compare phase".into();
    };
    let timing = if positions.iter().all(|position| position.estimated_output) {
        "Output estimate"
    } else {
        "Renderer phase"
    };
    format!(
        "{timing} · {} − {} {offset:+.3} beat",
        (b'A' + other as u8) as char,
        (b'A' + reference as u8) as char
    )
}

/// Paint source peaks and musical markers against one captured playhead.
/// Takes UI, theme, snapshot, geometry, zoom and clock position; invokes source-fraction seeks and returns the waveform response.
pub(super) fn paint(
    ui: &mut Ui,
    theme: &Theme,
    snap: &DeckSnap,
    color: Color32,
    size: Vec2,
    zoom: Zoom,
    position: Playhead,
    mut seek: impl FnMut(f32),
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme.bg_darker);
    let Some(window) = Window::new(snap, position.frames, zoom) else {
        accessibility::numeric(ui, &response, "Waveform position", 0.0, 0.0, 0.0, 0.1, " s");
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No audio",
            FontId::proportional(theme.text_size(10.0)),
            theme.muted,
        );
        return response;
    };
    if let Some(seconds) = accessibility::numeric(
        ui,
        &response,
        "Waveform position",
        window.seconds as f32,
        0.0,
        window.duration as f32,
        0.1,
        " s",
    ) {
        seek((f64::from(seconds) / window.duration) as f32);
    }
    help::annotate(ui, &response, HelpControl::Seek);
    let rows = rect.height().ceil().clamp(1.0, 4096.0) as usize;
    let middle = rect.center().x;
    let half_width = rect.width() * 0.46;
    if !paint_spectrum(&painter, theme, rect, snap, true, |fraction| window.seconds_at(window.start + fraction * (window.end - window.start))) {
    for row in 0..rows {
        let coordinate =
            window.start + (row as f64 + 0.5) / rows as f64 * (window.end - window.start);
        let Some(seconds) = window
            .seconds_at(coordinate)
            .filter(|seconds| (0.0..=window.duration).contains(seconds))
        else {
            continue;
        };
        let index = (seconds / window.duration * snap.peaks.len() as f64) as usize;
        let Some(peak) = snap
            .peaks
            .get(index.min(snap.peaks.len().saturating_sub(1)))
            .filter(|peak| peak.iter().all(|value| value.is_finite()))
        else {
            continue;
        };
        let y = rect.top() + row as f32 / rows as f32 * rect.height();
        painter.line_segment(
            [
                Pos2::new(middle, y),
                Pos2::new(middle - peak[0] * half_width, y),
            ],
            st(1.0, theme.waveform(theme.red, 0.9)),
        );
        painter.line_segment(
            [
                Pos2::new(middle, y),
                Pos2::new(middle + peak[1] * half_width, y),
            ],
            st(1.0, theme.waveform(theme.green, 0.85)),
        );
        painter.line_segment(
            [
                Pos2::new(middle - peak[2] * half_width * 0.35, y),
                Pos2::new(middle + peak[2] * half_width * 0.35, y),
            ],
            st(
                1.0,
                theme.waveform(theme.marker(color, theme.bg_darker), 0.5),
            ),
        );
    }
    }
    if let Some(grid) = &window.grid {
        let first = window.start.ceil() as i64;
        for beat in first..=first + zoom.beats() as i64 {
            let Some(seconds) = grid
                .seconds_at(beat as f64)
                .filter(|seconds| (0.0..=window.duration).contains(seconds))
            else {
                continue;
            };
            let y = rect.top() + window.fraction_at(seconds).unwrap() * rect.height();
            let bar = beat.rem_euclid(4) == 0;
            let stroke = if beat == 0 {
                st(2.0, theme.accent)
            } else {
                st(
                    if bar { 1.2 } else { 0.6 },
                    if bar { color } else { theme.fg_dim },
                )
            };
            painter.hline(rect.x_range(), y, stroke);
            if bar {
                painter.text(
                    Pos2::new(rect.right() - 2.0, y),
                    egui::Align2::RIGHT_BOTTOM,
                    format!("bar {}", beat.div_euclid(4) + 1),
                    FontId::proportional(theme.text_size(8.0)),
                    theme.fg_dim,
                );
            }
            if beat.rem_euclid(32) == 0 {
                painter.text(
                    Pos2::new(rect.left() + 2.0, y),
                    egui::Align2::LEFT_TOP,
                    format!("phrase {}", beat.div_euclid(32) + 1),
                    FontId::proportional(theme.text_size(8.0)),
                    color,
                );
            }
        }
    }
    if snap.loop_start.is_finite()
        && snap.loop_len.is_finite()
        && snap.loop_start >= 0.0
        && snap.loop_len > 1.0
        && snap.loop_start + snap.loop_len <= snap.frames
    {
        let first = window.fraction_at(snap.loop_start / snap.frames * window.duration);
        let last =
            window.fraction_at((snap.loop_start + snap.loop_len) / snap.frames * window.duration);
        if let Some((first, last)) = first
            .zip(last)
            .filter(|(first, last)| *first <= 1.0 && *last >= 0.0)
        {
            let range = Rect::from_min_max(
                Pos2::new(
                    rect.left(),
                    rect.top() + first.clamp(0.0, 1.0) * rect.height(),
                ),
                Pos2::new(
                    rect.right(),
                    rect.top() + last.clamp(0.0, 1.0) * rect.height(),
                ),
            );
            painter.rect_filled(
                range,
                0.0,
                color.gamma_multiply(if snap.loop_on { 0.13 } else { 0.06 }),
            );
            for (fraction, label) in [(first, "loop in"), (last, "loop out")] {
                let y = rect.top() + fraction * rect.height();
                painter.hline(rect.x_range(), y, st(1.2, color));
                painter.text(
                    Pos2::new(rect.right() - 2.0, y),
                    egui::Align2::RIGHT_TOP,
                    label,
                    FontId::proportional(theme.text_size(8.0)),
                    color,
                );
            }
        }
    }
    for (index, frame) in snap.hotcue_positions.iter().enumerate() {
        let Some(fraction) = frame
            .and_then(|frame| window.fraction_at(frame / snap.frames * window.duration))
            .filter(|fraction| (0.0..=1.0).contains(fraction))
        else {
            continue;
        };
        let y = rect.top() + fraction * rect.height();
        painter.hline(
            rect.x_range(),
            y,
            st(1.0, cue_editor::color(theme, snap.cue_styles[index], index)),
        );
        painter.text(
            Pos2::new(rect.left() + 2.0, y),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "{} {}",
                index + 1,
                cue_editor::short_name(snap.cue_styles[index].name.as_str(), 8)
            ),
            FontId::proportional(theme.text_size(9.0)),
            theme.fg,
        );
    }
    let timing = if position.estimated_output {
        "Output position estimate"
    } else {
        "Output timing unavailable · renderer position"
    };
    accessibility::status(
        ui,
        &response,
        &format!(
            "{}; {timing}; {}",
            phrase(snap, position.frames),
            snap.hotcue_positions
                .iter()
                .enumerate()
                .filter(|(_, frame)| frame.is_some())
                .map(|(index, _)| cue_editor::description(snap, index))
                .collect::<Vec<_>>()
                .join("; ")
        ),
    );
    painter.hline(rect.x_range(), rect.center().y, st(1.6, theme.accent));
    if response.clicked() || response.dragged() {
        if let Some(pointer) = response.interact_pointer_pos() {
            let fraction = ((pointer.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
            if let Some(seconds) =
                window.seconds_at(window.start + f64::from(fraction) * (window.end - window.start))
            {
                seek((seconds / window.duration).clamp(0.0, 1.0) as f32);
            }
        }
    }
    response
}

#[cfg(test)]
mod tests;
