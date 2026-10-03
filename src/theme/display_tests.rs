use super::*;
use crate::preferences::Profile;
use std::path::Path;

// Machado severity 1 matrices, from the Colour developers' published scientific dataset.
// https://github.com/colour-science/colour/blob/develop/colour/blindness/datasets/machado2010.py
const CVD: [[[f64; 3]; 3]; 3] = [
    [
        [0.152286, 1.052583, -0.204868],
        [0.114503, 0.786281, 0.099216],
        [-0.003882, -0.048116, 1.051998],
    ],
    [
        [0.367322, 0.860646, -0.227968],
        [0.280085, 0.672501, 0.047413],
        [-0.011820, 0.042940, 0.968881],
    ],
    [
        [1.255528, -0.076749, -0.178779],
        [-0.078411, 0.930809, 0.147602],
        [0.004733, 0.691367, 0.303900],
    ],
];
fn simulate(color: Color32, matrix: [[f64; 3]; 3]) -> Color32 {
    let linear = [color.r(), color.g(), color.b()].map(|c| {
        let c = f64::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    let channels = matrix.map(|row| {
        let c = row
            .into_iter()
            .zip(linear)
            .map(|(a, b)| a * b)
            .sum::<f64>()
            .clamp(0.0, 1.0);
        let c = if c <= 0.0031308 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        };
        (c * 255.0).round() as u8
    });
    Color32::from_rgb(channels[0], channels[1], channels[2])
}
#[test]
fn contrast_presets_keep_text_and_markers_visible_under_color_vision_simulations() {
    assert_eq!(
        display::contrast_ratio(Color32::BLACK, Color32::WHITE),
        21.0
    );
    for contrast in [Contrast::Dark, Contrast::Light] {
        let mut appearance = Profile::defaults(Path::new("/tmp")).appearance;
        appearance.contrast = contrast;
        let mut theme = Theme::default();
        theme.configure_display(&appearance);
        for bg in [
            theme.bg,
            theme.bg_dark,
            theme.bg_darker,
            theme.bg_light,
            theme.selection,
        ] {
            for fg in [
                theme.fg,
                theme.fg_dim,
                theme.fg_bright,
                theme.muted,
                theme.accent,
                theme.red,
                theme.green,
                theme.yellow,
                theme.magenta,
                theme.cyan,
                theme.orange,
            ] {
                assert!(
                    display::contrast_ratio(fg, bg) >= 4.5,
                    "{contrast:?}: {fg:?} on {bg:?}"
                );
                for matrix in CVD {
                    assert!(
                        display::contrast_ratio(simulate(fg, matrix), simulate(bg, matrix)) >= 4.5,
                        "simulated {contrast:?}: {fg:?} on {bg:?}"
                    );
                }
            }
            let marker = theme.marker(Color32::from_rgb(110, 110, 110), bg);
            assert!(display::contrast_ratio(marker, bg) >= 3.0);
        }
    }
}
#[test]
fn text_and_target_floors_hold_across_supported_scales_and_fonts() {
    for scale in [0.5, 0.75, 1.0, 1.25, 1.5, 2.0, 3.0] {
        for font_size in [8.0, 12.0, 18.0, 32.0, 48.0] {
            let mut t = Theme::default();
            t.scale = scale;
            t.font_size = font_size;
            assert!(t.text_size(9.0) * scale >= 11.0);
            assert!(t.target_size(20.0) * scale >= 24.0);
            let ctx = egui::Context::default();
            t.apply(&ctx);
            assert!(ctx.style().text_styles[&egui::TextStyle::Body].size * scale >= 11.0);
        }
    }
}

#[test]
fn focused_text_edit_uses_a_visible_cursor_on_the_actual_contrast_background() {
    for contrast in [Contrast::Dark, Contrast::Light] {
        let mut appearance = Profile::defaults(Path::new("/tmp")).appearance;
        appearance.contrast = contrast;
        let mut theme = Theme::default();
        theme.configure_display(&appearance);
        let ctx = egui::Context::default();
        theme.apply(&ctx);
        let visuals = ctx.style().visuals.clone();
        let defaults = if contrast == Contrast::Light {
            Visuals::light()
        } else {
            Visuals::dark()
        };
        assert_eq!(visuals.window_shadow, defaults.window_shadow);
        assert_eq!(visuals.popup_shadow, defaults.popup_shadow);
        assert_eq!(
            visuals.text_alpha_from_coverage,
            defaults.text_alpha_from_coverage
        );
        assert!(
            display::contrast_ratio(visuals.text_cursor.stroke.color, visuals.extreme_bg_color)
                >= 3.0
        );
        let mut text = "Focused native text".to_owned();
        let mut output = None;
        for frame in 0..3 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 400.0),
                    )),
                    time: Some(frame as f64 * 0.02),
                    focused: true,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let response = ui.add(egui::TextEdit::singleline(&mut text));
                        response.request_focus();
                    });
                },
            ));
        }
        let output = output.unwrap();
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::LineSegment { stroke, .. } if *stroke == visuals.text_cursor.stroke)), "{contrast:?}: native cursor was not painted");
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Rect(rect) if rect.fill == visuals.extreme_bg_color)));
    }
}
