use super::*;
use crate::ui::test_support::Fixture;
use egui::accesskit::{Action, ActionData, ActionRequest};

struct Gui { fixture: Fixture, ctx: egui::Context, time: f64, focused: bool, nodes: Vec<(egui::accesskit::NodeId, egui::accesskit::Node)> }
impl Gui {
    fn new() -> Self {
        let ctx = egui::Context::default(); ctx.enable_accesskit();
        let mut gui = Self { fixture: Fixture::new(128), ctx, time: 0.0, focused: true, nodes: Vec::new() };
        for deck in 0..2 { gui.fixture.rt.apply(Command::DeckSeek { deck, frac: 0.25 }); }
        gui.fixture.rt.publish_for_test(); gui.frame(vec![]); gui.frame(vec![]); gui
    }
    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.02;
        let output = self.ctx.run(egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0,1200.0))), time: Some(self.time), focused: self.focused, events, ..Default::default() }, |ctx| self.fixture.app.update_frame(ctx));
        self.nodes = output.platform_output.accesskit_update.unwrap().nodes;
        self.fixture.rt.process(&mut []); self.fixture.rt.publish_for_test();
    }
    fn target(&self, deck: u8) -> egui::accesskit::NodeId {
        let label = format!("Deck {}: Hold Cue audition", (b'A'+deck) as char);
        self.nodes.iter().find(|(_,n)| n.label() == Some(label.as_str())).unwrap_or_else(|| panic!("Missing {label}")).0
    }
    fn access(&mut self, deck: u8, action: Action, data: Option<ActionData>) { self.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest { target: self.target(deck), action, data })]); }
    fn held(&self, deck: usize) -> bool { self.fixture.app.engine.snapshot().decks[deck].controls.cue_held }
    fn sound(&mut self) -> f64 { let mut out = [0.0f32;2048]; self.fixture.rt.process(&mut out); self.fixture.rt.publish_for_test(); out.iter().map(|s| f64::from(*s).powi(2)).sum() }
    fn pos(&self, deck: usize) -> f64 { self.fixture.rt.decks[deck].pos }
    fn hit(&self, deck: u8) -> Rect { touch::target_rect(&self.ctx, self.ctx.viewport_id(), touch::Target::Cue(deck)).unwrap() }
}
fn key(key: Key, on: bool, modifiers: egui::Modifiers, repeat: bool) -> egui::Event { egui::Event::Key { key, physical_key: Some(key), pressed: on, modifiers, repeat } }
fn pointer(pos: Pos2, on: bool) -> Vec<egui::Event> { vec![egui::Event::PointerMoved(pos), egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed: on, modifiers: egui::Modifiers::NONE }] }
fn touch_event(device: u64, id: u64, phase: egui::TouchPhase, pos: Pos2) -> egui::Event { egui::Event::Touch { device_id: egui::TouchDeviceId(device), id: egui::TouchId(id), phase, pos, force: None } }

#[test]
fn native_pointer_and_focused_keys_audition_until_the_last_local_owner_releases() {
    let mut gui = Gui::new(); let cue = gui.pos(0); let other = gui.pos(1); let pos = gui.hit(0).center();
    gui.frame(pointer(pos, true)); assert!(gui.held(0)); assert!(gui.sound() > 0.01); assert!(gui.pos(0)>cue);
    gui.frame(vec![key(Key::Space,true,egui::Modifiers::NONE,false)]); assert!(gui.held(0));
    gui.frame(pointer(Pos2::new(1590.0,1190.0),false)); assert!(gui.held(0));
    gui.frame(vec![key(Key::Enter,true,egui::Modifiers::NONE,false)]);
    gui.frame(vec![key(Key::Space,false,egui::Modifiers::CTRL,false)]); assert!(gui.held(0));
    gui.frame(vec![key(Key::Enter,false,egui::Modifiers::SHIFT,false)]); assert!(!gui.held(0)); assert_eq!(gui.pos(0),cue); assert_eq!(gui.pos(1),other);
    gui.sound(); gui.sound(); assert!(gui.sound() < 0.000001);
    gui.frame(vec![key(Key::Space,true,egui::Modifiers::NONE,false)]); assert!(gui.held(0));
    gui.ctx.memory_mut(|m| { if let Some(id)=m.focused() {m.surrender_focus(id);} }); gui.frame(vec![]);
    assert!(!gui.held(0)); assert_eq!(gui.pos(0),cue);
}

#[test]
fn cue_shortcuts_capture_the_deck_ignore_repeat_and_release_through_modifier_dialog_and_focus_changes() {
    let mut gui=Gui::new(); let cue=gui.pos(0); let other=gui.pos(1);
    gui.frame(vec![key(Key::A,true,egui::Modifiers::NONE,false)]); assert!(gui.held(0)); assert!(!gui.held(1));
    gui.sound(); let advanced=gui.pos(0);
    gui.frame(vec![key(Key::A,true,egui::Modifiers::NONE,true)]); assert_eq!(gui.pos(0),advanced);
    gui.fixture.app.dispatch_shortcut(shortcuts::Action::Play(0)); gui.fixture.rt.process(&mut []);
    gui.frame(vec![key(Key::A,false,egui::Modifiers::CTRL,false)]); assert!(!gui.held(0)); assert!(gui.fixture.rt.decks[0].playing); assert_eq!(gui.pos(1),other);
    gui.fixture.app.dispatch_shortcut(shortcuts::Action::Play(0)); gui.fixture.rt.process(&mut []);
    gui.frame(vec![key(Key::A,true,egui::Modifiers::NONE,false)]); assert!(gui.held(0));
    gui.fixture.app.library_playlist.open=true; gui.frame(vec![]); assert!(!gui.held(0));
    gui.frame(vec![key(Key::A,false,egui::Modifiers::ALT,false)]); gui.fixture.app.library_playlist.open=false; gui.frame(vec![]); gui.frame(vec![]);
    gui.ctx.memory_mut(|m| { if let Some(id)=m.focused() {m.surrender_focus(id);} }); gui.frame(vec![]);
    gui.frame(vec![key(Key::L,true,egui::Modifiers::NONE,false)]); assert!(gui.held(1));
    gui.focused=false; gui.frame(vec![]); assert!(!gui.held(1)); gui.focused=true; gui.frame(vec![key(Key::L,true,egui::Modifiers::NONE,true)]); assert!(!gui.held(1));
    gui.frame(vec![key(Key::L,false,egui::Modifiers::NONE,false)]);
    gui.frame(vec![key(Key::A,true,egui::Modifiers::NONE,false),key(Key::A,false,egui::Modifiers::NONE,false)]); assert!(!gui.held(0));
    assert!(gui.pos(0)>=cue);
}

#[test]
fn assistive_and_multitouch_holds_have_explicit_release_and_keep_remote_owners_independent() {
    let mut gui=Gui::new(); let cue=gui.pos(0); let a=gui.hit(0).center(); let b=gui.hit(1).center();
    gui.access(0,Action::CustomAction,Some(ActionData::CustomAction(0))); assert!(gui.held(0)); assert!(gui.sound()>0.01);
    gui.frame(vec![touch_event(1,1,egui::TouchPhase::Start,a),touch_event(2,1,egui::TouchPhase::Start,a),touch_event(1,2,egui::TouchPhase::Start,b)]); assert!(gui.held(0)&&gui.held(1));
    gui.access(0,Action::CustomAction,Some(ActionData::CustomAction(1))); assert!(gui.held(0));
    gui.frame(vec![touch_event(1,1,egui::TouchPhase::End,a),touch_event(1,2,egui::TouchPhase::Cancel,b)]); assert!(gui.held(0)); assert!(!gui.held(1));
    gui.fixture.app.engine.send(Command::DeckControl { source:0,deck:0,control:Control::Hold {button:Button::Cue,on:true} }).unwrap(); gui.fixture.rt.process(&mut []);
    gui.frame(vec![touch_event(2,1,egui::TouchPhase::End,a)]); assert!(gui.held(0));
    gui.fixture.app.engine.send(Command::DeckControl { source:0,deck:0,control:Control::Hold {button:Button::Cue,on:false} }).unwrap(); gui.fixture.rt.process(&mut []); gui.fixture.rt.publish_for_test(); assert!(!gui.held(0)); assert_eq!(gui.pos(0),cue);
    gui.access(0,Action::Click,None); assert!(gui.held(0)); gui.access(0,Action::Click,None); assert!(!gui.held(0));
    gui.fixture.app.dispatch_shortcut(shortcuts::Action::Cue(0)); gui.fixture.rt.process(&mut []); gui.fixture.rt.publish_for_test(); assert!(!gui.held(0)); assert!(!gui.fixture.app.engine.snapshot().decks[0].previewing);
}

#[test]
fn media_changes_safety_recovery_and_panel_retirement_require_a_fresh_cue_press() {
    let mut gui=Gui::new();
    gui.frame(vec![key(Key::A,true,egui::Modifiers::NONE,false)]); assert!(gui.held(0));
    gui.fixture.rt.apply(Command::DeckUnload {deck:0}); gui.fixture.rt.publish_for_test(); gui.frame(vec![]); assert!(!gui.held(0));
    assert!(!gui.fixture.app.cue_audition.held(0));
    gui.access(1,Action::Click,None); assert!(gui.held(1));
    gui.fixture.app.engine.send(Command::SafetyStop(crate::engine::performance::Safety::Stop)).unwrap(); gui.fixture.rt.process(&mut []); gui.fixture.rt.publish_for_test(); gui.frame(vec![]); assert!(!gui.held(1));
    gui.fixture.app.engine.send(Command::RecoverPerformance).unwrap(); gui.fixture.rt.process(&mut []); gui.fixture.rt.publish_for_test(); gui.frame(vec![]); assert!(!gui.held(1));
    gui.access(1,Action::Click,None); assert!(gui.held(1));
    gui.fixture.app.release_workspace_panel(crate::preferences::workspaces::Panel::Decks); gui.fixture.rt.process(&mut []); gui.fixture.rt.publish_for_test(); assert!(!gui.held(1));
}

#[test]
fn cue_alternate_action_menu_arms_a_visible_hold_after_closing_and_can_release_it() {
    let mut gui=Gui::new(); let cue=gui.pos(0);
    gui.access(0,Action::ShowContextMenu,None); assert!(!gui.held(0));
    let target=gui.nodes.iter().find(|(_,n)| n.label().is_some_and(|label|label.ends_with("Press Cue"))).expect("real alternate Press Cue button").0;
    gui.frame(vec![egui::Event::AccessKitActionRequest(ActionRequest {target,action:Action::Click,data:None})]);
    gui.frame(vec![]); assert!(gui.held(0)); assert!(gui.sound()>0.01);
    gui.access(0,Action::CustomAction,Some(ActionData::CustomAction(1))); assert!(!gui.held(0)); assert_eq!(gui.pos(0),cue);
}
