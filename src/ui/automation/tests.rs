use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionRequest, Node, NodeId};
use serde_json::Value;

struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    nodes: Vec<(NodeId, Node)>,
    time: f64,
}
impl Gui {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            fixture: Fixture::new(256),
            ctx,
            nodes: Vec::new(),
            time: 0.0,
        };
        gui.frame(vec![]);
        gui.frame(vec![]);
        gui.click("Automation");
        gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1200.0))),
                time: Some(self.time),
                events,
                ..Default::default()
            },
            |ctx| self.fixture.app.update_frame(ctx),
        );
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        self.fixture.rt.process(&mut [0.0; 128]);
        self.fixture.rt.publish_for_test();
    }
    fn node(&self, name: &str) -> NodeId {
        self.nodes
            .iter()
            .find(|(_, node)| node.label() == Some(name))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "missing {name}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, node)| node.label())
                        .collect::<Vec<_>>()
                )
            })
    }
    fn click(&mut self, name: &str) {
        let target = self.node(name);
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Click,
            data: None,
        })]);
        self.frame(vec![]);
    }
    fn name(&mut self, value: &str) {
        let target = self.node("API track name");
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action: Action::Focus,
            data: None,
        })]);
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        };
        self.frame(vec![egui::Event::Key {
            key: Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        self.frame(vec![
            egui::Event::Key {
                key: Key::A,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers,
            },
            egui::Event::Text(value.into()),
        ]);
    }
    fn result(&self) -> Value {
        serde_json::from_str(&self.fixture.app.automation_panel.result).unwrap()
    }
}

#[test]
fn native_inspector_reads_state_applies_atomic_track_name_and_reports_completion() {
    let mut gui = Gui::new();
    gui.click("Read API state");
    assert_eq!(gui.result()["ok"], true);
    let slot = gui.fixture.app.snap.selected_track;
    let original = gui.fixture.rt.session.tracks[slot].name.clone();
    gui.name("Native API 日本語");
    gui.click("Apply API track name");
    assert_eq!(gui.result()["ok"], true);
    gui.click("Refresh API job");
    assert_eq!(gui.result()["result"]["status"], "applied");
    assert_eq!(
        gui.fixture.rt.session.tracks[slot].name,
        "Native API 日本語"
    );
    gui.fixture.app.engine.send(Command::Undo).unwrap();
    gui.frame(vec![]);
    assert_eq!(gui.fixture.rt.session.tracks[slot].name, original);
}

#[test]
fn native_inspector_schedules_and_cancels_the_original_track_gain() {
    let mut gui = Gui::new();
    gui.fixture.rt.apply(Command::Play);
    gui.fixture.rt.publish_for_test();
    gui.frame(vec![]);
    let slot = gui.fixture.app.snap.selected_track;
    let original = gui.fixture.rt.tracks[slot].gain;
    gui.click("Next beat");
    gui.click("Schedule selected track gain");
    assert_eq!(gui.result()["ok"], true);
    gui.click("Refresh API job");
    assert_eq!(gui.result()["result"]["status"], "pending");
    gui.click("Cancel API job");
    assert_eq!(gui.result()["result"]["status"], "cancelled");
    gui.fixture.rt.beat = gui.fixture.app.automation_panel.beat + 1.0;
    gui.frame(vec![]);
    assert_eq!(gui.fixture.rt.tracks[slot].gain, original);
    gui.fixture.rt.apply(Command::Stop);
    gui.fixture.rt.publish_for_test();
    gui.frame(vec![]);
    let target = gui.node("Schedule selected track gain");
    assert!(gui
        .nodes
        .iter()
        .find(|(id, _)| *id == target)
        .unwrap()
        .1
        .is_disabled());
}
