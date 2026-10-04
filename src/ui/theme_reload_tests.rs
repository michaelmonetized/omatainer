use super::{test_support::Fixture, *};
use crate::theme::reload::test_support::{self as sources, Fixture as Sources};
use std::time::Duration;

pub(super) fn frame(
    ctx: &egui::Context,
    fixture: &mut Fixture,
    time: &mut f64,
) -> egui::FullOutput {
    *time += 0.02;
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1440.0, 1000.0))),
            time: Some(*time),
            ..Default::default()
        },
        |ctx| {
            fixture.app.update_frame(ctx);
            egui::Window::new("theme verification").show(ctx, |ui| {
                ui.label("iiiWWW0123");
            });
        },
    );
    fixture.rt.process(&mut [0.0; 128]);
    output
}
fn probe(output: &egui::FullOutput) -> Vec2 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == "iiiWWW0123" => {
                Some(text.galley.size())
            }
            _ => None,
        })
        .unwrap()
}
pub(super) fn installed(ctx: &egui::Context) -> Arc<egui::FontData> {
    ctx.fonts(|fonts| {
        let fonts = fonts.lock();
        let definitions = fonts.fonts.definitions();
        for family in [egui::FontFamily::Monospace, egui::FontFamily::Proportional] {
            assert_eq!(definitions.families[&family][0], "omatainer-selected");
        }
        definitions.font_data["omatainer-selected"].clone()
    })
}

#[test]
fn actual_gui_installs_font_and_text_size_after_font_or_shell_only_reload() {
    let source = Sources::new();
    let colors_modified = std::fs::metadata(&source.theme.path)
        .unwrap()
        .modified()
        .unwrap();
    let mut fixture = Fixture::new(80);
    fixture.app.theme_reload = Some(source.loader());
    let ctx = egui::Context::default();
    let mut time = 0.0;
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Hack"
    });
    let first = probe(&frame(&ctx, &mut fixture, &mut time));
    let original_font = installed(&ctx);
    assert_eq!(ctx.style().text_styles[&egui::TextStyle::Body].size, 12.0);
    source.shell("[font]\nbase-size = 18\n");
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font_size == 18.0
    });
    let sized = probe(&frame(&ctx, &mut fixture, &mut time));
    for style in [
        egui::TextStyle::Body,
        egui::TextStyle::Button,
        egui::TextStyle::Monospace,
    ] {
        assert_eq!(ctx.style().text_styles[&style].size, 18.0);
    }
    assert!(sized.x > first.x * 1.4 && sized.y > first.y * 1.4);
    assert!(
        Arc::ptr_eq(&original_font, &installed(&ctx)),
        "size-only reload need not replace font bytes"
    );
    source.select("Ubuntu", "ubuntu.ttf");
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Ubuntu"
    });
    let changed = probe(&frame(&ctx, &mut fixture, &mut time));
    let selected_font = installed(&ctx);
    assert_ne!(selected_font.font, original_font.font);
    assert_ne!(
        sized.x, changed.x,
        "selected font must change actual text geometry"
    );
    assert_eq!(
        std::fs::metadata(&source.theme.path)
            .unwrap()
            .modified()
            .unwrap(),
        colors_modified
    );
    assert_eq!(fixture.app.theme.path, source.theme.path);
}

#[test]
fn held_theme_resolution_does_not_block_actual_frames_or_engine_controls() {
    let source = Sources::new();
    let (loader, control) = sources::held(&source);
    control
        .started
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    let mut fixture = Fixture::new(80);
    fixture.app.theme_reload = Some(loader);
    let ctx = egui::Context::default();
    let mut time = 0.0;
    let started = Instant::now();
    for i in 0..24 {
        fixture
            .app
            .engine
            .cmd
            .send(Command::Master(i as f32 / 24.0))
            .unwrap();
        let output = frame(&ctx, &mut fixture, &mut time);
        assert!(!output.shapes.is_empty());
        assert_eq!(fixture.rt.master, i as f32 / 24.0);
    }
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(fixture.app.theme.font, "bundled default");
    control.release.send(()).unwrap();
    sources::until(|| {
        frame(&ctx, &mut fixture, &mut time);
        fixture.app.theme.font == "Hack"
    });
    frame(&ctx, &mut fixture, &mut time);
    installed(&ctx);
}
