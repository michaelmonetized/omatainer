use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionRequest};

struct Gui { fixture: Fixture, ctx: egui::Context, time: f64 }
impl Gui {
    fn new() -> Self {
        let ctx = egui::Context::default(); ctx.enable_accesskit();
        let mut gui = Self { fixture: Fixture::new(128), ctx, time: 0.0 };
        for deck in 0..2 { gui.fixture.rt.apply(Command::DeckSeek { deck, frac: 0.4 }); }
        gui.fixture.rt.publish_for_test();
        gui.frame(vec![]); gui.frame(vec![]); gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
        self.time += 0.02;
        let output = self.ctx.run(egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1200.0))),
            time: Some(self.time), focused: true, events, ..Default::default()
        }, |ctx| self.fixture.app.update_frame(ctx));
        self.fixture.rt.process(&mut []); self.fixture.rt.publish_for_test(); output
    }
    fn click(&mut self, label: &str) {
        let output = self.frame(vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        let target = nodes.iter().find(|(_, node)| node.label() == Some(label)).map(|(id, _)| *id)
            .unwrap_or_else(|| panic!("Missing {label}: {:?}", nodes.iter().filter_map(|(_, n)| n.label()).collect::<Vec<_>>()));
        self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest { target, action: Action::Click, data: None })]);
        self.frame(vec![]);
    }
}

#[test]
fn native_deck_controls_select_sizes_jump_exact_decks_and_keep_transport_stopped() {
    let mut gui = Gui::new();
    let positions = gui.fixture.rt.decks.each_ref().map(|d| d.pos);
    gui.click("Deck A: Beat jump backward");
    assert!(gui.fixture.rt.decks[0].pos < positions[0]); assert_eq!(gui.fixture.rt.decks[1].pos, positions[1]);
    gui.click("Deck A: Beat jump forward"); assert!((gui.fixture.rt.decks[0].pos - positions[0]).abs() < 1.0e-6);
    gui.click("Deck B: Beat jump size");
    gui.click("8 beats");
    assert_eq!(gui.fixture.app.engine.snapshot().decks[1].controls.beat_jump_size, 6);
    gui.click("Deck B: Beat jump backward"); assert!(gui.fixture.rt.decks[1].pos < positions[1]);
    assert!(gui.fixture.rt.decks.iter().all(|d| !d.playing)); assert!(!gui.fixture.rt.playing);
}

#[test]
fn jump_shortcuts_use_pending_selection_and_text_dialog_and_repeat_guards() {
    let mut gui = Gui::new();
    gui.fixture.app.dispatch_shortcut(shortcuts::Action::BeatJumpScale(true));
    gui.fixture.rt.process(&mut []); gui.fixture.rt.publish_for_test(); assert_eq!(gui.fixture.app.engine.snapshot().decks[0].controls.beat_jump_size, 6);
    let old_b = gui.fixture.rt.decks[1].pos;
    gui.fixture.app.deck_selection = deck_selection::Selection::new(gui.fixture.app.snap.selected_deck_request);
    let output = gui.frame(vec![]);
    let target = output.platform_output.accesskit_update.unwrap().nodes.into_iter().find(|(_, n)| n.label() == Some("Deck B")).unwrap().0;
    let _ = gui.ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1200.0))), focused: true, events: vec![egui::Event::AccessKitActionRequest(ActionRequest { target, action: Action::Click, data: None })], ..Default::default() }, |ctx| gui.fixture.app.update_frame(ctx));
    assert_eq!(gui.fixture.app.load_target(), 1);
    let old_a = gui.fixture.rt.decks[0].pos;
    gui.fixture.app.dispatch_shortcut(shortcuts::Action::BeatJump(false));
    gui.fixture.rt.process(&mut []);
    assert_eq!(gui.fixture.rt.decks[0].pos, old_a); assert!(gui.fixture.rt.decks[1].pos < old_b);
    for binding in shortcuts::BINDINGS.iter().filter(|b| matches!(b.action, shortcuts::Action::BeatJump(_) | shortcuts::Action::BeatJumpScale(_))) {
        assert_eq!(shortcuts::lookup(binding.key, binding.modifiers, true), None);
    }
    let old_b = gui.fixture.rt.decks[1].pos;
    gui.frame(vec![egui::Event::Key { key: Key::CloseBracket, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::SHIFT }]);
    assert_eq!(gui.fixture.rt.decks[0].pos, old_a); assert!(gui.fixture.rt.decks[1].pos > old_b);
    gui.frame(vec![egui::Event::Key { key: Key::CloseBracket, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::SHIFT }]);
    let before = gui.fixture.rt.decks.each_ref().map(|d| d.pos);
    let key = egui::Event::Key { key: Key::CloseBracket, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::SHIFT };
    let edit = egui::Id::new("beat-jump-text-guard");
    let _ = gui.ctx.run(Default::default(), |ctx| { egui::CentralPanel::default().show(ctx, |ui| {
        ui.add(egui::TextEdit::singleline(&mut String::new()).id(edit));
        ui.memory_mut(|m| m.request_focus(edit));
    }); });
    gui.frame(vec![key.clone()]); assert_eq!(gui.fixture.rt.decks.each_ref().map(|d| d.pos), before);
    gui.fixture.app.keys_open = false;
    gui.ctx.memory_mut(|m| m.surrender_focus(edit));
    gui.frame(vec![]);
    gui.fixture.app.library_playlist.open = true;
    gui.frame(vec![key]); assert_eq!(gui.fixture.rt.decks.each_ref().map(|d| d.pos), before);
}
