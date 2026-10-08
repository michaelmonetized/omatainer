use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionData, ActionRequest, Node, NodeId};

struct Gui {
    fixture: Fixture,
    ctx: egui::Context,
    time: f64,
    nodes: Vec<(NodeId, Node)>,
    focused: bool,
}
impl Gui {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut gui = Self {
            fixture: Fixture::new(256),
            ctx,
            time: 0.0,
            nodes: Vec::new(),
            focused: true,
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
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        self.fixture.rt.process(&mut []);
        self.fixture.rt.publish_for_test();
    }
    fn action(&mut self, label: &str, action: Action, data: Option<ActionData>) {
        self.frame(vec![]);
        let target = self
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some(label))
            .map(|(id, _)| *id)
            .unwrap_or_else(|| {
                panic!(
                    "missing {label}: {:?}",
                    self.nodes
                        .iter()
                        .filter_map(|(_, n)| n.label())
                        .collect::<Vec<_>>()
                )
            });
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {
            target,
            action,
            data,
        })]);
        self.frame(vec![]);
    }
    fn mode(&mut self, deck: u8, mode: Mode) {
        self.settings(deck);
        self.action(
            &format!("Deck {}: Sync mode {}", (b'A' + deck) as char, mode.label()),
            Action::Click,
            None,
        );
    }
    fn leader(&mut self, deck: u8, leader: Leader) {
        self.settings(deck);
        self.action(
            &format!(
                "Deck {}: Sync leader {}",
                (b'A' + deck) as char,
                leader.label()
            ),
            Action::Click,
            None,
        );
    }
    fn settings(&mut self, deck: u8) {
        self.frame(vec![]);
        let leader = format!("Deck {}: Sync leader Deck A", (b'A' + deck) as char);
        if !self
            .nodes
            .iter()
            .any(|(_, node)| node.label() == Some(leader.as_str()))
        {
            self.action(
                &format!("Deck {}: Sync settings", (b'A' + deck) as char),
                Action::Click,
                None,
            );
        }
    }
}

#[test]
fn native_sync_modes_and_deliberate_leaders_arm_stopped_decks_without_changing_playback_or_other_controls(
) {
    let mut gui = Gui::new();
    let positions = gui.fixture.rt.decks.each_ref().map(|d| d.pos);
    let gain = gui.fixture.rt.decks[1].gain;
    gui.leader(0, Leader::DeckA);
    gui.mode(1, Mode::Tempo);
    assert_eq!(
        gui.fixture.app.engine.snapshot().sync_leader,
        Some(Leader::DeckA)
    );
    assert_eq!(gui.fixture.rt.decks[1].sync_mode(), Mode::Tempo);
    gui.mode(1, Mode::Bar);
    assert_eq!(gui.fixture.rt.decks[1].sync_mode(), Mode::Bar);
    assert!(!gui.fixture.app.engine.snapshot().decks[1].sync_aligned);
    assert!(!gui.fixture.rt.decks[0].playing && !gui.fixture.rt.decks[1].playing);
    assert_eq!(gui.fixture.rt.decks.each_ref().map(|d| d.pos), positions);
    gui.settings(1);
    gui.action("Deck B: Re-arm beat sync", Action::Click, None);
    assert_eq!(gui.fixture.rt.decks[1].sync_mode(), Mode::Beat);
    gui.mode(1, Mode::Off);
    assert_eq!(gui.fixture.rt.decks[1].sync_mode(), Mode::Off);
    assert_eq!(gui.fixture.rt.decks[1].gain, gain);
    gui.leader(1, Leader::Transport);
    gui.mode(0, Mode::Beat);
    assert_eq!(
        gui.fixture.app.engine.snapshot().sync_leader,
        Some(Leader::Transport)
    );
    assert_eq!(gui.fixture.rt.decks[0].sync_mode(), Mode::Beat);
}
