use super::*;
use crate::preferences::Profile;
use egui::accesskit::{Action, ActionRequest};
use test_support::Fixture;

fn theme(scale: f32, font: f32, contrast: crate::theme::Contrast) -> Theme {
    let mut appearance = Profile::defaults(std::path::Path::new("/tmp")).appearance;
    appearance.scale = scale;
    appearance.font_size = Some(font);
    appearance.contrast = contrast;
    let mut t = Theme::default();
    t.font_size = font;
    t.configure_display(&appearance);
    t
}
fn frame(
    f: &mut Fixture,
    ctx: &egui::Context,
    size: Vec2,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size / ctx.zoom_factor())),
            time: Some(time),
            events,
            focused: true,
            ..Default::default()
        },
        |ctx| f.app.update_frame(ctx),
    )
}
#[test]
fn reference_layouts_keep_scaled_controls_readable_and_focus_reachable() {
    for (size, scale, font) in [
        (Vec2::new(1280.0, 720.0), 0.5, 8.0),
        (Vec2::new(1366.0, 768.0), 1.25, 18.0),
        (Vec2::new(3840.0, 2160.0), 2.0, 32.0),
        (Vec2::new(1280.0, 720.0), 3.0, 48.0),
    ] {
        let mut f = Fixture::new(128);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        f.app.theme = theme(scale, font, crate::theme::Contrast::Dark);
        f.app.theme.reduced_motion = true;
        ctx.set_zoom_factor(scale);
        f.app.theme.apply(&ctx);
        frame(&mut f, &ctx, size, 0.0, vec![]);
        let out = frame(&mut f, &ctx, size, 0.1, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().unwrap().nodes;
        let (id, node) = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Deck B: Pitch"))
            .unwrap();
        let bounds = node.bounds().unwrap();
        assert!(
            bounds.width() * f64::from(scale) >= 24.0,
            "pitch target {bounds:?} at {scale}"
        );
        assert!(bounds.height() * f64::from(scale) >= 24.0);
        let event = egui::Event::AccessKitActionRequest(ActionRequest {
            target: *id,
            action: Action::Focus,
            data: None,
        });
        frame(&mut f, &ctx, size, 0.2, vec![event]);
        let out = frame(&mut f, &ctx, size, 0.3, vec![]);
        let nodes = &out.platform_output.accesskit_update.as_ref().unwrap().nodes;
        let (_, node) = nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Deck B: Pitch"))
            .unwrap();
        let b = node.bounds().unwrap();
        assert!(
            b.x0 >= 0.0
                && b.x1 <= f64::from(size.x / scale) + 1.0
                && b.y0 >= 0.0
                && b.y1 <= f64::from(size.y / scale) + 1.0,
            "focused control clipped: {b:?}, {size:?}, scale {scale}"
        );
        assert!(out
            .shapes
            .iter()
            .any(|s| matches!(&s.shape,egui::Shape::Text(t) if t.galley.text()=="Q")));
        let (sq, wave, side, mid) = scratch_metrics(
            size.x / scale,
            200.0,
            28.0,
            4.0,
            f.app
                .theme
                .target_size(22.0)
                .max(f.app.theme.text_size(11.0) * 2.0 + 8.0),
        );
        assert!(sq * scale >= 24.0 && wave >= sq * 7.0 && side > 0.0 && mid >= 96.0);
    }
}
#[test]
fn waveform_level_contrast_and_motion_change_paint_without_signal_geometry() {
    let snap = crate::engine::DeckSnap {
        frames: 144000.0,
        duration: 3.0,
        pos: 36000.0,
        playing: true,
        meter: 0.4,
        peaks: Arc::new(vec![[0.2, 0.4, 0.6]; 1024]),
        ..Default::default()
    };
    let ctx = egui::Context::default();
    let mut t = theme(1.0, 12.0, crate::theme::Contrast::Dark);
    let paint = |t: &Theme| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(600.0, 800.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    vertical_wave(ui, t, &snap, t.accent, 100.0, 200.0, |_| {
                        panic!("contrast cannot seek")
                    });
                    fader(ui, t, 0.5, 8.0, snap.meter, t.accent, 32.0, 200.0, 0);
                    let readout = Readout::from_snapshot(&snap, DeckTimeSettings::default());
                    platter(ui, t, &snap, &readout, t.accent, 160.0, |_, _| {
                        panic!("motion cannot jog")
                    });
                });
            },
        )
    };
    let normal = paint(&t);
    t.waveform_contrast = 3.0;
    t.level_contrast = 3.0;
    t.reduced_motion = true;
    let changed = paint(&t);
    let lines = |out: &egui::FullOutput| {
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::LineSegment { points, stroke }
                    if points[0].y == points[1].y
                        && points[0].y <= 208.0
                        && points[0].x <= 108.0
                        && points[1].x <= 108.0 =>
                {
                    Some((*points, stroke.color))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let a = lines(&normal);
    let b = lines(&changed);
    assert!(a.len() > 100);
    assert_eq!(
        a.iter().map(|v| v.0).collect::<Vec<_>>(),
        b.iter().map(|v| v.0).collect::<Vec<_>>()
    );
    assert!(a.iter().zip(&b).any(|(a, b)| a.1 != b.1));
    let activity = |out: &egui::FullOutput| {
        out.shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Rect(r)
                    if (r.rect.width() - 3.0).abs() < 0.01 && r.rect.height() > 20.0 =>
                {
                    Some((r.rect, r.fill))
                }
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(activity(&normal).0, activity(&changed).0);
    assert_ne!(activity(&normal).1, activity(&changed).1);
    let platter_arm = |out: &egui::FullOutput| {
        out.shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::LineSegment { points, stroke }
                    if stroke.width == 2.0 && points[0].y > 400.0 && points[1].y > 400.0 =>
                {
                    Some(*points)
                }
                _ => None,
            })
            .last()
            .unwrap()
    };
    assert_ne!(platter_arm(&normal), platter_arm(&changed));
}
#[test]
fn appearance_frames_preserve_bit_exact_renderer_output() {
    let mut a = Fixture::new(128);
    let mut b = Fixture::new(128);
    for f in [&mut a, &mut b] {
        f.rt.apply(Command::DeckPlay { deck: 0 });
        f.rt.apply(Command::TogglePlay);
    }
    b.app.theme = theme(2.0, 32.0, crate::theme::Contrast::Light);
    b.app.theme.reduced_motion = true;
    b.app.theme.waveform_contrast = 3.0;
    b.app.theme.level_contrast = 3.0;
    let ac = egui::Context::default();
    let bc = egui::Context::default();
    a.app.theme.apply(&ac);
    b.app.theme.apply(&bc);
    let mut audible = false;
    for index in 0..16 {
        frame(
            &mut a,
            &ac,
            Vec2::new(1366.0, 768.0),
            index as f64 / 60.0,
            vec![],
        );
        frame(
            &mut b,
            &bc,
            Vec2::new(1366.0, 768.0),
            index as f64 / 60.0,
            vec![],
        );
        let mut left = [0.0; 512];
        let mut right = [0.0; 512];
        a.rt.process(&mut left);
        b.rt.process(&mut right);
        audible |= left.iter().any(|v| *v != 0.0);
        assert_eq!(left.map(f32::to_bits), right.map(f32::to_bits));
    }
    assert!(audible, "audio oracle must exercise real sound");
}

#[test]
fn button_targets_and_primary_labels_fit_at_supported_scales() {
    for (scale, font) in [(0.5, 8.0), (1.25, 18.0), (2.0, 32.0), (3.0, 48.0)] {
        let t = theme(scale, font, crate::theme::Contrast::Light);
        let ctx = egui::Context::default();
        t.apply(&ctx);
        let mut rects = Vec::new();
        let size = t.target_size(22.0).max(if font <= 18.0 {
            t.text_size(11.0) * 2.0 + 8.0
        } else {
            0.0
        });
        let out = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 900.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    rects.push(pill(ui, &t, "Play", true, t.accent).rect);
                    rects.push(sq_btn(ui, &t, "Q", true, t.accent, size).rect);
                    rects.push(sq_btn(ui, &t, "1\nA", true, t.accent, size).rect);
                });
            },
        );
        for rect in &rects {
            assert!(rect.width() * scale >= 24.0 && rect.height() * scale >= 24.0);
        }
        for shape in out.shapes {
            if let egui::Shape::Text(text) = shape.shape {
                let bounds = text.visual_bounding_rect();
                assert!(
                    rects.iter().any(|r| r.expand(1.0).contains_rect(bounds)),
                    "label {:?} outside controls {:?}, {scale}x {font}",
                    text.galley.text(),
                    rects
                );
            }
        }
    }
}
