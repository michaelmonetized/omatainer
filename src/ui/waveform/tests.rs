use super::*;
use crate::engine::{beatgrid::Grid, DeckSnap};
use crate::ui::library_annotations::tests::{Files, Gui};
use std::sync::atomic::AtomicBool;

#[test]
fn spectrum_mesh_resolves_physical_pixels_and_uses_all_measured_frequency_colors() {
    let rate = 48_000;
    let pcm: Vec<f32> = (0..rate * 8).map(|frame| {
        let band = (frame / rate) as usize;
        let hz = [40.0, 100.0, 250.0, 700.0, 1600.0, 4000.0, 8500.0, 16000.0][band];
        (std::f64::consts::TAU * hz * f64::from(frame) / f64::from(rate)).sin() as f32
            * (0.25 + 0.65 * (std::f64::consts::TAU * f64::from(frame) / 7000.0).sin().abs() as f32)
    }).collect();
    let spectrum = Arc::new(crate::engine::waveform::Waveform::analyze(&pcm, 1, rate, || false).unwrap());
    let snap = DeckSnap { frames: pcm.len() as f64, source_sample_rate: rate, spectrum: Some(spectrum.clone()), ..Default::default() };
    for scale in [1.0, 2.0] {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(scale);
        let theme = Theme::default();
        let mut rect = Rect::NOTHING;
        let output = ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 240.0))), ..Default::default() }, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                rect = ui.allocate_exact_size(Vec2::new(960.0, 200.0), Sense::hover()).0;
                ui.painter().rect_filled(rect, 0.0, theme.bg_darker);
                assert!(paint_spectrum(ui.painter(), &theme, rect, &snap, false, |fraction| Some(fraction * 8.0)));
            });
        });
        let mesh = output.shapes.iter().find_map(|shape| if let egui::Shape::Mesh(mesh) = &shape.shape { Some(mesh) } else { None }).unwrap();
        assert!(mesh.vertices.len() >= (rect.width() * scale) as usize * 64);
        for color in SPECTRUM_COLORS {
            assert!(mesh.vertices.iter().any(|vertex| vertex.color == theme.waveform(color, 0.95)));
        }
        assert!(mesh.vertices.iter().all(|vertex| rect.expand(1.0).contains(vertex.pos)));
        assert!(mesh.vertices.iter().any(|vertex| vertex.color == Color32::TRANSPARENT));
        assert_eq!(Arc::strong_count(&spectrum), 2);
        if scale == 1.0 {
            if let Ok(directory) = std::env::var("OMAT_WAVEFORM_ARTIFACTS") {
                let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1000\" height=\"240\" viewBox=\"0 0 1000 240\"><rect width=\"1000\" height=\"240\" fill=\"#14161d\"/>");
                for triangle in mesh.indices.chunks_exact(3) {
                    let vertices = [0, 1, 2].map(|index| mesh.vertices[triangle[index] as usize]);
                    let color = vertices[0].color;
                    if color.a() == 0 { continue; }
                    svg.push_str(&format!("<path d=\"M{},{} L{},{} L{},{} Z\" fill=\"#{:02x}{:02x}{:02x}\"/>", vertices[0].pos.x, vertices[0].pos.y, vertices[1].pos.x, vertices[1].pos.y, vertices[2].pos.x, vertices[2].pos.y, color.r(), color.g(), color.b()));
                }
                svg.push_str("</svg>");
                std::fs::write(std::path::Path::new(&directory).join("native-frequency-waveform.svg"), svg).unwrap();
            }
        }
    }
}

#[test]
fn day_long_source_retains_single_frame_motion_at_ninety_six_khz() {
    let snap = DeckSnap { source_sample_rate: 96000, frames: 86400.0 * 96000.0,
        pos: 72000.0 * 96000.0, grid: Some(Grid::new(0.0, 120.0).unwrap()),
        duration: 86400.0, ..Default::default() };
    let first = Window::new(&snap, snap.pos, Zoom::TwoBars).unwrap();
    let next = Window::new(&snap, snap.pos + 1.0, Zoom::TwoBars).unwrap();
    assert!((next.seconds - first.seconds - 1.0 / 96000.0).abs() < 1e-10);
    assert_eq!(first.end - first.start, 8.0);
}

#[test]
fn long_source_zoom_phase_and_markers_use_exact_source_time_and_saved_grid() {
    let grid = Grid::new(0.125, 120.0)
        .unwrap()
        .with_anchor(8.0, 5.125)
        .unwrap();
    let mut snap = DeckSnap {
        frames: 48000.0 * 129600.0,
        source_sample_rate: 48000,
        pos: 48000.0 * 123456.125,
        grid: Some(grid),
        media_key: 17,
        ..Default::default()
    };
    let position = playhead(
        &snap,
        Some(Position {
            source_frame: snap.pos - 4800.0,
            media_key: 17,
            output_frame: 8,
        }),
    );
    assert!(position.estimated_output);
    for zoom in [
        Zoom::TwoBars,
        Zoom::FourBars,
        Zoom::EightBars,
        Zoom::SixteenBars,
    ] {
        let window = Window::new(&snap, position.frames, zoom).unwrap();
        assert_eq!(window.end - window.start, zoom.beats());
        assert!((window.fraction_at(window.seconds).unwrap() - 0.5).abs() < 1e-6);
        for fraction in [0.0, 0.25, 0.75, 1.0] {
            let seconds = window
                .seconds_at(window.start + fraction * zoom.beats())
                .unwrap();
            assert!((f64::from(window.fraction_at(seconds).unwrap()) - fraction).abs() < 1e-6);
        }
    }
    assert!(
        !playhead(
            &snap,
            Some(Position {
                source_frame: 1.0,
                media_key: 18,
                output_frame: 8
            })
        )
        .estimated_output
    );
    assert!(!playhead(&snap, None).estimated_output);
    let other = DeckSnap {
        pos: snap.pos + 6000.0,
        ..snap.clone()
    };
    let positions = [playhead(&snap, None), playhead(&other, None)];
    assert!(phase(&[snap.clone(), other], &positions, 0).contains("+0.200 beat"));
    snap.pos = -12000.0;
    assert!(phrase(&snap, snap.pos).contains("bar 8/8"));
}

#[test]
fn native_link_zoom_save_reopen_and_conflicting_draft_preserve_audio_and_view() {
    let files = Files::new();
    let mut gui = Gui::new(&files);
    gui.app.library_annotations.open = false;
    let prefs = files.0.join("waveform-preferences.json");
    gui.app.settings = preferences::Settings::with_worker_for_test(prefs.clone());
    let source = gui.rt.decks[0].audio.clone();
    gui.frame(vec![]);
    gui.click("Deck A: Waveform zoom");
    assert_eq!(gui.app.waveform.settings.zoom, [Zoom::EightBars; DECKS]);
    gui.click("Linked waveform zoom");
    gui.click("Deck B: Waveform zoom");
    assert_eq!(
        gui.app.waveform.settings.zoom,
        [Zoom::EightBars, Zoom::SixteenBars]
    );
    gui.click("Save waveform view");
    gui.wait(|gui| !gui.app.settings.busy());
    let saved = crate::preferences::storage::load(&prefs, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        saved.preferences.current().unwrap().waveforms,
        gui.app.waveform.settings
    );
    if let Some(source) = source {
        assert!(Arc::ptr_eq(
            &source,
            gui.rt.decks[0].audio.as_ref().unwrap()
        ));
    }
    gui.finish();
    drop(gui);
    let mut reopened = Gui::new(&files);
    reopened.app.library_annotations.open = false;
    reopened.app.initialize_preferences(
        &egui::Context::default(),
        crate::preferences::worker::Startup::read(prefs.clone(), files.0.clone()),
        saved.preferences.current().unwrap().audio.clone(),
    );
    reopened.wait(|gui| !gui.app.settings.busy());
    assert_eq!(
        reopened.app.settings.profile().waveforms,
        saved.preferences.current().unwrap().waveforms
    );
    assert_eq!(
        reopened.app.waveform.settings,
        saved.preferences.current().unwrap().waveforms
    );
    reopened.frame(vec![]);
    reopened.click("Linked waveform zoom");
    assert_eq!(
        reopened.app.waveform.settings.zoom[0],
        reopened.app.waveform.settings.zoom[1]
    );
    reopened
        .app
        .settings
        .draft
        .profiles
        .get_mut("Studio")
        .unwrap()
        .shortcuts_enabled = false;
    reopened.app.save_waveform_view();
    assert!(!reopened.app.settings.busy());
    assert_eq!(
        crate::preferences::storage::load(&prefs, &AtomicBool::new(false))
            .unwrap()
            .preferences,
        saved.preferences
    );
    reopened.finish();
}

#[test]
fn painted_downbeats_saved_cues_and_loop_boundaries_share_the_centered_playhead() {
    let grid = Grid::new(0.125, 120.0)
        .unwrap()
        .with_anchor(8.0, 5.125)
        .unwrap();
    let sr = 48000.0;
    let mut snap = DeckSnap {
        frames: 20.0 * sr,
        source_sample_rate: 48000,
        pos: 5.125 * sr,
        grid: Some(grid),
        peaks: Arc::new(vec![[0.25, 0.5, 0.75]; 8192]),
        loop_start: grid.seconds_at(4.0).unwrap() * sr,
        loop_len: (grid.seconds_at(12.0).unwrap() - grid.seconds_at(4.0).unwrap()) * sr,
        loop_on: true,
        ..Default::default()
    };
    snap.hotcue_positions[0] = Some(grid.seconds_at(6.0).unwrap() * sr);
    let before = Arc::strong_count(&snap.peaks);
    let ctx = egui::Context::default();
    let theme = Theme::default();
    let mut rect = Rect::NOTHING;
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(300.0, 500.0))),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                rect = paint(
                    ui,
                    &theme,
                    &snap,
                    theme.accent,
                    Vec2::new(120.0, 400.0),
                    Zoom::FourBars,
                    playhead(&snap, None),
                    |_| panic!("painting cannot seek"),
                )
                .rect;
            });
        },
    );
    assert_eq!(Arc::strong_count(&snap.peaks), before);
    let horizontal: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::LineSegment { points, .. }
                if points[0].y == points[1].y
                    && points[0].x == rect.left()
                    && points[1].x == rect.right() =>
            {
                Some(points[0].y)
            }
            _ => None,
        })
        .collect();
    for fraction in [0.0, 0.25, 0.375, 0.5, 0.75] {
        assert!(
            horizontal
                .iter()
                .any(|y| (*y - (rect.top() + fraction * rect.height())).abs() < 0.001),
            "missing marker at {fraction}"
        );
    }
    let text: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.contains(&"loop in") && text.contains(&"loop out"));
}
