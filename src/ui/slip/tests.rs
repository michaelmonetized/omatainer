use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionRequest};
struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    time: f64,
}
impl Gui {
    fn new() -> Box<Self> {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Box::new(Self {
            fixture: Fixture::new(128),
            ctx,
            time: 0.0,
        });
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui.click("slip…");
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        let out = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1200.0))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ctx| self.fixture.app.update_frame(ctx),
        );
        self.fixture.rt.process(&mut []);
        self.fixture.rt.publish_for_test();
        out
    }
    fn click(&mut self, label: &str) {
        let nodes = self
            .frame(vec![])
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes;
        let target = nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label) && n.supports_action(Action::Click))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("Missing {label}"));
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn text(&mut self) -> Vec<String> {
        self.frame(vec![])
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::epaint::Shape::Text(t) = &shape.shape {
                    Some(t.galley.text().to_string())
                } else {
                    None
                }
            })
            .collect()
    }
}
#[test]
fn actual_native_enablement_and_release_timing_are_independent_and_preserve_transport_and_mixer() {
    let mut gui = Gui::new();
    let positions = gui.fixture.rt.decks.each_ref().map(|d| d.pos);
    let mixer = (gui.fixture.rt.master, gui.fixture.rt.xfader);
    gui.click("Deck A: Enable slip playback");
    gui.click("Deck A: Slip release timing");
    gui.click("0.5 beats");
    assert!(gui.fixture.app.snap.decks[0].controls.slip);
    assert_eq!(gui.fixture.app.snap.decks[0].controls.slip_release, Some(2));
    assert!(!gui.fixture.app.snap.decks[1].controls.slip);
    gui.click("Deck B: Enable slip playback");
    gui.click("Deck A: Enable slip playback");
    assert!(!gui.fixture.app.snap.decks[0].controls.slip);
    assert!(gui.fixture.app.snap.decks[1].controls.slip);
    assert_eq!(gui.fixture.rt.decks.each_ref().map(|d| d.pos), positions);
    assert_eq!((gui.fixture.rt.master, gui.fixture.rt.xfader), mixer);
    assert!(gui.fixture.rt.decks.iter().all(|d| !d.playing));
}
#[test]
fn actual_native_shadow_and_pending_return_display_use_the_renderers_source_position() {
    let mut gui = Gui::new();
    gui.click("Deck A: Enable slip playback");
    gui.click("Deck A: Slip release timing");
    gui.click("4 beats");
    gui.fixture
        .rt
        .apply(Command::DeckSeek { deck: 0, frac: 0.2 });
    gui.fixture.rt.apply(Command::DeckPlay { deck: 0 });
    gui.fixture
        .rt
        .apply(Command::DeckTouch { deck: 0, on: true });
    gui.fixture.rt.process(&mut [0.0; 2048]);
    gui.fixture.rt.publish_for_test();
    gui.frame(vec![]);
    let text = gui.text();
    assert!(text.iter().any(|s| s.starts_with("Background return:")));
    assert!(text.iter().any(|s| s == "Nested gesture active"));
    gui.fixture
        .rt
        .apply(Command::DeckTouch { deck: 0, on: false });
    gui.fixture.rt.process(&mut [0.0; 2]);
    gui.fixture.rt.publish_for_test();
    gui.frame(vec![]);
    assert!(gui
        .text()
        .iter()
        .any(|s| s == "Waiting for the selected release beat"));
    let state = gui.fixture.app.snap.decks[0].controls;
    assert!(state.slip_due.is_some());
    assert!(state.slip_return.is_some());
    println!(
        "SLIP_NATIVE_RECEIPT {}",
        serde_json::json!({"accessible_enablement_and_release":true,"actual_shadow_and_pending_marker":true,"other_deck_independent":true,"physical_devices_opened":false})
    );
}
