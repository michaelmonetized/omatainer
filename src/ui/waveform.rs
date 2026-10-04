use super::*;
use crate::engine::{audible::Position, DeckSnap};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Zoom {
    TwoBars,
    #[default]
    FourBars,
    EightBars,
    SixteenBars,
}
impl Zoom {
    pub fn beats(self) -> f64 {
        match self {
            Self::TwoBars => 8.0,
            Self::FourBars => 16.0,
            Self::EightBars => 32.0,
            Self::SixteenBars => 64.0,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::TwoBars => "2 bars",
            Self::FourBars => "4 bars",
            Self::EightBars => "8 bars",
            Self::SixteenBars => "16 bars",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct Settings {
    pub linked: bool,
    pub zoom: [Zoom; DECKS],
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            linked: true,
            zoom: [Zoom::default(); DECKS],
        }
    }
}
impl Settings {
    /// Set the visible beat span.
    /// Takes a deck and zoom; returns with linked decks sharing the chosen span.
    pub fn set(&mut self, deck: usize, zoom: Zoom) {
        if self.linked {
            self.zoom.fill(zoom);
        } else if let Some(current) = self.zoom.get_mut(deck) {
            *current = zoom;
        }
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
            && position.media_key == snap.audible_key
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
            7.0 * zoom.beats() / 16.0
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
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 2.0),
        egui::Align2::CENTER_BOTTOM,
        if position.estimated_output {
            "output estimate"
        } else {
            "renderer position"
        },
        FontId::proportional(theme.text_size(8.0)),
        theme.muted,
    );
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
mod tests {
    use super::*;
    use crate::engine::beatgrid::Grid;

    #[test]
    fn linked_zoom_and_selected_deck_phase_use_the_saved_local_grids() {
        let mut settings = Settings::default();
        settings.set(1, Zoom::EightBars);
        assert_eq!(settings.zoom, [Zoom::EightBars; DECKS]);
        settings.linked = false;
        settings.set(1, Zoom::TwoBars);
        assert_eq!(settings.zoom, [Zoom::EightBars, Zoom::TwoBars]);
        let decks = [
            DeckSnap {
                source_sample_rate: 44100,
                grid: Some(Grid::new(0.0, 120.0).unwrap()),
                ..Default::default()
            },
            DeckSnap {
                source_sample_rate: 96000,
                grid: Some(
                    Grid::new(0.0, 120.0)
                        .unwrap()
                        .set_anchor(2.0, 60.0)
                        .unwrap(),
                ),
                ..Default::default()
            },
        ];
        let positions = [
            Playhead {
                frames: 0.125 * 44100.0,
                estimated_output: true,
            },
            Playhead {
                frames: 2.5 * 96000.0,
                estimated_output: true,
            },
        ];
        assert!(phase(&decks, &positions, 0).ends_with("B − A +0.250 beat"));
        assert!(phase(&decks, &positions, 1).ends_with("A − B -0.250 beat"));
        assert_eq!(
            phrase(&decks[1], 2.5 * 96000.0),
            "Phrase 1 · bar 2/8 · beat 1/4"
        );
        assert_eq!(
            phase(&[], &positions, 0),
            "Set both beatgrids to compare phase"
        );
    }

    #[test]
    fn actual_paint_aligns_grid_cue_and_loop_lines_to_the_output_position_across_a_tempo_anchor() {
        let mut snap = DeckSnap {
            source_sample_rate: 48000,
            frames: 60.0 * 48000.0,
            pos: 12.0 * 48000.0,
            duration: 60.0,
            grid: Some(
                Grid::new(4.0, 120.0)
                    .unwrap()
                    .set_anchor(9.0, 90.0)
                    .unwrap(),
            ),
            audible_key: 7,
            peaks: Arc::new(vec![[0.25, 0.5, 0.1]; 4096]),
            loop_start: 5.0 * 48000.0,
            loop_len: 6.0 * 48000.0,
            loop_on: true,
            ..Default::default()
        };
        snap.hotcue_positions[0] = Some(9.5 * 48000.0);
        let head = playhead(
            &snap,
            Some(Position {
                source_frame: 8.0 * 48000.0,
                media_key: 7,
                output_frame: 10,
            }),
        );
        assert!(head.estimated_output);
        assert_eq!(head.frames, 8.0 * 48000.0);
        let context = egui::Context::default();
        let theme = Theme::default();
        let mut rect = Rect::NOTHING;
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, 480.0))),
                ..Default::default()
            },
            |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    rect = paint(
                        ui,
                        &theme,
                        &snap,
                        theme.track_color(0),
                        Vec2::new(120.0, 400.0),
                        Zoom::FourBars,
                        head,
                        |_| {},
                    )
                    .rect;
                });
            },
        );
        let has_line = |fraction: f32, width: f32, color: Color32| {
            output.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, stroke } => {
                    points[0].x == rect.left()
                        && points[1].x == rect.right()
                        && (points[0].y - (rect.top() + fraction * rect.height())).abs() < 0.001
                        && stroke.width == width
                        && stroke.color == color
                }
                _ => false,
            })
        };
        assert!(has_line(0.0, 2.0, theme.accent));
        assert!(has_line(0.25, 1.2, theme.track_color(0)));
        assert!(has_line(0.75, 1.2, theme.track_color(0)));
        assert!(has_line(0.125, 1.2, theme.track_color(0)));
        assert!(has_line(0.8125, 1.2, theme.track_color(0)));
        assert!(has_line(
            0.671875,
            1.0,
            cue_editor::color(&theme, snap.cue_styles[0], 0)
        ));
        assert!(has_line(0.5, 1.6, theme.accent));
        assert_eq!(
            playhead(
                &snap,
                Some(Position {
                    source_frame: 8.0 * 48000.0,
                    media_key: 8,
                    output_frame: 10
                })
            )
            .frames,
            snap.pos
        );
    }

    #[test]
    fn long_track_zoom_retains_small_frame_motion_and_legacy_view_defaults() {
        let snap = DeckSnap {
            source_sample_rate: 96000,
            frames: 86400.0 * 96000.0,
            pos: 72000.0 * 96000.0,
            grid: Some(Grid::new(0.0, 120.0).unwrap()),
            duration: 86400.0,
            ..Default::default()
        };
        let first = Window::new(&snap, snap.pos, Zoom::TwoBars).unwrap();
        let next = Window::new(&snap, snap.pos + 1.0, Zoom::TwoBars).unwrap();
        assert!((next.seconds - first.seconds - 1.0 / 96000.0).abs() < 1e-10);
        assert_eq!(first.end - first.start, 8.0);
        let decoded: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(decoded, Settings::default());
        assert!(
            serde_json::from_str::<Settings>("{\"zoom\":[\"infinite\",\"four_bars\"]}").is_err()
        );
        assert!(serde_json::from_str::<Settings>("{\"future\":true}").is_err());
    }
}
