use super::*;
use std::time::Duration;
struct Gui {
    fixture: test_support::Fixture,
    ctx: egui::Context,
    time: f64,
}
impl Gui {
    fn new() -> Self {
        let mut fixture = test_support::Fixture::new(64);
        fixture.rt.publish_for_test();
        Self {
            fixture,
            ctx: egui::Context::default(),
            time: 0.0,
        }
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1500.0, 1100.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| self.fixture.app.update_frame(ctx),
        )
    }
    fn click(&mut self, label: &str) {
        self.frame(vec![]);
        let output = self.frame(vec![]);
        let pos = test_support::label_center(&output, label);
        for pressed in [true, false] {
            self.frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]);
        }
    }
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.fixture.app.licenses.pending.is_some() {
            self.frame(vec![]);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
fn visible(output: &egui::FullOutput, text: &str) -> bool {
    output.shapes.iter().any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(t) if t.galley.text().contains(text)))
}
#[test]
fn actual_gui_opens_indexes_font_notices_closes_and_reopens_without_project_changes() {
    let mut gui = Gui::new();
    let before = gui.fixture.app.snap.bpm;
    gui.click("Content & licenses");
    gui.settle();
    assert!(gui.fixture.app.licenses.open);
    gui.fixture.app.licenses.query = "font:ubuntu".into();
    gui.click("font:ubuntu — Ubuntu Light");
    let output = gui.frame(vec![]);
    assert!(visible(&output, "Ubuntu-font-1.0"));
    assert!(visible(&output, "Commercial") || visible(&output, "commercial"));
    gui.click("Full notice 1");
    let output = gui.frame(vec![]);
    assert!(visible(&output, "UBUNTU FONT LICENCE Version 1.0"));
    let catalog = gui.fixture.app.licenses.catalog.as_ref().unwrap() as *const Catalog;
    // Standard native window close; closed panels never alter any project state.
    gui.click("Close license viewer");
    assert!(!gui.fixture.app.licenses.open);
    gui.click("Content & licenses");
    assert_eq!(
        gui.fixture.app.licenses.catalog.as_ref().unwrap() as *const Catalog,
        catalog
    );
    assert_eq!(gui.fixture.app.snap.bpm, before);
    assert!(gui.fixture.app.licenses.pending.is_none());
}
#[test]
fn close_during_parse_and_failure_are_visible_and_do_not_block_controls() {
    let mut gui = Gui::new();
    gui.fixture.app.licenses.open = true;
    gui.fixture.app.licenses.start("invalid", "{}");
    gui.fixture.app.licenses.open = false;
    gui.settle();
    gui.click("Content & licenses");
    let output = gui.frame(vec![]);
    assert!(visible(&output, "License records unavailable"));
    assert!(visible(&output, "No license permission can be inferred"));
    assert!(gui.fixture.app.licenses.catalog.is_none());
    gui.click("Close license viewer");
    assert!(gui.fixture.app.submit(Command::SetBpm(133.0)));
    gui.fixture.rt.process(&mut []);
    assert_eq!(gui.fixture.rt.bpm, 133.0);
}

#[test]
fn large_runtime_notice_renders_visible_lines_while_commands_keep_progressing() {
    let mut gui = Gui::new();
    gui.click("Content & licenses");
    gui.settle();
    gui.fixture.app.licenses.query = "toolchain:".into();
    let label = {
        let catalog = gui.fixture.app.licenses.catalog.as_ref().unwrap();
        let entry = catalog
            .manifest
            .entries
            .iter()
            .find(|e| e.category == "toolchain-runtime")
            .unwrap();
        let id = entry.notices[0].sha256.as_ref().unwrap();
        let text = &catalog.notices[id];
        assert!(text.len() > 1_000_000);
        assert_eq!(
            catalog.notice_lines[id]
                .iter()
                .map(|range| &text[range.clone()])
                .collect::<String>(),
            *text
        );
        format!("{} — {}", entry.id, entry.name)
    };
    gui.click(&label);
    gui.click("Full notice 1");
    for step in 0..16 {
        assert!(gui.fixture.app.submit(Command::SetBpm(120.0 + step as f32)));
        gui.fixture.rt.process(&mut [0.0; 128]);
        let output = gui.frame(vec![]);
        let painted_text: usize = output
            .shapes
            .iter()
            .map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(t) => t.galley.text().len(),
                _ => 0,
            })
            .sum();
        assert!(
            painted_text < 64_000,
            "entire long notice was laid out: {painted_text}"
        );
        assert_eq!(gui.fixture.rt.bpm, 120.0 + step as f32);
    }
}
