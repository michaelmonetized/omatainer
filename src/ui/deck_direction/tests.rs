use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};

struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    time: f64,
    nodes: Vec<(NodeId, Node)>,
    focused: bool,
    text: Vec<String>,
}
impl Gui {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            fixture: Fixture::new(256),
            ctx,
            time: 0.0,
            nodes: vec![],
            focused: true,
            text: vec![],
        };
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1800.0, 1600.0))),
                time: Some(self.time),
                focused: self.focused,
                events,
                ..Default::default()
            },
            |ctx| self.fixture.app.update_frame(ctx),
        );
        self.text = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::epaint::Shape::Text(text) = &shape.shape {
                    Some(text.galley.text().to_string())
                } else {
                    None
                }
            })
            .collect();
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        self.fixture.rt.process(&mut []);
        self.fixture.rt.publish_for_test();
    }
    fn target(&self, label: &str) -> NodeId {
        self.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "Missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, node)| node.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        self.frame(vec![]);
        let target = self.target(label);
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data,
        })]);
        self.frame(vec![]);
    }
    fn point(&self, label: &str) -> Pos2 {
        let bounds = self
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        Pos2::new(
            ((bounds.x0 + bounds.x1) * 0.5) as f32,
            ((bounds.y0 + bounds.y1) * 0.5) as f32,
        )
    }
}

#[test]
fn actual_native_reverse_toggle_confirms_its_latch_waveform_direction_and_exact_deck() {
    let mut gui = Gui::new();
    gui.action("Deck A: Reverse playback", Action::Click, None);
    assert!(
        gui.fixture.app.engine.snapshot().decks[0]
            .controls
            .reverse_latched
    );
    assert!(
        !gui.fixture.app.engine.snapshot().decks[1]
            .controls
            .reverse_latched
    );
    gui.frame(vec![]);
    assert!(gui.text.iter().any(|text| text.contains("← Reverse")));
    assert!(gui.nodes.iter().any(
        |(_, node)| node.label() == Some("Deck A: Playback direction")
            && node.value() == Some("← Reverse")
    ));
    gui.action("Deck A: Reverse playback", Action::Click, None);
    assert!(
        !gui.fixture.app.engine.snapshot().decks[0]
            .controls
            .reverse_latched
    );
}

#[test]
fn native_censor_keyboard_touch_and_assistive_owners_release_independently_and_retire_at_focus_loss(
) {
    let mut gui = Gui::new();
    gui.action("Deck B: Hold Censor", Action::Focus, None);
    gui.frame(vec![egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(gui.fixture.app.engine.snapshot().decks[1].controls.bleep);
    assert!(!gui.fixture.app.engine.snapshot().decks[0].controls.bleep);
    let point = gui.point("Deck B: Hold Censor");
    let touch = |phase, pos| egui::Event::Touch {
        device_id: egui::TouchDeviceId(17),
        id: egui::TouchId(1),
        phase,
        pos,
        force: Some(0.4),
    };
    gui.frame(vec![touch(egui::TouchPhase::Start, point)]);
    gui.frame(vec![egui::Event::Key {
        key: Key::Space,
        physical_key: None,
        pressed: false,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    assert!(gui.fixture.app.engine.snapshot().decks[1].controls.bleep);
    gui.action(
        "Deck B: Hold Censor",
        Action::CustomAction,
        Some(ActionData::CustomAction(0)),
    );
    gui.frame(vec![touch(
        egui::TouchPhase::End,
        point + Vec2::new(1000.0, 1000.0),
    )]);
    assert!(gui.fixture.app.engine.snapshot().decks[1].controls.bleep);
    gui.action(
        "Deck B: Hold Censor",
        Action::CustomAction,
        Some(ActionData::CustomAction(1)),
    );
    assert!(!gui.fixture.app.engine.snapshot().decks[1].controls.bleep);
    gui.action("Deck B: Hold Censor", Action::Click, None);
    assert!(gui.fixture.app.engine.snapshot().decks[1].controls.bleep);
    gui.focused = false;
    gui.frame(vec![]);
    assert!(!gui.fixture.app.engine.snapshot().decks[1].controls.bleep);
}

#[test]
fn real_native_pointer_censor_release_preserves_an_independent_controller_owner() {
    let mut gui = Gui::new();
    let controller = crate::engine::midi::next_source_id();
    let point = gui.point("Deck A: Hold Censor");
    let pointer = |pressed, pos| egui::Event::PointerButton {
        button: PointerButton::Primary,
        pressed,
        pos,
        modifiers: egui::Modifiers::NONE,
    };
    gui.frame(vec![egui::Event::PointerMoved(point), pointer(true, point)]);
    assert!(gui.fixture.app.engine.snapshot().decks[0].controls.bleep);
    gui.fixture.rt.apply(Command::DeckControl {
        source: controller,
        deck: 0,
        control: Control::Hold {
            button: Button::Bleep,
            on: true,
        },
    });
    gui.fixture.rt.publish_for_test();
    gui.frame(vec![
        egui::Event::PointerMoved(point + Vec2::new(300.0, 300.0)),
        pointer(false, point + Vec2::new(300.0, 300.0)),
    ]);
    assert!(gui.fixture.app.engine.snapshot().decks[0].controls.bleep);
    assert!(gui.fixture.app.deck_direction.owners[0]
        .iter()
        .all(Option::is_none));
    gui.fixture.rt.apply(Command::DeckControl {
        source: controller,
        deck: 0,
        control: Control::Hold {
            button: Button::Bleep,
            on: false,
        },
    });
    gui.fixture.rt.publish_for_test();
    assert!(!gui.fixture.app.engine.snapshot().decks[0].controls.bleep);
}
